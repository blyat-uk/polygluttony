//! Connections commands (O1–O5, O21): the translation and web-search roles,
//! the provider catalog, and connection testing / model listing.

use tauri::AppHandle;

use crate::config::providers::{self, ModelCapabilities, ProviderInfo};
use crate::config::store;
use crate::config::{AppConfig, Connection, Driver, Provider, WebSearch};
use crate::error::{AppError, AppResult};
use crate::llm::create_driver;
use crate::llm::detect::detect_format;
use crate::llm::error::LlmError;
use crate::models::{ConnectionsConfig, FirstRunStatus, TestProblem, TestResult};

/// Pure: the page's slice of the config.
pub(crate) fn connections_config(cfg: &AppConfig) -> ConnectionsConfig {
    ConnectionsConfig {
        translation: cfg.translation.clone(),
        web_search: cfg.web_search.clone(),
        provider_keys: cfg.provider_keys.clone(),
    }
}

/// Load, apply `change`, persist, and hand the page its new state.
fn update(
    app: &AppHandle,
    change: impl FnOnce(&mut AppConfig) -> AppResult<()>,
) -> AppResult<ConnectionsConfig> {
    let mut cfg = store::load(app)?;
    change(&mut cfg)?;
    store::save(app, &cfg)?;
    Ok(connections_config(&cfg))
}

#[tauri::command]
pub fn get_connections(app: AppHandle) -> AppResult<ConnectionsConfig> {
    Ok(connections_config(&store::load(&app)?))
}

#[tauri::command]
pub fn list_providers() -> Vec<ProviderInfo> {
    providers::providers()
}

#[tauri::command]
pub fn model_capabilities(provider: Provider, model: String) -> ModelCapabilities {
    providers::capabilities(provider, &model)
}

/// Save the translation connection. Thinking is normalised to what the model
/// offers and web search follows the model (see `store::set_translation`).
#[tauri::command]
pub fn save_translation(app: AppHandle, connection: Connection) -> AppResult<ConnectionsConfig> {
    update(&app, |cfg| {
        store::set_translation(cfg, connection);
        Ok(())
    })
}

#[tauri::command]
pub fn remove_translation(app: AppHandle) -> AppResult<ConnectionsConfig> {
    update(&app, |cfg| {
        store::clear_translation(cfg);
        Ok(())
    })
}

#[tauri::command]
pub fn set_web_search(app: AppHandle, web_search: WebSearch) -> AppResult<ConnectionsConfig> {
    update(&app, |cfg| store::set_web_search(cfg, web_search))
}

#[tauri::command]
pub fn first_run_status(app: AppHandle) -> AppResult<FirstRunStatus> {
    Ok(FirstRunStatus {
        has_usable_connection: store::has_usable_connection(&store::load(&app)?),
    })
}

/// A cheap, valid probe of `c` on `driver`: no thinking, 16 tokens (the
/// Responses API's floor; enough with a search tool attached too). The search
/// tool rides along only when asked for and the model can search — exactly
/// when `store::resolve_web_search` would attach it at run time.
fn probe_connection(mut c: Connection, driver: Driver, web_search: bool) -> Connection {
    c.driver = driver;
    c.thinking = None;
    c.web_search = web_search && providers::capabilities(c.provider, &c.model).web_search;
    c.max_tokens = Some(16);
    c
}

/// The driver to talk to `conn` with, plus the detected one when detection
/// ran. Only Custom has an unknown API format; every other provider's driver
/// is fixed.
async fn resolve_driver(conn: &Connection) -> Result<(Driver, Option<Driver>), LlmError> {
    if conn.provider == Provider::Custom {
        let detected = detect_format(&conn.base_url, &conn.api_key).await?;
        Ok((detected, Some(detected)))
    } else {
        Ok((conn.driver, None))
    }
}

