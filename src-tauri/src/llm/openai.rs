//! OpenAI-compatible chat-completions driver (Ollama, LM Studio, OpenRouter
//! and other custom endpoints). Sends no thinking parameters: their models'
//! controls are unknown. Mirrors `llm/openai.py:OpenAiDriver`.

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};

use crate::config::Connection;
use crate::llm::error::LlmError;
use crate::llm::{base_of, client_for, get_json, parse_model_ids, post_json, timeout_of, LlmDriver, LlmRequest, LlmResponse, Usage};
use crate::llm::sse;

pub struct OpenAiDriver {
    conn: Connection,
    client: reqwest::Client,
}

impl OpenAiDriver {
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
        json!({
            "model": self.conn.model,
            "max_tokens": self.conn.max_tokens.unwrap_or(8192),
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
        })
    }
}

#[async_trait]
impl LlmDriver for OpenAiDriver {
    async fn complete(&self, system: &str, user: &str) -> Result<String, LlmError> {
        let url = format!("{}/chat/completions", base_of(&self.conn));
        let body = self.build_body(system, user);
        let data = post_json(&self.client, &url, self.headers(), &body, timeout_of(&self.conn)).await?;
        let text = data
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if text.trim().is_empty() {
            return Err(LlmError::Empty);
        }
        Ok(text)
    }

    async fn stream(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError> {
        let mut body = self.build_body(&req.system, &req.user);
        body["stream"] = json!(true);
        body["stream_options"] = json!({"include_usage": true});
        let mut text = String::new();
        let mut usage = Usage::default();
        let mut done = false;
        sse::post_sse(
            &self.client,
            &format!("{}/chat/completions", base_of(&self.conn)),
            self.headers(),
            &body,
            |v| {
                // Provider error object embedded in a chunk.
                if v.get("error").is_some() {
                    let snippet: String = v.to_string().chars().take(500).collect();
                    return Err(LlmError::Transport(format!("provider stream error: {snippet}")));
                }
                if let Some(t) = v["choices"][0]["delta"]["content"].as_str() {
                    text.push_str(t);
                }
                // A chunk with a non-null finish_reason signals completion.
                if v["choices"][0]["finish_reason"].is_string() {
                    done = true;
                }
                // The final include_usage chunk has choices:[] and carries totals.
                if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
                    usage.input_tokens = u["prompt_tokens"].as_u64();
                    usage.output_tokens = u["completion_tokens"].as_u64();
                    done = true;
                }
                Ok(())
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
    use crate::config::{sample, Provider};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn conn(base: &str) -> Connection {
        Connection {
            base_url: base.into(),
            max_tokens: Some(16),
            timeout: Some(10),
            ..sample(Provider::Custom, "gpt-x")
        }
    }

    #[tokio::test]
    async fn complete_reads_choice_content() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("authorization", "Bearer k"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "hello"}}]
            })))
            .mount(&server)
            .await;
        let d = OpenAiDriver::new(conn(&server.uri()));
        assert_eq!(d.complete("s", "u").await.unwrap(), "hello");
    }

    #[tokio::test]
    async fn list_models_parses_ids() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id":"gpt-5.2"},{"id":"gpt-4.1"}]
            })))
            .mount(&server)
            .await;
        let models = OpenAiDriver::new(conn(&server.uri())).list_models().await.unwrap();
        assert_eq!(models, vec!["gpt-5.2", "gpt-4.1"]);
    }

    #[tokio::test]
    async fn stream_accumulates_chunks_and_usage() {
        let server = MockServer::start().await;
        // The real include_usage final chunk has choices:[] (empty array) and carries usage.
        let sse_body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":8,\"completion_tokens\":4}}\n\n",
            "data: [DONE]\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_raw(sse_body, "text/event-stream"),
            )
            .mount(&server)
            .await;
        let driver = OpenAiDriver::new(conn(&server.uri()));
        let resp = driver
            .stream(&LlmRequest { system: "s".into(), user: "u".into() })
            .await
            .unwrap();
        assert_eq!(resp.text, "HiHi");
        assert_eq!(resp.usage.input_tokens, Some(8));
        assert_eq!(resp.usage.output_tokens, Some(4));
    }
}
