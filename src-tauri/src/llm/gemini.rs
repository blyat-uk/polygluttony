//! Native Gemini API driver (`generateContent` / `streamGenerateContent`).
//! Google connections used to go through Gemini's OpenAI-compatible endpoint,
//! which can't ground answers in Google Search — the glossary's "look up
//! established names online" call needs that, so Google talks natively.

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde_json::{json, Value};

use crate::config::providers::{self, ThinkingParam};
use crate::config::Connection;
use crate::llm::error::LlmError;
use crate::llm::{base_of, client_for, get_json, post_json, timeout_of, LlmDriver, LlmRequest, LlmResponse, Usage};
use crate::llm::sse;

/// `finishReason`s meaning Gemini withheld the answer. The same request gets
/// the same verdict, so these fail without a retry.
const BLOCKED_FINISH_REASONS: &[&str] =
    &["SAFETY", "RECITATION", "PROHIBITED_CONTENT", "BLOCKLIST", "SPII"];

/// Model-list pages to follow. One page of 1000 covers the whole catalog; the
/// cap only guards against a server that keeps handing out page tokens.
const MAX_MODEL_PAGES: usize = 5;

pub struct GeminiDriver {
    conn: Connection,
    client: reqwest::Client,
}

impl GeminiDriver {
    pub fn new(conn: Connection) -> Self {
        let client = client_for(&conn);
        Self { conn, client }
    }

    fn headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Ok(v) = HeaderValue::from_str(&self.conn.api_key) {
            h.insert("x-goog-api-key", v);
        }
        h
    }

    /// `{base}/models/{model}:{method}`. A model already written as a resource
    /// name (`models/gemini-2.5-pro`, the way the API lists them) is used as given.
    fn method_url(&self, method: &str) -> String {
        let model = &self.conn.model;
        let resource = if model.contains('/') { model.clone() } else { format!("models/{model}") };
        format!("{}/{resource}:{method}", base_of(&self.conn))
    }

    /// Build the request body shared by `complete` and `stream`.
    fn build_body(&self, system: &str, user: &str) -> Value {
        let c = &self.conn;
        let max_tokens = c.max_tokens.unwrap_or(8192);
        let mut generation_config = json!({"maxOutputTokens": max_tokens});
        // Gemini 3 thinks at a level; Gemini 2.5 takes a token budget.
        match providers::thinking_param(c.provider, &c.model, c.thinking, max_tokens) {
            ThinkingParam::GeminiLevel(level) => {
                generation_config["thinkingConfig"] = json!({"thinkingLevel": level});
            }
            ThinkingParam::GeminiBudget(budget) => {
                generation_config["thinkingConfig"] = json!({"thinkingBudget": budget});
            }
            _ => {}
        }
        let mut body = json!({
            "contents": [{"role": "user", "parts": [{"text": user}]}],
            "generationConfig": generation_config,
        });
        if !system.trim().is_empty() {
            body["systemInstruction"] = json!({"parts": [{"text": system}]});
        }
        // Runtime flag: only the resolved web-search connection carries it.
        if c.web_search {
            body["tools"] = json!([{"google_search": {}}]);
        }
        body
    }
}

/// Folds `GenerateContentResponse`s — the one-shot reply, or each streamed
/// chunk in turn — into the answer text, the token counts and why it stopped.
#[derive(Debug, Default)]
struct Reply {
    text: String,
    finish_reason: Option<String>,
    block_reason: Option<String>,
    prompt_tokens: Option<u64>,
    /// Google Search results fed back to the model (grounded calls only).
    tool_prompt_tokens: Option<u64>,
    candidates_tokens: Option<u64>,
    thoughts_tokens: Option<u64>,
    cached_tokens: Option<u64>,
}