/// Connect's test: one tiny real request. `web_search` (JS `webSearch`, may be
/// omitted) also attaches the provider's search tool, so a model or account
/// that rejects it fails here instead of at glossary time.
#[tauri::command]
pub async fn test_connection(connection: Connection, web_search: Option<bool>) -> AppResult<TestResult> {
    Ok(run_test(providers::canonicalize(connection), web_search.unwrap_or(false)).await)
}

/// The test itself, on an already-canonical connection (split out so tests can
/// aim it at a mock server; canonicalizing pins cloud providers' base URLs).
async fn run_test(connection: Connection, web_search: bool) -> TestResult {
    let (driver, detected) = match resolve_driver(&connection).await {
        Ok(resolved) => resolved,
        // Undetermined format (both routes 404 / host unreachable): a graceful
        // result rather than an IPC-level error (spec §5.7).
        Err(_) => {
            let message = undetected_message(&connection);
            return failed(&connection, None, TestProblem::Address, true, message);
        }
    };

    let model = connection.model.clone();
    let client = create_driver(probe_connection(connection.clone(), driver, web_search));
    match client.complete("Reply with OK.", "ping").await {
        // A 200 response with empty content still proves the key + model are
        // valid — some reasoning models (e.g. GLM via z.ai) emit no text within
        // the tiny probe budget, and a search turn can spend it all on the
        // tool call. A connectivity test treats that as success.
        Ok(_) | Err(LlmError::Empty) => {
            let message = match detected {
                Some(d) => format!("✓ Detected {} — responded as {model}", driver_label(d)),
                None => format!("✓ Connection works — responded as {model}"),
            };
            TestResult {
                ok: true,
                model,
                detected_driver: detected,
                message,
                problem: None,
                can_save_anyway: false,
            }
        }
        Err(e) => {
            let (problem, can_save_anyway) = classify_test_error(&e);
            let message = failure_message(&connection, problem, &e);
            failed(&connection, detected, problem, can_save_anyway, message)
        }
    }
}

/// The models the server offers, for the model menu. `Err` (a message fit to
/// show) when the server couldn't be asked — unreachable, key rejected, no
/// recognisable API, an error reply — and `Ok(vec![])` only when it answered
/// with no models. The page reads the first as "can't reach" and the second as
/// "running, but no models installed"; the curated list stays either way.
#[tauri::command]
pub async fn list_models(connection: Connection) -> AppResult<Vec<String>> {
    let mut conn = providers::canonicalize(connection);
    let (driver, _) =
        resolve_driver(&conn).await.map_err(|_| AppError::Other(undetected_message(&conn)))?;
    conn.driver = driver;
    let listed = create_driver(conn.clone()).list_models().await;
    listed.map_err(|e| AppError::Other(list_failure_message(&conn, &e)))
}

fn failed(
    conn: &Connection,
    detected: Option<Driver>,
    problem: TestProblem,
    can_save_anyway: bool,
    message: String,
) -> TestResult {
    TestResult {
        ok: false,
        model: conn.model.clone(),
        detected_driver: detected,
        message,
        problem: Some(problem),
        can_save_anyway,
    }
}

/// Pure: which field a failed test points at, and whether saving untested is
/// reasonable — true only when the failure may pass on its own (unreachable,
/// timed out, rate-limited, server trouble), never for a bad key or model.
pub(crate) fn classify_test_error(err: &LlmError) -> (TestProblem, bool) {
    match err {
        LlmError::Http { status: 401 | 403, .. } => (TestProblem::Key, false),
        LlmError::Http { status: 404, .. } => (TestProblem::Model, false),
        LlmError::Http { status: 400, body, .. } => {
            let body = body.to_ascii_lowercase();
            // Gemini answers a bad key with 400 API_KEY_INVALID, not 401.
            if body.contains("api key") || body.contains("api_key") {
                (TestProblem::Key, false)
            } else if body.contains("model") {
                (TestProblem::Model, false)
            } else {
                (TestProblem::Other, false)
            }
        }
        LlmError::Http { status: 408, .. } | LlmError::Transport(_) => (TestProblem::Address, true),
        LlmError::Http { status: 429 | 500..=599, .. } => (TestProblem::Other, true),
        LlmError::Http { .. } | LlmError::Parse(_) | LlmError::Declined(_) | LlmError::Empty => {
            (TestProblem::Other, false)
        }
    }
}

