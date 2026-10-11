//! Anthropic Messages API driver. Mirrors `llm/anthropic.py:AnthropicDriver`.

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde_json::{json, Value};

use crate::config::providers::{self, ThinkingParam};
use crate::config::Connection;
use crate::llm::error::LlmError;
use crate::llm::{base_of, client_for, get_json, parse_model_ids, post_json, timeout_of, LlmDriver, LlmRequest, LlmResponse, Usage};
use crate::llm::sse;

/// Shortest timeout a web-search call gets, whatever the connection's tuning.
const WEB_SEARCH_TIMEOUT_SECS: u32 = 600;
/// How many times one web-search call resumes a `pause_turn` before giving up.
const MAX_PAUSE_CONTINUATIONS: usize = 3;

pub struct AnthropicDriver {
    conn: Connection,
    client: reqwest::Client,
}

impl AnthropicDriver {
    pub fn new(mut conn: Connection) -> Self {
        if conn.web_search {
            // A web-search call is non-streaming: the server searches, reads
            // the results and writes the whole answer before the first byte
            // comes back, which can outlast the idle read timeout tuned for
            // streamed batches (180s by default). This driver serves only that
            // call, so floor `timeout` — it feeds both the client's idle read
            // timeout and the per-request deadline.
            conn.timeout = Some(conn.timeout.unwrap_or_default().max(WEB_SEARCH_TIMEOUT_SECS));
        }
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
        // Server-side search: results come back as content blocks in the same
        // response. The dynamic-filtering tool version needs no separate
        // code-execution tool.
        if c.web_search {
            body["tools"] = json!([{
                "type": providers::anthropic_web_search_tool(&c.model),
                "name": "web_search",
                "max_uses": 5,
            }]);
        }
        body
    }

    /// A web-search call: non-streaming POSTs, resuming each `pause_turn` (the
    /// server's own search loop hit its iteration cap) at most
    /// [`MAX_PAUSE_CONTINUATIONS`] times. Text and usage add up across the turn.
    async fn search_turn(&self, system: &str, user: &str) -> Result<LlmResponse, LlmError> {
        let url = format!("{}/v1/messages", base_of(&self.conn));
        let mut body = self.build_body(system, user);
        let ask = body["messages"][0].clone();
        let mut turn: Vec<Value> = Vec::new();
        let mut text = String::new();
        let mut usage = Usage::default();
        for _ in 0..=MAX_PAUSE_CONTINUATIONS {
            let data =
                post_json(&self.client, &url, self.headers(), &body, timeout_of(&self.conn)).await?;
            add_usage(&mut usage, &data["usage"]);
            text.push_str(&text_of(&data));
            if data["stop_reason"] != "pause_turn" {
                if text.trim().is_empty() {
                    return Err(LlmError::Empty);
                }
                return Ok(LlmResponse { text, usage });
            }
            // Resume: everything the turn produced so far goes back verbatim
            // (thinking blocks included) as the trailing assistant message, and
            // the API picks up from its last `server_tool_use`. No "Continue"
            // user message.
            turn.extend(data["content"].as_array().cloned().unwrap_or_default());
            body["messages"] = json!([ask, {"role": "assistant", "content": turn}]);
        }
        // Not `Transport`: the service would retry it, re-running (and
        // re-billing) every search for a turn that is unlikely to settle.
        Err(LlmError::Declined(format!(
            "web search didn't finish after {MAX_PAUSE_CONTINUATIONS} continuations"
        )))
    }
}

/// Concatenated `text` blocks of a Messages response; thinking and server-tool
/// blocks are skipped.
fn text_of(data: &Value) -> String {
    data.get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default()
}

/// Add one response's `usage` to a running total; a meter stays `None` until
/// some response reports it.
fn add_usage(total: &mut Usage, u: &Value) {
    let add = |sum: &mut Option<u64>, key: &str| {
        if let Some(n) = u[key].as_u64() {
            *sum = Some(sum.unwrap_or(0) + n);
        }
    };
    add(&mut total.input_tokens, "input_tokens");
    add(&mut total.output_tokens, "output_tokens");
    add(&mut total.cache_creation_input_tokens, "cache_creation_input_tokens");
    add(&mut total.cache_read_input_tokens, "cache_read_input_tokens");
}

#[async_trait]
impl LlmDriver for AnthropicDriver {
    async fn complete(&self, system: &str, user: &str) -> Result<String, LlmError> {
        if self.conn.web_search {
            return self.search_turn(system, user).await.map(|r| r.text);
        }
        let url = format!("{}/v1/messages", base_of(&self.conn));
        let body = self.build_body(system, user);
        let data = post_json(&self.client, &url, self.headers(), &body, timeout_of(&self.conn)).await?;
        let text = text_of(&data);
        if text.trim().is_empty() {
            return Err(LlmError::Empty);
        }
        Ok(text)
    }

