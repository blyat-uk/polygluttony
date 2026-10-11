//! Anthropic Messages API driver. Mirrors `llm/anthropic.py:AnthropicDriver`.

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde_json::{json, Value};

use crate::config::providers::{self, ThinkingParam};
use crate::config::Connection;
use crate::llm::error::LlmError;
use crate::llm::{base_of, client_for, get_json, parse_model_ids, post_json, timeout_of, LlmDriver, LlmRequest, LlmResponse, Usage};
use crate::llm::sse;

pub struct AnthropicDriver {
    conn: Connection,
    client: reqwest::Client,
}

impl AnthropicDriver {
    pub fn new(conn: Connection) -> Self {
        let client = client_for(&conn);
        Self { conn, client }
    }

    fn headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Ok(v) = HeaderValue::from_str(&self.conn.api_key) {
            h.insert("x-api-key", v);
        }
        h.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        h
    }

    /// Build the base request body shared by `complete` and `stream`.
    fn build_body(&self, system: &str, user: &str) -> Value {
        let max_tokens = self.conn.max_tokens.unwrap_or(8192);
        let mut body = json!({
            "model": self.conn.model,
            "max_tokens": max_tokens,
            "messages": [{"role": "user", "content": user}],
        });
        // The system prompt (instructions + glossary) repeats verbatim across a
        // run's batches, so cache it: later requests read it at a fraction of
        // the input price. The API rejects an empty text block, so skip those.
        if !system.is_empty() {
            body["system"] = json!([
                {"type": "text", "text": system, "cache_control": {"type": "ephemeral"}}
            ]);
        }
        // Claude 4.6+ rejects `budget_tokens`: it thinks adaptively at an
        // effort level. Older models still take a token budget.
        let c = &self.conn;
        match providers::thinking_param(c.provider, &c.model, c.thinking, max_tokens) {
            ThinkingParam::AnthropicAdaptive(effort) => {
                body["thinking"] = json!({"type": "adaptive"});
                body["output_config"] = json!({"effort": effort});
            }
            ThinkingParam::AnthropicBudget(budget) => {
                body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
            }
            _ => {}
        }
        body
    }
}