/// `host[:port]` of a base URL, for messages.
fn host_of(base_url: &str) -> &str {
    let url = base_url.trim();
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?', '#']).next().filter(|h| !h.is_empty()).unwrap_or(url)
}

/// Who answered, for messages: the provider's name, or "The server" for Custom.
fn who(conn: &Connection) -> String {
    if conn.provider == Provider::Custom {
        "The server".to_string()
    } else {
        providers::provider_info(conn.provider).label
    }
}

/// Pure: a Custom URL where neither API format answered.
fn undetected_message(conn: &Connection) -> String {
    format!(
        "No OpenAI- or Anthropic-compatible API answered at {}. Check the URL.",
        host_of(&conn.base_url)
    )
}

/// Pure: why the model list couldn't be fetched — in a failed test's words
/// for a bad key or an unreachable server. A 404 here means the server has no
/// model list, not that it doesn't know a model.
fn list_failure_message(conn: &Connection, err: &LlmError) -> String {
    match classify_test_error(err).0 {
        problem @ (TestProblem::Key | TestProblem::Address) => failure_message(conn, problem, err),
        TestProblem::Model | TestProblem::Other => {
            format!("{} didn't list its models ({err}).", who(conn))
        }
    }
}

/// Pure: the message shown with a failed test.
fn failure_message(conn: &Connection, problem: TestProblem, err: &LlmError) -> String {
    let info = providers::provider_info(conn.provider);
    let who = who(conn);
    match problem {
        TestProblem::Key => {
            let hint = info.key_hint.map(|h| format!(" ({h})")).unwrap_or_default();
            format!("{who} rejected this key. Check you copied all of it{hint}.")
        }
        TestProblem::Model => format!("{who} doesn't recognise the model “{}”.", conn.model),
        TestProblem::Address if conn.provider.is_local() => {
            format!("Can't reach {} at {} — is it running?", info.label, host_of(&conn.base_url))
        }
        TestProblem::Address => format!(
            "Couldn't reach {}. Check your internet connection or proxy. ({err})",
            host_of(&conn.base_url)
        ),
        TestProblem::Other => err.to_string(),
    }
}