impl Reply {
    fn absorb(&mut self, v: &Value) -> Result<(), LlmError> {
        if v.get("error").is_some() {
            let snippet: String = v.to_string().chars().take(500).collect();
            return Err(LlmError::Transport(format!("provider stream error: {snippet}")));
        }
        if let Some(reason) = v["promptFeedback"]["blockReason"].as_str() {
            self.block_reason = Some(reason.to_string());
        }
        let candidate = &v["candidates"][0];
        // Parts flagged `thought` are reasoning summaries, not the answer. With
        // Google Search grounding the answer is already in the text parts;
        // `groundingMetadata` only cites the sources, so it's ignored.
        for part in candidate["content"]["parts"].as_array().into_iter().flatten() {
            if part["thought"].as_bool() != Some(true) {
                if let Some(t) = part["text"].as_str() {
                    self.text.push_str(t);
                }
            }
        }
        if let Some(reason) = candidate["finishReason"].as_str() {
            self.finish_reason = Some(reason.to_string());
        }
        // Full counts usually arrive on the last chunk only; each keeps the
        // latest value reported.
        let m = &v["usageMetadata"];
        self.prompt_tokens = m["promptTokenCount"].as_u64().or(self.prompt_tokens);
        self.tool_prompt_tokens = m["toolUsePromptTokenCount"].as_u64().or(self.tool_prompt_tokens);
        self.candidates_tokens = m["candidatesTokenCount"].as_u64().or(self.candidates_tokens);
        self.thoughts_tokens = m["thoughtsTokenCount"].as_u64().or(self.thoughts_tokens);
        self.cached_tokens = m["cachedContentTokenCount"].as_u64().or(self.cached_tokens);
        Ok(())
    }

    fn usage(&self) -> Usage {
        // Thinking tokens are billed as output.
        let output_tokens = match (self.candidates_tokens, self.thoughts_tokens) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
        };
        // Google Search results are fed back to the model as extra input,
        // reported apart from the prompt.
        let input_tokens = match (self.prompt_tokens, self.tool_prompt_tokens) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
        };
        Usage {
            input_tokens,
            output_tokens,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: self.cached_tokens,
        }
    }

    /// The answer, or why there isn't one. A truncated (`MAX_TOKENS`) answer
    /// is still an answer — the pipeline handles the missing lines. A stream
    /// must also have reported a `finishReason`: one that just stops was cut off.
    fn finish(self, streamed: bool) -> Result<LlmResponse, LlmError> {
        let no_text = self.text.trim().is_empty();
        if no_text {
            if let Some(reason) = &self.block_reason {
                return Err(LlmError::Declined(format!("Gemini blocked the prompt ({reason})")));
            }
            if let Some(reason) = self.finish_reason.as_deref() {
                if BLOCKED_FINISH_REASONS.contains(&reason) {
                    return Err(LlmError::Declined(format!("Gemini blocked the response ({reason})")));
                }
            }
        }
        if streamed && self.finish_reason.is_none() {
            return Err(LlmError::Transport("stream ended before completion".into()));
        }
        if no_text {
            return Err(LlmError::Empty);
        }
        let usage = self.usage();
        Ok(LlmResponse { text: self.text, usage })
    }
}

/// Ids of the models that can `generateContent` (embedding and other models
/// are dropped), without the API's `models/` prefix.
fn generate_content_models(v: &Value) -> Vec<String> {
    v["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| {
            m["supportedGenerationMethods"]
                .as_array()
                .is_some_and(|methods| methods.iter().any(|x| x.as_str() == Some("generateContent")))
        })
        .filter_map(|m| m["name"].as_str())
        .map(|name| name.strip_prefix("models/").unwrap_or(name).to_string())
        .collect()
}

#[async_trait]
impl LlmDriver for GeminiDriver {
    async fn complete(&self, system: &str, user: &str) -> Result<String, LlmError> {
        let url = self.method_url("generateContent");
        let body = self.build_body(system, user);
        let data = post_json(&self.client, &url, self.headers(), &body, timeout_of(&self.conn)).await?;
        let mut reply = Reply::default();
        reply.absorb(&data)?;
        Ok(reply.finish(false)?.text)
    }