#[async_trait]
impl LlmDriver for AnthropicDriver {
    async fn complete(&self, system: &str, user: &str) -> Result<String, LlmError> {
        let url = format!("{}/v1/messages", base_of(&self.conn));
        let body = self.build_body(system, user);
        let data = post_json(&self.client, &url, self.headers(), &body, timeout_of(&self.conn)).await?;
        let text: String = data
            .get("content")
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect::<String>()
            })
            .unwrap_or_default();
        if text.trim().is_empty() {
            return Err(LlmError::Empty);
        }
        Ok(text)
    }

    async fn stream(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError> {
        let mut body = self.build_body(&req.system, &req.user);
        body["stream"] = json!(true);
        let mut text = String::new();
        let mut usage = Usage::default();
        let mut done = false;
        sse::post_sse(
            &self.client,
            &format!("{}/v1/messages", base_of(&self.conn)),
            self.headers(),
            &body,
            |v| match v.get("type").and_then(|t| t.as_str()) {
                Some("error") => {
                    let snippet: String = v.to_string().chars().take(500).collect();
                    Err(LlmError::Transport(format!("provider stream error: {snippet}")))
                }
                Some("content_block_delta") => {
                    if v["delta"]["type"] == "text_delta" {
                        if let Some(t) = v["delta"]["text"].as_str() {
                            text.push_str(t);
                        }
                    }
                    Ok(())
                }
                Some("message_start") => {
                    let u = &v["message"]["usage"];
                    usage.input_tokens = u["input_tokens"].as_u64();
                    usage.cache_creation_input_tokens = u["cache_creation_input_tokens"].as_u64();
                    usage.cache_read_input_tokens = u["cache_read_input_tokens"].as_u64();
                    Ok(())
                }
                Some("message_delta") => {
                    usage.output_tokens = v["usage"]["output_tokens"].as_u64();
                    Ok(())
                }
                Some("message_stop") => {
                    done = true;
                    Ok(())
                }
                _ => Ok(()),
            },
        )
        .await?;
        if !done {
            return Err(LlmError::Transport("stream ended before completion".into()));
        }
        if text.trim().is_empty() {
            return Err(LlmError::Empty);
        }
        Ok(LlmResponse { text, usage })
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let url = format!("{}/v1/models", base_of(&self.conn));
        let data = get_json(&self.client, &url, self.headers(), timeout_of(&self.conn)).await?;
        Ok(parse_model_ids(&data))
    }

    fn model(&self) -> &str {
        &self.conn.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{sample, Provider, ThinkingLevel};
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn conn(base: &str) -> Connection {
        Connection {
            base_url: base.into(),
            max_tokens: Some(16),
            timeout: Some(10),
            ..sample(Provider::Anthropic, "claude-x")
        }
    }

    fn body_for(model: &str, thinking: Option<ThinkingLevel>, max_tokens: u32) -> Value {
        let c = Connection {
            thinking,
            max_tokens: Some(max_tokens),
            ..sample(Provider::Anthropic, model)
        };
        AnthropicDriver::new(c).build_body("sys", "u")
    }

    #[tokio::test]
    async fn complete_extracts_text_blocks() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "k"))
            .and(header("anthropic-version", "2023-06-01"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [{"type":"text","text":"OK"},{"type":"thinking","text":"hmm"}]
            })))
            .mount(&server)
            .await;
        let d = AnthropicDriver::new(conn(&server.uri()));
        assert_eq!(d.complete("sys", "ping").await.unwrap(), "OK");
    }

    #[tokio::test]
    async fn http_error_maps_to_llm_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("nope"))
            .mount(&server)
            .await;
        let err = AnthropicDriver::new(conn(&server.uri()))
            .complete("s", "u").await.unwrap_err();
        assert!(err.is_auth());
    }

    #[tokio::test]
    async fn list_models_parses_ids() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id":"claude-opus-4-5"},{"id":"claude-haiku-4-5"}]
            })))
            .mount(&server)
            .await;
        let models = AnthropicDriver::new(conn(&server.uri())).list_models().await.unwrap();
        assert_eq!(models, vec!["claude-opus-4-5", "claude-haiku-4-5"]);
    }

    #[tokio::test]
    async fn stream_accumulates_text_deltas() {
        let server = MockServer::start().await;
        let sse_body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10}}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hmm\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":5}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_raw(sse_body, "text/event-stream"),
            )
            .mount(&server)
            .await;
        let driver = AnthropicDriver::new(conn(&server.uri()));
        let resp = driver
            .stream(&LlmRequest { system: "s".into(), user: "u".into() })
            .await
            .unwrap();
        assert_eq!(resp.text, "Hello");
        assert_eq!(resp.usage.input_tokens, Some(10));
        assert_eq!(resp.usage.output_tokens, Some(5));
    }

    /// A mid-stream provider error event must surface as a retryable Err.
    #[tokio::test]
    async fn stream_error_event_returns_retryable_err() {
        let server = MockServer::start().await;
        let sse_body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":5}}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}}\n\n",
            "event: error\n",
            "data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_raw(sse_body, "text/event-stream"),
            )
            .mount(&server)
            .await;
        let driver = AnthropicDriver::new(conn(&server.uri()));
        let err = driver
            .stream(&LlmRequest { system: "s".into(), user: "u".into() })
            .await
            .unwrap_err();
        assert!(err.is_retryable(), "provider stream error should be retryable, got: {err:?}");
        let msg = err.to_string();
        assert!(msg.contains("provider stream error"), "unexpected message: {msg}");
    }

    /// The system prompt (instructions + glossary) repeats across every batch
    /// of a run, so it carries a cache breakpoint. (wiremock 404s otherwise.)
    #[tokio::test]
    async fn system_prompt_carries_a_cache_breakpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(body_partial_json(serde_json::json!({
                "system": [{"type": "text", "text": "sys", "cache_control": {"type": "ephemeral"}}]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [{"type":"text","text":"OK"}]
            })))
            .mount(&server)
            .await;
        let d = AnthropicDriver::new(conn(&server.uri()));
        assert_eq!(d.complete("sys", "ping").await.unwrap(), "OK");
    }

    #[tokio::test]
    async fn stream_reads_cache_usage() {
        let server = MockServer::start().await;
        let sse_body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10,",
            "\"cache_creation_input_tokens\":300,\"cache_read_input_tokens\":1200}}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":5}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_raw(sse_body, "text/event-stream"),
            )
            .mount(&server)
            .await;
        let resp = AnthropicDriver::new(conn(&server.uri()))
            .stream(&LlmRequest { system: "s".into(), user: "u".into() })
            .await
            .unwrap();
        assert_eq!(resp.usage.cache_creation_input_tokens, Some(300));
        assert_eq!(resp.usage.cache_read_input_tokens, Some(1200));
    }

    /// Claude 4.6+ thinks adaptively at an effort level — `budget_tokens` is
    /// rejected there. (wiremock returns 404 on an unmatched body.)
    #[tokio::test]
    async fn adaptive_claude_sends_adaptive_thinking_with_effort() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(body_partial_json(serde_json::json!({
                "thinking": {"type": "adaptive"},
                "output_config": {"effort": "high"}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [{"type":"text","text":"OK"}]
            })))
            .mount(&server)
            .await;
        let c = Connection {
            model: "claude-sonnet-5-5".into(),
            thinking: Some(ThinkingLevel::High),
            ..conn(&server.uri())
        };
        assert_eq!(AnthropicDriver::new(c).complete("sys", "ping").await.unwrap(), "OK");

        let body = body_for("claude-opus-4-6", Some(ThinkingLevel::Low), 32000);
        assert_eq!(body["thinking"], serde_json::json!({"type": "adaptive"}));
        assert_eq!(body["output_config"], serde_json::json!({"effort": "low"}));
    }

    #[test]
    fn legacy_claude_sends_a_budget_below_max_tokens() {
        let body = body_for("claude-haiku-4-5", Some(ThinkingLevel::Medium), 32000);
        assert_eq!(body["thinking"], serde_json::json!({"type": "enabled", "budget_tokens": 8000}));
        assert!(body.get("output_config").is_none());
        // A budget that can't fit under max_tokens is dropped, not sent.
        let tiny = body_for("claude-haiku-4-5", Some(ThinkingLevel::High), 1024);
        assert!(tiny.get("thinking").is_none());
    }

    #[test]
    fn no_thinking_fields_without_a_level_or_on_custom_endpoints() {
        for body in [
            body_for("claude-sonnet-5-5", None, 32000),
            body_for("claude-haiku-4-5", Some(ThinkingLevel::Off), 32000),
        ] {
            assert!(body.get("thinking").is_none() && body.get("output_config").is_none());
        }
        // An Anthropic-compatible gateway: we can't know what its model takes.
        let custom = Connection {
            thinking: Some(ThinkingLevel::High),
            ..sample(Provider::Custom, "claude-sonnet-5-5")
        };
        let body = AnthropicDriver::new(custom).build_body("s", "u");
        assert!(body.get("thinking").is_none());
    }

    /// A stream that closes without `message_stop` must return Err.
    #[tokio::test]
    async fn stream_without_terminal_event_returns_err() {
        let server = MockServer::start().await;
        // No message_stop — stream just ends after the delta.
        let sse_body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":5}}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"truncated\"}}\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_raw(sse_body, "text/event-stream"),
            )
            .mount(&server)
            .await;
        let driver = AnthropicDriver::new(conn(&server.uri()));
        let err = driver
            .stream(&LlmRequest { system: "s".into(), user: "u".into() })
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("stream ended before completion"),
            "unexpected message: {msg}",
        );
        assert!(err.is_retryable(), "incomplete stream should be retryable");
    }
}
