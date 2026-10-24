//! Connections commands (O1–O5, O21): the model library and the steps that
//! use it, the provider catalog, and connection testing / model listing.
//! Each mutation is a pure `store` helper (where the rules live and are
//! tested) behind a thin adapter: load, apply, persist, return the new state.

use tauri::AppHandle;

use crate::config::providers::{self, ModelCapabilities, ProviderInfo};
use crate::config::store;
use crate::config::{AppConfig, Connection, Driver, Model, Provider, Step, StepSettings};
use crate::error::{AppError, AppResult};
use crate::events::ErrorKind;
use crate::llm::create_driver;
use crate::llm::detect::detect_format;
use crate::llm::error::LlmError;
use crate::models::{ConnectionsConfig, FirstRunStatus, TestProblem, TestResult};

/// Pure: the page's slice of the config.
pub(crate) fn connections_config(cfg: &AppConfig) -> ConnectionsConfig {
    ConnectionsConfig {
        models: cfg.models.clone(),
        steps: cfg.steps.clone(),
        per_step: cfg.per_step,
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

/// Add a library model (empty `id`) or update one; `assign` (JS may omit it)
/// also hands that step this model. See `store::save_model`.
#[tauri::command]
pub fn save_model(app: AppHandle, model: Model, assign: Option<Step>) -> AppResult<ConnectionsConfig> {
    update(&app, |cfg| store::save_model(cfg, model, assign).map(drop))
}

/// Remove a library model; its steps move on (see `store::remove_model`).
#[tauri::command]
pub fn remove_model(app: AppHandle, id: String) -> AppResult<ConnectionsConfig> {
    update(&app, |cfg| store::remove_model(cfg, &id))
}

/// One step's model and settings; `null` turns web search off.
#[tauri::command]
pub fn set_step(
    app: AppHandle,
    step: Step,
    settings: Option<StepSettings>,
) -> AppResult<ConnectionsConfig> {
    update(&app, |cfg| store::set_step(cfg, step, settings))
}

/// The one-model view's Thinking / Tuning (see `store::set_shared_steps`).
#[tauri::command]
pub fn set_shared_steps(app: AppHandle, settings: StepSettings) -> AppResult<ConnectionsConfig> {
    update(&app, |cfg| store::set_shared_steps(cfg, settings))
}

#[tauri::command]
pub fn show_every_step(app: AppHandle) -> AppResult<ConnectionsConfig> {
    update(&app, store::show_every_step)
}

/// Back to one model, JS `modelId` (see `store::use_one_model`).
#[tauri::command]
pub fn use_one_model(app: AppHandle, model_id: String) -> AppResult<ConnectionsConfig> {
    update(&app, |cfg| store::use_one_model(cfg, &model_id))
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

/// What Connect's test and the model list talk to: the model alone, with no
/// step behind it — no Thinking, no step's Tuning. `tuned` fills in the
/// provider's default timeouts (the test's first call may wait for a local
/// model to load); untuned keeps the drivers' own fallbacks, as the model
/// list always had.
fn bare_connection(model: &Model, tuned: bool) -> Connection {
    let untouched = StepSettings::new(model.id.clone());
    if tuned {
        store::run_connection(model, &untouched)
    } else {
        Connection::from_step(model, &untouched)
    }
}

/// Connect's test: one tiny real request. `web_search` (JS `webSearch`, may be
/// omitted) also attaches the provider's search tool, so a model or account
/// that rejects it fails here instead of at glossary time.
#[tauri::command]
pub async fn test_connection(model: Model, web_search: Option<bool>) -> AppResult<TestResult> {
    Ok(run_test(&providers::canonicalize(model), web_search.unwrap_or(false)).await)
}

/// The test itself, on an already-canonical model (split out so tests can aim
/// it at a mock server; canonicalizing pins cloud providers' base URLs).
async fn run_test(model: &Model, web_search: bool) -> TestResult {
    let connection = bare_connection(model, true);
    let (driver, detected) = match resolve_driver(&connection).await {
        Ok(resolved) => resolved,
        // Undetermined format (both routes 404 / host unreachable): a graceful
        // result rather than an IPC-level error (spec §5.7).
        Err(_) => {
            let message = undetected_message(&connection);
            return failed(&connection, None, TestProblem::Address, true, message, None);
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
                detail: None,
            }
        }
        Err(e) => failed_from(&connection, detected, &e),
    }
}

/// A test that reached the provider and failed: what kind of problem, the
/// message for it, and the provider's own words for "Details".
fn failed_from(conn: &Connection, detected: Option<Driver>, err: &LlmError) -> TestResult {
    let (problem, can_save_anyway) = classify_test_error(err);
    let message = failure_message(conn, problem, err);
    failed(conn, detected, problem, can_save_anyway, message, Some(err.to_string()))
}

/// The models the server offers, for the model menu. `Err` (a message fit to
/// show) when the server couldn't be asked — unreachable, key rejected, no
/// recognisable API, an error reply — and `Ok(vec![])` only when it answered
/// with no models. The page reads the first as "can't reach" and the second as
/// "running, but no models installed"; the curated list stays either way.
#[tauri::command]
pub async fn list_models(model: Model) -> AppResult<Vec<String>> {
    let mut conn = bare_connection(&providers::canonicalize(model), false);
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
    detail: Option<String>,
) -> TestResult {
    TestResult {
        ok: false,
        model: conn.model.clone(),
        detected_driver: detected,
        message,
        problem: Some(problem),
        can_save_anyway,
        detail,
    }
}

/// Pure: which field a failed test points at, and whether saving untested is
/// reasonable — true only when the failure may pass on its own (unreachable,
/// timed out, rate-limited, server trouble), never for a bad key or model, and
/// not for an empty account (it needs credit first).
///
/// Classified exactly as a run classifies the same error ([`LlmError::kind`]),
/// so a test and a run never disagree: a 429 counts as billing only when it
/// says the account is out of credit (`insufficient_quota`), not when a
/// rate-limit message merely mentions quota or billing.
pub(crate) fn classify_test_error(err: &LlmError) -> (TestProblem, bool) {
    match err.kind() {
        ErrorKind::Auth => (TestProblem::Key, false),
        ErrorKind::Billing => (TestProblem::Billing, false),
        ErrorKind::RateLimit => (TestProblem::RateLimit, true),
        ErrorKind::Model => (TestProblem::Model, false),
        ErrorKind::Network => (TestProblem::Address, true),
        ErrorKind::Server => (TestProblem::Server, true),
        ErrorKind::Other => (TestProblem::Other, false),
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
        TestProblem::Model
        | TestProblem::Billing
        | TestProblem::RateLimit
        | TestProblem::Server
        | TestProblem::Other => format!("{} didn't list its models ({err}).", who(conn)),
    }
}

/// Pure: the message for a failed test. The page words each problem itself
/// (`error-copy.ts`); this is the fallback, and what logs and the model list
/// show. Billing, rate-limit and server trouble keep the provider's words.
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
        TestProblem::Billing | TestProblem::RateLimit | TestProblem::Server | TestProblem::Other => {
            err.to_string()
        }
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
    use crate::config::{sample, sample_model};
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn http(status: u16, body: &str) -> LlmError {
        LlmError::Http { status, body: body.into(), retry_after: None }
    }

    #[test]
    fn connections_config_mirrors_the_library_steps_and_keys() {
        let mut cfg = default_config();
        store::save_model(&mut cfg, sample_model(Provider::Anthropic, "claude-sonnet-5-5"), None).unwrap();
        store::show_every_step(&mut cfg).unwrap();
        let view = connections_config(&cfg);
        assert_eq!(view.models, cfg.models);
        assert_eq!(view.steps, cfg.steps);
        assert!(view.per_step);
        assert_eq!(view.provider_keys.get("anthropic").map(String::as_str), Some("k"));
        // The wire shape the page reads.
        let wire = serde_json::to_value(&view).unwrap();
        assert_eq!(wire["steps"]["web_search"]["model"], "m1");
        assert_eq!(wire["models"][0]["id"], "m1");
    }

    #[test]
    fn the_test_and_the_model_list_use_the_model_alone() {
        let mut ollama = sample_model(Provider::Ollama, "qwen3");
        ollama.connect_timeout = Some(3);
        // The test: provider-default tuning, no Thinking.
        let tested = bare_connection(&ollama, true);
        assert_eq!((tested.timeout, tested.thinking, tested.connect_timeout), (Some(600), None, Some(3)));
        assert_eq!(bare_connection(&sample_model(Provider::Anthropic, "claude-sonnet-5-5"), true).timeout, Some(180));
        // The list: nothing filled in, so the drivers' own fallbacks apply.
        let listed = bare_connection(&ollama, false);
        assert_eq!((listed.timeout, listed.max_tokens, listed.connect_timeout), (None, None, Some(3)));
        assert!(!tested.web_search && !listed.web_search);
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

    /// A Claude model on the mock server (`run_test` skips canonicalize,
    /// which would pin it back to api.anthropic.com).
    fn claude_on(server: &MockServer) -> Model {
        Model { base_url: server.uri(), ..sample_model(Provider::Anthropic, "claude-sonnet-5-5") }
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
        let searched = run_test(&claude_on(&server), true).await;
        assert!(searched.ok, "{}", searched.message);
        let plain = run_test(&claude_on(&server), false).await;
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
        let r = run_test(&claude_on(&server), true).await;
        assert!(!r.ok);
        assert_eq!((r.problem, r.can_save_anyway), (Some(TestProblem::Other), false));
        assert!(r.message.contains("web search is not enabled"), "{}", r.message);
    }

    fn ollama_on(base_url: String) -> Model {
        Model { base_url, ..sample_model(Provider::Ollama, "qwen3") }
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
        let mut custom = sample_model(Provider::Custom, "m");
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
        // Everything else.
        assert_eq!(classify_test_error(&http(422, "unprocessable")), (Other, false));
        assert_eq!(classify_test_error(&LlmError::Parse("html".into())), (Other, false));
    }

    #[test]
    fn out_of_credit_is_a_billing_problem() {
        use TestProblem::Billing;
        assert_eq!(classify_test_error(&http(402, "payment required")), (Billing, false));
        // Anthropic: a 400 about the credit balance.
        let anthropic = r#"{"type":"error","error":{"type":"invalid_request_error","message":"Your credit balance is too low to access the Anthropic API. Please go to Plans & Billing to upgrade or purchase credits."}}"#;
        assert_eq!(classify_test_error(&http(400, anthropic)), (Billing, false));
        // OpenAI: a 429 whose code is insufficient_quota.
        let openai = r#"{"error":{"message":"You exceeded your current quota, please check your plan and billing details.","type":"insufficient_quota","code":"insufficient_quota"}}"#;
        assert_eq!(classify_test_error(&http(429, openai)), (Billing, false));
        // A 403 that names billing.
        assert_eq!(classify_test_error(&http(403, "Billing is not enabled for this project")), (Billing, false));
    }

    #[test]
    fn throttling_is_a_rate_limit_problem() {
        use TestProblem::RateLimit;
        assert_eq!(classify_test_error(&http(429, "rate limited")), (RateLimit, true));
        assert_eq!(
            classify_test_error(&http(429, r#"{"type":"error","error":{"type":"rate_limit_error","message":"Number of requests has exceeded your rate limit"}}"#)),
            (RateLimit, true)
        );
        // Gemini says "quota" for a per-minute limit; RESOURCE_EXHAUSTED marks it as one.
        let gemini = r#"{"error":{"code":429,"message":"You exceeded your current quota, please check your plan and billing details.","status":"RESOURCE_EXHAUSTED"}}"#;
        assert_eq!(classify_test_error(&http(429, gemini)), (RateLimit, true));
        assert_eq!(classify_test_error(&http(429, "Resource has been exhausted (e.g. check quota).")), (RateLimit, true));
        // OpenAI free tier: a daily request limit that mentions adding a payment method and billing.
        let openai_free = r#"{"error":{"message":"Rate limit reached for gpt-4o-mini in organization org-abc on requests per day (RPD): Limit 200, Used 200, Requested 1. Please try again in 7m12s. Visit https://platform.openai.com/account/rate-limits to learn more. You can increase your rate limit by adding a payment method to your account at https://platform.openai.com/account/billing.","type":"requests","param":null,"code":"rate_limit_exceeded"}}"#;
        assert_eq!(classify_test_error(&http(429, openai_free)), (RateLimit, true));
        // Azure OpenAI: a token-rate limit whose link says "quotaincrease".
        let azure = r#"{"error":{"code":"429","message":"Requests to the ChatCompletions_Create Operation under Azure OpenAI API version 2024-10-21 have exceeded token rate limit of your current OpenAI S0 pricing tier. Please retry after 6 seconds. Please go here: https://aka.ms/oai/quotaincrease if you would like to further increase the default rate limit."}}"#;
        assert_eq!(classify_test_error(&http(429, azure)), (RateLimit, true));
    }

    #[test]
    fn a_test_classifies_errors_as_a_run_does() {
        let samples = [
            http(400, "Your credit balance is too low"),
            http(429, r#"{"error":{"code":"insufficient_quota"}}"#),
            http(429, "You exceeded your current quota, please check your plan and billing details."),
            http(403, "Billing is not enabled for this project"),
            http(401, "bad key"),
            http(404, "no such model"),
            http(503, "overloaded"),
            LlmError::Transport("dns error".into()),
        ];
        for e in &samples {
            let expected = match e.kind() {
                ErrorKind::Auth => TestProblem::Key,
                ErrorKind::Billing => TestProblem::Billing,
                ErrorKind::RateLimit => TestProblem::RateLimit,
                ErrorKind::Model => TestProblem::Model,
                ErrorKind::Network => TestProblem::Address,
                ErrorKind::Server => TestProblem::Server,
                ErrorKind::Other => TestProblem::Other,
            };
            assert_eq!(classify_test_error(e).0, expected, "{e}");
        }
    }

    #[test]
    fn server_trouble_is_a_server_problem() {
        use TestProblem::Server;
        for status in [500, 502, 503, 504, 529] {
            assert_eq!(classify_test_error(&http(status, "overloaded")), (Server, true), "{status}");
        }
    }

    #[test]
    fn a_failed_test_keeps_the_providers_own_words_for_details() {
        let ant = sample(Provider::Anthropic, "claude-sonnet-5-5");
        let err = http(529, r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#);
        let r = failed_from(&ant, None, &err);
        assert_eq!((r.problem, r.can_save_anyway), (Some(TestProblem::Server), true));
        assert_eq!(r.detail.as_deref(), Some(err.to_string().as_str()));
        // Problems with their own words name them in `message` too; the raw text stays in `detail`.
        let r = failed_from(&ant, None, &http(401, "invalid x-api-key"));
        assert!(r.message.starts_with("Anthropic rejected this key."), "{}", r.message);
        assert_eq!(r.detail.as_deref(), Some("HTTP 401: invalid x-api-key"));
    }

    #[test]
    fn problem_kinds_go_over_the_wire_like_run_error_kinds() {
        let wire = |p: TestProblem| serde_json::to_value(p).unwrap();
        assert_eq!(wire(TestProblem::Billing), "billing");
        assert_eq!(wire(TestProblem::RateLimit), "rate_limit");
        assert_eq!(wire(TestProblem::Server), "server");
        assert_eq!(wire(TestProblem::Key), "key");
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