    async fn stream(&self, req: &LlmRequest) -> Result<LlmResponse, LlmError> {
        let url = format!("{}?alt=sse", self.method_url("streamGenerateContent"));
        let body = self.build_body(&req.system, &req.user);
        let mut reply = Reply::default();
        sse::post_sse(&self.client, &url, self.headers(), &body, |v| reply.absorb(&v)).await?;
        reply.finish(true)
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let first = reqwest::Url::parse(&format!("{}/models", base_of(&self.conn)))
            .map_err(|e| LlmError::Transport(format!("invalid base URL: {e}")))?;
        let mut ids = Vec::new();
        let mut page_token: Option<String> = None;
        for page in 0..MAX_MODEL_PAGES {
            let mut url = first.clone();
            url.query_pairs_mut().append_pair("pageSize", "1000");
            if let Some(token) = &page_token {
                url.query_pairs_mut().append_pair("pageToken", token);
            }
            let data = match get_json(&self.client, url.as_str(), self.headers(), timeout_of(&self.conn)).await {
                Ok(data) => data,
                Err(e) if page == 0 => return Err(e),
                // Later pages are a bonus: keep what the earlier ones listed.
                Err(_) => break,
            };
            ids.extend(generate_content_models(&data));
            page_token = data["nextPageToken"].as_str().filter(|t| !t.is_empty()).map(String::from);
            if page_token.is_none() {
                break;
            }
        }
        Ok(ids)
    }

    fn model(&self) -> &str {
        &self.conn.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{sample, Provider, ThinkingLevel};
    use wiremock::matchers::{body_partial_json, header, method, path, query_param, query_param_is_missing};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const GENERATE: &str = "/v1beta/models/gemini-2.5-pro:generateContent";
    const STREAM: &str = "/v1beta/models/gemini-2.5-pro:streamGenerateContent";

    /// A Google connection on the mock server, keeping the real base's `/v1beta`.
    fn conn(server: &MockServer, model: &str) -> Connection {
        Connection {
            base_url: format!("{}/v1beta", server.uri()),
            max_tokens: Some(16),
            timeout: Some(10),
            ..sample(Provider::Google, model)
        }
    }

    fn body_for(model: &str, thinking: Option<ThinkingLevel>) -> Value {
        let c = Connection { thinking, max_tokens: Some(32000), ..sample(Provider::Google, model) };
        GeminiDriver::new(c).build_body("sys", "u")
    }

    fn req() -> LlmRequest {
        LlmRequest { system: "s".into(), user: "u".into() }
    }

    /// One SSE event per chunk, each a whole `GenerateContentResponse`.
    fn sse_reply(chunks: &[Value]) -> ResponseTemplate {
        let body: String = chunks.iter().map(|c| format!("data: {c}\r\n\r\n")).collect();
        ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
    }

    fn answer(text: &str, finish: &str) -> Value {
        json!({"candidates": [{"content": {"role": "model", "parts": [{"text": text}]}, "finishReason": finish}]})
    }

    #[tokio::test]
    async fn complete_posts_to_generate_content_with_the_key_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(GENERATE))
            .and(header("x-goog-api-key", "k"))
            .and(header("content-type", "application/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{
                    "content": {"role": "model", "parts": [
                        {"text": "Weighing the reply", "thought": true},
                        {"text": "O"},
                        {"text": "K"}
                    ]},
                    "finishReason": "STOP"
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let d = GeminiDriver::new(conn(&server, "gemini-2.5-pro"));
        assert_eq!(d.complete("sys", "ping").await.unwrap(), "OK");
    }

    #[test]
    fn body_carries_the_system_instruction_user_turn_and_output_cap() {
        let body = body_for("gemini-2.5-pro", None);
        assert_eq!(body["systemInstruction"], json!({"parts": [{"text": "sys"}]}));
        assert_eq!(body["contents"], json!([{"role": "user", "parts": [{"text": "u"}]}]));
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 32000);
    }