fn driver_label(d: Driver) -> &'static str {
    match d {
        Driver::Anthropic => "Anthropic-compatible",
        Driver::Openai | Driver::OpenaiResponses => "OpenAI-compatible",
        Driver::Gemini => "Gemini",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::providers::default_config;
    use crate::config::sample;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn http(status: u16, body: &str) -> LlmError {
        LlmError::Http { status, body: body.into(), retry_after: None }
    }

    #[test]
    fn connections_config_mirrors_the_roles_and_keys() {
        let mut cfg = default_config();
        store::set_translation(&mut cfg, sample(Provider::Anthropic, "claude-sonnet-5-5"));
        let view = connections_config(&cfg);
        assert_eq!(view.translation, cfg.translation);
        assert_eq!(view.web_search, WebSearch::Translation);
        assert_eq!(view.provider_keys.get("anthropic").map(String::as_str), Some("k"));
    }

    #[test]
    fn probe_is_cheap_and_tool_free() {
        let mut c = sample(Provider::Openai, "gpt-5.5");
        c.thinking = Some(crate::config::ThinkingLevel::High);
        c.web_search = true;
        c.max_tokens = Some(32000);
        let p = probe_connection(c, Driver::OpenaiResponses, false);
        assert_eq!((p.thinking, p.web_search, p.max_tokens), (None, false, Some(16)));
    }

    #[test]
    fn probe_attaches_search_only_when_asked_and_the_model_can_search() {
        let gpt = sample(Provider::Openai, "gpt-5.5");
        let p = probe_connection(gpt, Driver::OpenaiResponses, true);
        assert_eq!((p.thinking, p.web_search, p.max_tokens), (None, true, Some(16)));
        // At run time these never search, so the probe doesn't either.
        for c in [sample(Provider::Ollama, "qwen3"), sample(Provider::Openai, "o3-mini")] {
            let driver = c.driver;
            assert!(!probe_connection(c, driver, true).web_search);
        }
    }

    fn json_reply(body: serde_json::Value) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(body)
    }

    async fn sent_bodies(server: &MockServer) -> Vec<serde_json::Value> {
        let requests = server.received_requests().await.unwrap();
        requests.iter().map(|r| r.body_json().unwrap()).collect()
    }

    /// A Claude connection on the mock server (`run_test` skips canonicalize,
    /// which would pin it back to api.anthropic.com).
    fn claude_on(server: &MockServer) -> Connection {
        Connection { base_url: server.uri(), ..sample(Provider::Anthropic, "claude-sonnet-5-5") }
    }

    #[tokio::test]
    async fn a_web_search_test_sends_the_tool_and_counts_a_bare_reply_as_working() {
        let server = MockServer::start().await;
        // The probe budget went on the search call: no text, still a success.
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(json_reply(json!({
                "stop_reason": "max_tokens",
                "content": [{"type": "server_tool_use", "id": "srv_1", "name": "web_search", "input": {}}],
                "usage": {"input_tokens": 9, "output_tokens": 16}
            })))
            .mount(&server)
            .await;
        let searched = run_test(claude_on(&server), true).await;
        assert!(searched.ok, "{}", searched.message);
        let plain = run_test(claude_on(&server), false).await;
        assert!(plain.ok, "{}", plain.message);

        let bodies = sent_bodies(&server).await;
        assert_eq!(bodies[0]["tools"][0]["name"], "web_search");
        assert_eq!(bodies[0]["max_tokens"], 16);
        assert!(bodies[1].get("tools").is_none(), "a plain test attaches no tool");
    }

    #[tokio::test]
    async fn a_rejected_search_tool_fails_the_test() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(400).set_body_string(
                r#"{"type":"error","error":{"type":"invalid_request_error","message":"web search is not enabled for this organization"}}"#,
            ))
            .mount(&server)
            .await;
        let r = run_test(claude_on(&server), true).await;
        assert!(!r.ok);
        assert_eq!((r.problem, r.can_save_anyway), (Some(TestProblem::Other), false));
        assert!(r.message.contains("web search is not enabled"), "{}", r.message);
    }

    fn ollama_on(base_url: String) -> Connection {
        Connection { base_url, ..sample(Provider::Ollama, "qwen3") }
    }

    /// A port nothing listens on.
    fn closed_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    #[tokio::test]
    async fn list_models_tells_no_models_from_an_error_reply() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/empty/models"))
            .respond_with(json_reply(json!({"object": "list", "data": []})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/full/models"))
            .respond_with(json_reply(json!({"object": "list", "data": [{"id": "qwen3"}]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/broken/models"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;
        let list = |dir: &str| list_models(ollama_on(format!("{}/{dir}", server.uri())));
        // Running, but nothing installed.
        assert_eq!(list("empty").await.unwrap(), Vec::<String>::new());
        assert_eq!(list("full").await.unwrap(), vec!["qwen3"]);
        let err = list("broken").await.unwrap_err().to_string();
        assert_eq!(err, "Ollama didn't list its models (HTTP 500: boom).");
    }

    #[tokio::test]
    async fn list_models_reports_an_unreachable_server() {
        let port = closed_port();
        let err = list_models(ollama_on(format!("http://127.0.0.1:{port}/v1"))).await.unwrap_err();
        assert_eq!(err.to_string(), format!("Can't reach Ollama at 127.0.0.1:{port} — is it running?"));

        // A Custom URL is detected first; nothing answering is an error too.
        let mut custom = sample(Provider::Custom, "m");
        custom.base_url = format!("http://127.0.0.1:{port}/v1");
        let err = list_models(custom).await.unwrap_err().to_string();
        assert!(err.starts_with("No OpenAI- or Anthropic-compatible API answered at 127.0.0.1:"), "{err}");
    }

    #[test]
    fn list_failures_reuse_the_tests_words_for_key_and_address() {
        let gemini = sample(Provider::Google, "gemini-2.5-pro");
        let bad_key = http(400, r#"{"error":{"message":"API key not valid."}}"#);
        assert!(list_failure_message(&gemini, &bad_key).starts_with("Google rejected this key."));
        // No model list at all isn't "doesn't recognise the model".
        let mut custom = sample(Provider::Custom, "m");
        custom.base_url = "https://gw.example.com/v1".into();
        assert_eq!(
            list_failure_message(&custom, &http(404, "not found")),
            "The server didn't list its models (HTTP 404: not found)."
        );
    }

    #[test]
    fn test_errors_point_at_the_field_that_caused_them() {
        use TestProblem::*;
        assert_eq!(classify_test_error(&http(401, "invalid x-api-key")), (Key, false));
        assert_eq!(classify_test_error(&http(403, "forbidden")), (Key, false));
        assert_eq!(classify_test_error(&http(404, "not_found_error: model")), (Model, false));
        assert_eq!(
            classify_test_error(&http(400, r#"{"error":{"message":"The model `x` does not exist"}}"#)),
            (Model, false)
        );
        assert_eq!(
            classify_test_error(&http(400, r#"{"error":{"message":"API key not valid.","status":"INVALID_ARGUMENT"}}"#)),
            (Key, false)
        );
        assert_eq!(classify_test_error(&http(400, "max_tokens: too large")), (Other, false));
        // Possibly temporary: offer "Save anyway".
        assert_eq!(classify_test_error(&LlmError::Transport("connection refused".into())), (Address, true));
        assert_eq!(classify_test_error(&http(408, "timeout")), (Address, true));
        assert_eq!(classify_test_error(&http(429, "rate limited")), (Other, true));
        assert_eq!(classify_test_error(&http(500, "boom")), (Other, true));
        assert_eq!(classify_test_error(&http(529, "overloaded")), (Other, true));
        // Everything else.
        assert_eq!(classify_test_error(&http(422, "unprocessable")), (Other, false));
        assert_eq!(classify_test_error(&LlmError::Parse("html".into())), (Other, false));
    }

    #[test]
    fn failure_messages_name_the_provider_and_host() {
        let ant = sample(Provider::Anthropic, "claude-nope");
        let key = failure_message(&ant, TestProblem::Key, &http(401, ""));
        assert_eq!(key, "Anthropic rejected this key. Check you copied all of it (Anthropic keys start with sk-ant-).");
        let model = failure_message(&ant, TestProblem::Model, &http(404, ""));
        assert_eq!(model, "Anthropic doesn't recognise the model “claude-nope”.");
        let refused = LlmError::Transport("connection refused".into());
        let cloud = failure_message(&ant, TestProblem::Address, &refused);
        assert!(cloud.starts_with("Couldn't reach api.anthropic.com."), "{cloud}");

        // Local servers: name the host and ask whether it's running.
        let ollama = sample(Provider::Ollama, "qwen3");
        assert_eq!(
            failure_message(&ollama, TestProblem::Address, &refused),
            "Can't reach Ollama at localhost:11434 — is it running?"
        );

        let mut custom = sample(Provider::Custom, "m");
        custom.base_url = "https://gw.example.com/v1".into();
        assert_eq!(
            failure_message(&custom, TestProblem::Key, &http(401, "")),
            "The server rejected this key. Check you copied all of it."
        );
        assert_eq!(failure_message(&custom, TestProblem::Other, &http(422, "bad")), "HTTP 422: bad");
    }

    #[test]
    fn host_of_strips_scheme_and_path() {
        assert_eq!(host_of("https://api.openai.com/v1"), "api.openai.com");
        assert_eq!(host_of("http://localhost:1234/v1"), "localhost:1234");
        assert_eq!(host_of("gw.example.com"), "gw.example.com");
        assert_eq!(host_of(""), "");
    }
}
