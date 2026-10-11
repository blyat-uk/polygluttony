//! OpenAI Responses API driver (web-search capable). Mirrors
//! `llm/openai_responses.py:OpenAiResponsesDriver`.

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};

use crate::config::providers::{self, ThinkingParam};
use crate::config::Connection;
use crate::llm::error::LlmError;
use crate::llm::{base_of, client_for, get_json, parse_model_ids, post_json, timeout_of, LlmDriver, LlmRequest, LlmResponse, Usage};
use crate::llm::sse;

pub struct OpenAiResponsesDriver {
    conn: Connection,
    client: reqwest::Client,
}

impl OpenAiResponsesDriver {
    pub fn new(conn: Connection) -> Self {
        let client = client_for(&conn);
        Self { conn, client }
    }

    fn headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Ok(v) = HeaderValue::from_str(&format!("Bearer {}", self.conn.api_key)) {
            h.insert(AUTHORIZATION, v);
        }
        h
    }

    /// Build the base request body shared by `complete` and `stream`.
    fn build_body(&self, system: &str, user: &str) -> Value {
        let c = &self.conn;
        let max_tokens = c.max_tokens.unwrap_or(8192);
        let mut body = json!({
            "model": c.model,
            "max_output_tokens": max_tokens,
            "input": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
        });
        // Runtime flag: only the resolved web-search connection carries it.
        if c.web_search {
            body["tools"] = json!([{"type": "web_search"}]);
        }
        if let ThinkingParam::OpenAiEffort(effort) =
            providers::thinking_param(c.provider, &c.model, c.thinking, max_tokens)
        {
            body["reasoning"] = json!({"effort": effort});
        }
        body
    }
}

#[async_trait]
impl LlmDriver for OpenAiResponsesDriver {
    async fn complete(&self, system: &str, user: &str) -> Result<String, LlmError> {
        let url = format!("{}/responses", base_of(&self.conn));
        let body = self.build_body(system, user);
        let data = post_json(&self.client, &url, self.headers(), &body, timeout_of(&self.conn)).await?;
        let text: String = data
            .get("output")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|it| it.get("type").and_then(Value::as_str) == Some("message"))
                    .filter_map(|it| it.get("content").and_then(Value::as_array))
                    .flatten()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("output_text"))
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
            &format!("{}/responses", base_of(&self.conn)),
            self.headers(),
            &body,
            |v| match v.get("type").and_then(|t| t.as_str()) {
                Some("response.failed") | Some("response.incomplete") | Some("error") => {
                    let snippet: String = v.to_string().chars().take(500).collect();
                    Err(LlmError::Transport(format!("provider stream error: {snippet}")))
                }
                Some("response.output_text.delta") => {
                    if let Some(t) = v["delta"].as_str() {
                        text.push_str(t);
                    }
                    Ok(())
                }
                Some("response.completed") => {
                    usage.input_tokens = v["response"]["usage"]["input_tokens"].as_u64();
                    usage.output_tokens = v["response"]["usage"]["output_tokens"].as_u64();
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
        let url = format!("{}/models", base_of(&self.conn));
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
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn conn(base: &str) -> Connection {
        Connection {
            base_url: base.into(),
            max_tokens: Some(16),
            timeout: Some(10),
            ..sample(Provider::Openai, "gpt-5.2")
        }
    }

    /// The runtime `web_search` flag must declare the GA `web_search` tool —
    /// not the legacy `web_search_preview`, which is frozen out of newer controls.
    #[tokio::test]
    async fn web_search_sends_the_ga_tool_type() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .and(wiremock::matchers::body_partial_json(serde_json::json!({
                "tools": [{"type": "web_search"}]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "output": [{"type":"message","content":[{"type":"output_text","text":"hi"}]}]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let mut c = conn(&server.uri());
        c.web_search = true;
        let d = OpenAiResponsesDriver::new(c);
        assert_eq!(d.complete("s", "u").await.unwrap(), "hi");
    }

    #[test]
    fn no_tools_without_the_runtime_flag() {
        let body = OpenAiResponsesDriver::new(conn("http://x")).build_body("s", "u");
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn reasoning_models_get_the_thinking_level_as_effort() {
        let body_for = |model: &str, thinking| {
            let c = Connection { thinking, ..sample(Provider::Openai, model) };
            OpenAiResponsesDriver::new(c).build_body("s", "u")
        };
        let body = body_for("gpt-5.5", Some(ThinkingLevel::Low));
        assert_eq!(body["reasoning"], serde_json::json!({"effort": "low"}));
        assert!(body_for("gpt-5.5", None).get("reasoning").is_none(), "model default");
        assert!(body_for("gpt-4.1", Some(ThinkingLevel::High)).get("reasoning").is_none());
    }

    #[tokio::test]
    async fn complete_reads_output_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "output": [
                    {"type":"reasoning","content":[]},
                    {"type":"message","content":[{"type":"output_text","text":"hi"}]}
                ]
            })))
            .mount(&server)
            .await;
        let d = OpenAiResponsesDriver::new(conn(&server.uri()));
        assert_eq!(d.complete("s", "u").await.unwrap(), "hi");
    }

    #[tokio::test]
    async fn stream_accumulates_text_and_usage() {
        let server = MockServer::start().await;
        let sse_body = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":4,\"output_tokens\":3}}}\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_raw(sse_body, "text/event-stream"),
            )
            .mount(&server)
            .await;
        let driver = OpenAiResponsesDriver::new(conn(&server.uri()));
        let resp = driver
            .stream(&LlmRequest { system: "s".into(), user: "u".into() })
            .await
            .unwrap();
        assert_eq!(resp.text, "Hi");
        assert_eq!(resp.usage.input_tokens, Some(4));
        assert_eq!(resp.usage.output_tokens, Some(3));
    }
}