    #[test]
    fn empty_system_prompt_sends_no_system_instruction() {
        let d = GeminiDriver::new(sample(Provider::Google, "gemini-2.5-pro"));
        assert!(d.build_body("", "u").get("systemInstruction").is_none());
        assert!(d.build_body(" \n", "u").get("systemInstruction").is_none());
    }

    /// Gemini 3 takes a thinking level. (wiremock 404s on an unmatched body.)
    #[tokio::test]
    async fn gemini_3_sends_its_thinking_level() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1beta/models/gemini-3.1-pro-preview:generateContent"))
            .and(body_partial_json(json!({
                "generationConfig": {"thinkingConfig": {"thinkingLevel": "high"}}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(answer("OK", "STOP")))
            .expect(1)
            .mount(&server)
            .await;
        let c = Connection {
            thinking: Some(ThinkingLevel::High),
            ..conn(&server, "gemini-3.1-pro-preview")
        };
        assert_eq!(GeminiDriver::new(c).complete("sys", "ping").await.unwrap(), "OK");
    }

    #[test]
    fn gemini_25_sends_a_thinking_budget() {
        let body = body_for("gemini-2.5-pro", Some(ThinkingLevel::Medium));
        assert_eq!(body["generationConfig"]["thinkingConfig"], json!({"thinkingBudget": 8192}));
    }

    #[test]
    fn no_thinking_config_without_a_supported_level() {
        for body in [
            body_for("gemini-2.5-pro", None),
            // Medium isn't a Gemini 3 level; older models have no thinking control.
            body_for("gemini-3.1-pro-preview", Some(ThinkingLevel::Medium)),
            body_for("gemini-2.0-flash", Some(ThinkingLevel::High)),
        ] {
            assert!(body["generationConfig"].get("thinkingConfig").is_none(), "{body}");
        }
    }

    /// The glossary's web-search call streams like every other request, with
    /// Google Search attached; the grounded answer is plain text parts.
    #[tokio::test]
    async fn web_search_attaches_the_google_search_tool() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(STREAM))
            .and(query_param("alt", "sse"))
            .and(body_partial_json(json!({"tools": [{"google_search": {}}]})))
            .respond_with(sse_reply(&[
                json!({"candidates": [{"content": {"parts": [{"text": "Fans call him "}]}}]}),
                json!({"candidates": [{
                    "content": {"parts": [{"text": "Rimuru."}]},
                    "finishReason": "STOP",
                    "groundingMetadata": {
                        "webSearchQueries": ["Tensura Rimuru name"],
                        "groundingChunks": [{"web": {"uri": "https://example.com", "title": "Wiki"}}]
                    }
                }]}),
            ]))
            .expect(1)
            .mount(&server)
            .await;
        let mut c = conn(&server, "gemini-2.5-pro");
        c.web_search = true;
        let resp = GeminiDriver::new(c).stream(&req()).await.unwrap();
        assert_eq!(resp.text, "Fans call him Rimuru.");
    }

    #[test]
    fn no_tools_without_the_runtime_flag() {
        assert!(body_for("gemini-2.5-pro", None).get("tools").is_none());
    }