    async fn stream(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError> {
        // Web search goes non-streaming: a paused turn resumes by sending its
        // content back exactly as returned, which the JSON reply hands over
        // as-is.
        if self.conn.web_search {
            return self.search_turn(&req.system, &req.user).await;
        }
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

    fn search_body_for(model: &str, thinking: Option<ThinkingLevel>) -> Value {
        let c = Connection {
            thinking,
            max_tokens: Some(32000),
            web_search: true,
            ..sample(Provider::Anthropic, model)
        };
        AnthropicDriver::new(c).build_body("sys", "u")
    }

    fn search_conn(base: &str) -> Connection {
        Connection { web_search: true, ..conn(base) }
    }

    fn req() -> LlmRequest {
        LlmRequest { system: "s".into(), user: "u".into() }
    }

    /// JSON bodies of every request the mock server received, in order.
    async fn sent_bodies(server: &MockServer) -> Vec<Value> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| r.body_json::<Value>().unwrap())
            .collect()
    }

    fn json_reply(body: Value) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(body)
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

    #[tokio::test]
    async fn no_tools_without_web_search_on_either_path() {
        let json_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(json_reply(serde_json::json!({"content": [{"type":"text","text":"OK"}]})))
            .mount(&json_server)
            .await;
        AnthropicDriver::new(conn(&json_server.uri())).complete("s", "u").await.unwrap();

        let sse_server = MockServer::start().await;
        let sse_body = concat!(
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"OK\"}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(sse_body, "text/event-stream"))
            .mount(&sse_server)
            .await;
        AnthropicDriver::new(conn(&sse_server.uri())).stream(&req()).await.unwrap();

        let complete = &sent_bodies(&json_server).await[0];
        let streamed = &sent_bodies(&sse_server).await[0];
        assert!(complete.get("tools").is_none(), "complete sent tools: {complete}");
        assert!(streamed.get("tools").is_none(), "stream sent tools: {streamed}");
        // The plain path still streams.
        assert_eq!(streamed["stream"], serde_json::json!(true));
    }

    #[test]
    fn web_search_tool_version_follows_the_model() {
        for (model, kind) in [
            ("claude-sonnet-5-5", "web_search_20260209"),
            ("claude-haiku-4-5", "web_search_20250305"),
        ] {
            assert_eq!(
                search_body_for(model, None)["tools"],
                serde_json::json!([{"type": kind, "name": "web_search", "max_uses": 5}]),
                "{model}"
            );
        }
    }

    /// Search turns go out as plain JSON requests on both entry points and
    /// keep only the text blocks of the reply.
    #[tokio::test]
    async fn web_search_is_non_streaming_and_keeps_only_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(json_reply(serde_json::json!({
                "stop_reason": "end_turn",
                "content": [
                    {"type": "thinking", "thinking": "plan", "signature": "sig"},
                    {"type": "server_tool_use", "id": "srv_1", "name": "web_search",
                     "input": {"query": "fan names"}},
                    {"type": "web_search_tool_result", "tool_use_id": "srv_1",
                     "content": [{"type": "web_search_result", "url": "https://x", "title": "X"}]},
                    {"type": "text", "text": "Hello "},
                    {"type": "text", "text": "world",
                     "citations": [{"type": "web_search_result_location", "url": "https://x"}]}
                ],
                "usage": {"input_tokens": 40, "output_tokens": 9,
                          "cache_creation_input_tokens": 0, "cache_read_input_tokens": 300}
            })))
            .mount(&server)
            .await;
        let d = AnthropicDriver::new(search_conn(&server.uri()));
        let resp = d.stream(&req()).await.unwrap();
        assert_eq!(resp.text, "Hello world");
        assert_eq!(resp.usage.input_tokens, Some(40));
        assert_eq!(resp.usage.output_tokens, Some(9));
        assert_eq!(resp.usage.cache_read_input_tokens, Some(300));
        assert_eq!(d.complete("s", "u").await.unwrap(), "Hello world");

        for body in sent_bodies(&server).await {
            assert!(body.get("stream").is_none(), "web search must not stream: {body}");
            assert_eq!(body["tools"][0]["name"], "web_search");
        }
    }

    /// A paused turn is resumed by re-sending it with the reply's content as
    /// the trailing assistant message — no "Continue" user turn.
    #[tokio::test]
    async fn pause_turn_resends_the_content_and_sums_usage() {
        let server = MockServer::start().await;
        let paused = serde_json::json!([
            {"type": "server_tool_use", "id": "srv_1", "name": "web_search",
             "input": {"query": "fan names"}},
            {"type": "web_search_tool_result", "tool_use_id": "srv_1",
             "content": {"type": "web_search_tool_result_error", "error_code": "max_uses_exceeded"}}
        ]);
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(json_reply(serde_json::json!({
                "stop_reason": "pause_turn",
                "content": paused,
                "usage": {"input_tokens": 10, "output_tokens": 5,
                          "cache_creation_input_tokens": 300, "cache_read_input_tokens": 0}
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(json_reply(serde_json::json!({
                "stop_reason": "end_turn",
                "content": [{"type": "text", "text": "{\"terms\": {}}"}],
                "usage": {"input_tokens": 20, "output_tokens": 7,
                          "cache_creation_input_tokens": 0, "cache_read_input_tokens": 300}
            })))
            .mount(&server)
            .await;

        let resp = AnthropicDriver::new(search_conn(&server.uri())).stream(&req()).await.unwrap();
        assert_eq!(resp.text, "{\"terms\": {}}");
        assert_eq!(resp.usage.input_tokens, Some(30));
        assert_eq!(resp.usage.output_tokens, Some(12));
        assert_eq!(resp.usage.cache_creation_input_tokens, Some(300));
        assert_eq!(resp.usage.cache_read_input_tokens, Some(300));

        let bodies = sent_bodies(&server).await;
        assert_eq!(bodies.len(), 2);
        let messages = bodies[1]["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2, "no extra user turn: {messages:?}");
        assert_eq!(messages[0], serde_json::json!({"role": "user", "content": "u"}));
        assert_eq!(messages[1], serde_json::json!({"role": "assistant", "content": paused}));
        // The resumed request is otherwise the same request.
        assert_eq!(bodies[1]["tools"], bodies[0]["tools"]);
        assert_eq!(bodies[1]["system"], bodies[0]["system"]);
    }

    #[tokio::test]
    async fn web_search_gives_up_after_three_continuations() {
        let server = MockServer::start().await;
        let block = serde_json::json!({"type": "server_tool_use", "id": "srv_1",
                                       "name": "web_search", "input": {"query": "q"}});
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(json_reply(serde_json::json!({
                "stop_reason": "pause_turn",
                "content": [block],
                "usage": {"input_tokens": 1, "output_tokens": 1}
            })))
            .mount(&server)
            .await;
        let err = AnthropicDriver::new(search_conn(&server.uri()))
            .stream(&req())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("didn't finish after 3 continuations"), "{err}");
        // Retrying would re-run every search; the turn is unlikely to settle.
        assert!(!err.is_retryable());

        let bodies = sent_bodies(&server).await;
        assert_eq!(bodies.len(), 4, "first request + 3 continuations");
        // Each continuation carries everything the turn produced so far.
        let last = bodies[3]["messages"].as_array().unwrap();
        assert_eq!(last.len(), 2);
        assert_eq!(last[1]["content"], serde_json::json!([block, block, block]));
    }

    #[test]
    fn adaptive_thinking_and_web_search_share_one_body() {
        let body = search_body_for("claude-sonnet-5-5", Some(ThinkingLevel::Medium));
        assert_eq!(body["thinking"], serde_json::json!({"type": "adaptive"}));
        assert_eq!(body["output_config"], serde_json::json!({"effort": "medium"}));
        assert_eq!(body["tools"][0]["type"], "web_search_20260209");
        // The system prompt keeps its cache breakpoint.
        assert_eq!(body["system"][0]["cache_control"], serde_json::json!({"type": "ephemeral"}));
    }

    /// A non-streaming search can sit silent longer than the tuned idle read
    /// timeout; the web-search driver must not cut it off.
    #[tokio::test]
    async fn web_search_outlasts_the_tuned_read_timeout() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                json_reply(serde_json::json!({"content": [{"type":"text","text":"OK"}]}))
                    .set_delay(std::time::Duration::from_millis(1500)),
            )
            .mount(&server)
            .await;
        let tuned = Connection { timeout: Some(1), ..conn(&server.uri()) };
        // The same delay kills a plain request on this tuning...
        assert!(AnthropicDriver::new(tuned.clone()).complete("s", "u").await.is_err());
        // ...but not a web-search one.
        let d = AnthropicDriver::new(Connection { web_search: true, ..tuned });
        assert_eq!(d.conn.timeout, Some(WEB_SEARCH_TIMEOUT_SECS));
        assert_eq!(d.stream(&req()).await.unwrap().text, "OK");
        // A longer tuned timeout is kept.
        let long = Connection { timeout: Some(900), ..search_conn(&server.uri()) };
        assert_eq!(AnthropicDriver::new(long).conn.timeout, Some(900));
    }
}