    #[tokio::test]
    async fn stream_accumulates_chunks_skips_thoughts_and_takes_the_final_usage() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(STREAM))
            .and(query_param("alt", "sse"))
            .and(header("x-goog-api-key", "k"))
            .respond_with(sse_reply(&[
                json!({
                    "candidates": [{"content": {"role": "model", "parts": [
                        {"text": "Planning the lines", "thought": true},
                        {"text": "Hel"}
                    ]}}],
                    "usageMetadata": {"promptTokenCount": 12, "candidatesTokenCount": 1}
                }),
                json!({"candidates": [{"content": {"role": "model", "parts": [{"text": "lo"}]}}]}),
                json!({
                    "candidates": [{
                        "content": {"role": "model", "parts": [{"text": " world"}]},
                        "finishReason": "STOP"
                    }],
                    "usageMetadata": {
                        "promptTokenCount": 12, "toolUsePromptTokenCount": 3, "candidatesTokenCount": 5, "thoughtsTokenCount": 7,
                        "cachedContentTokenCount": 4, "totalTokenCount": 24
                    }
                }),
            ]))
            .mount(&server)
            .await;
        let resp = GeminiDriver::new(conn(&server, "gemini-2.5-pro")).stream(&req()).await.unwrap();
        assert_eq!(resp.text, "Hello world");
        assert_eq!(resp.usage.input_tokens, Some(15)); // prompt + search results
        assert_eq!(resp.usage.output_tokens, Some(12), "answer + thinking tokens");
        assert_eq!(resp.usage.cache_read_input_tokens, Some(4));
        assert_eq!(resp.usage.cache_creation_input_tokens, None);
    }

    /// A mid-stream error object must surface as a retryable Err.
    #[tokio::test]
    async fn stream_error_object_returns_retryable_err() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(STREAM))
            .respond_with(sse_reply(&[
                json!({"candidates": [{"content": {"parts": [{"text": "partial"}]}}]}),
                json!({"error": {"code": 503, "message": "The model is overloaded.", "status": "UNAVAILABLE"}}),
            ]))
            .mount(&server)
            .await;
        let err = GeminiDriver::new(conn(&server, "gemini-2.5-pro")).stream(&req()).await.unwrap_err();
        assert!(err.is_retryable(), "provider stream error should be retryable, got: {err:?}");
        let msg = err.to_string();
        assert!(msg.contains("provider stream error") && msg.contains("overloaded"), "{msg}");
    }

    /// A stream that closes without a `finishReason` was cut off.
    #[tokio::test]
    async fn stream_without_a_finish_reason_returns_retryable_err() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(STREAM))
            .respond_with(sse_reply(&[
                json!({"candidates": [{"content": {"parts": [{"text": "truncated"}]}}]}),
            ]))
            .mount(&server)
            .await;
        let err = GeminiDriver::new(conn(&server, "gemini-2.5-pro")).stream(&req()).await.unwrap_err();
        assert!(err.to_string().contains("stream ended before completion"), "{err}");
        assert!(err.is_retryable(), "incomplete stream should be retryable");
    }

    #[tokio::test]
    async fn safety_blocked_response_is_a_non_retryable_error() {
        let server = MockServer::start().await;
        let blocked = json!({"candidates": [{
            "finishReason": "SAFETY",
            "safetyRatings": [{"category": "HARM_CATEGORY_HARASSMENT", "probability": "HIGH", "blocked": true}]
        }]});
        Mock::given(method("POST"))
            .and(path(GENERATE))
            .respond_with(ResponseTemplate::new(200).set_body_json(blocked.clone()))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(STREAM))
            .respond_with(sse_reply(&[blocked]))
            .mount(&server)
            .await;
        let d = GeminiDriver::new(conn(&server, "gemini-2.5-pro"));
        let one_shot = d.complete("s", "u").await.unwrap_err();
        let streamed = d.stream(&req()).await.unwrap_err();
        for err in [one_shot, streamed] {
            assert!(!err.is_retryable(), "{err:?}");
            assert!(err.to_string().contains("Gemini blocked the response (SAFETY)"), "{err}");
        }
    }

    /// A blocked prompt gets no candidates at all — and so no `finishReason`,
    /// which must not be mistaken for a cut-off stream.
    #[tokio::test]
    async fn blocked_prompt_is_a_non_retryable_error() {
        let server = MockServer::start().await;
        let blocked = json!({
            "promptFeedback": {"blockReason": "PROHIBITED_CONTENT"},
            "usageMetadata": {"promptTokenCount": 9}
        });
        Mock::given(method("POST"))
            .and(path(GENERATE))
            .respond_with(ResponseTemplate::new(200).set_body_json(blocked.clone()))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(STREAM))
            .respond_with(sse_reply(&[blocked]))
            .mount(&server)
            .await;
        let d = GeminiDriver::new(conn(&server, "gemini-2.5-pro"));
        let one_shot = d.complete("s", "u").await.unwrap_err();
        let streamed = d.stream(&req()).await.unwrap_err();
        for err in [one_shot, streamed] {
            assert!(!err.is_retryable(), "{err:?}");
            assert!(err.to_string().contains("Gemini blocked the prompt (PROHIBITED_CONTENT)"), "{err}");
        }
    }

    /// No answer text is `Empty` — including a reply whose whole budget went
    /// on thinking, which the connection test's tiny probe counts as success.
    #[tokio::test]
    async fn no_answer_text_is_empty() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(GENERATE))
            .respond_with(ResponseTemplate::new(200).set_body_json(answer("", "STOP")))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(STREAM))
            .respond_with(sse_reply(&[json!({
                "candidates": [{
                    "content": {"parts": [{"text": "Still thinking", "thought": true}]},
                    "finishReason": "MAX_TOKENS"
                }],
                "usageMetadata": {"promptTokenCount": 5, "thoughtsTokenCount": 16}
            })]))
            .mount(&server)
            .await;
        let d = GeminiDriver::new(conn(&server, "gemini-2.5-pro"));
        assert!(matches!(d.complete("s", "u").await, Err(LlmError::Empty)));
        assert!(matches!(d.stream(&req()).await, Err(LlmError::Empty)));
    }

    /// A truncated answer is still an answer; the pipeline deals with the gap.
    #[tokio::test]
    async fn max_tokens_with_text_returns_the_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(GENERATE))
            .respond_with(ResponseTemplate::new(200).set_body_json(answer("1|Hello", "MAX_TOKENS")))
            .mount(&server)
            .await;
        let d = GeminiDriver::new(conn(&server, "gemini-2.5-pro"));
        assert_eq!(d.complete("s", "u").await.unwrap(), "1|Hello");
    }

    #[tokio::test]
    async fn http_errors_map_to_llm_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(GENERATE))
            .respond_with(ResponseTemplate::new(400).set_body_string(
                r#"{"error":{"code":400,"message":"API key not valid.","status":"INVALID_ARGUMENT"}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(STREAM))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "7"))
            .mount(&server)
            .await;
        let d = GeminiDriver::new(conn(&server, "gemini-2.5-pro"));
        let bad_key = d.complete("s", "u").await.unwrap_err();
        assert!(matches!(&bad_key, LlmError::Http { status: 400, body, .. } if body.contains("API key")));
        assert!(!bad_key.is_retryable());
        let throttled = d.stream(&req()).await.unwrap_err();
        assert!(throttled.is_retryable());
        assert_eq!(throttled.retry_after(), Some(std::time::Duration::from_secs(7)));
    }

    #[tokio::test]
    async fn list_models_keeps_generate_content_models_across_pages() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1beta/models"))
            .and(query_param("pageSize", "1000"))
            .and(query_param_is_missing("pageToken"))
            .and(header("x-goog-api-key", "k"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "models": [
                    {"name": "models/gemini-2.5-pro", "supportedGenerationMethods": ["generateContent", "countTokens"]},
                    {"name": "models/text-embedding-004", "supportedGenerationMethods": ["embedContent"]},
                    {"name": "models/no-methods-listed"}
                ],
                "nextPageToken": "p2/+="
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1beta/models"))
            .and(query_param("pageToken", "p2/+="))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "models": [
                    {"name": "models/gemini-3.1-pro-preview", "supportedGenerationMethods": ["generateContent"]}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let models = GeminiDriver::new(conn(&server, "gemini-2.5-pro")).list_models().await.unwrap();
        assert_eq!(models, vec!["gemini-2.5-pro", "gemini-3.1-pro-preview"]);
    }

    #[test]
    fn model_resource_names_are_used_as_given() {
        let url = |model: &str| GeminiDriver::new(sample(Provider::Google, model)).method_url("generateContent");
        let expected = "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:generateContent";
        assert_eq!(url("gemini-2.5-pro"), expected);
        assert_eq!(url("models/gemini-2.5-pro"), expected);
    }
}
