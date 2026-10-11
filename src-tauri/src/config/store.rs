//! Config persistence. Pure `AppConfig` helpers (unit-tested), the tolerant
//! loader, and a thin Tauri store adapter (`load`/`save`).

use std::collections::BTreeMap;

use serde_json::{Map, Value};
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

use crate::config::providers::{self, capabilities, default_config, model_label, normalize_thinking};
use crate::config::{AppConfig, Connection, WebSearch, CONFIG_VERSION};
use crate::error::{AppError, AppResult};

const STORE_FILE: &str = "config.json";
const STORE_KEY: &str = "app";

// ---- usability & role resolution -------------------------------------------

fn is_localhost(base_url: &str) -> bool {
    let url = base_url.to_ascii_lowercase();
    url.contains("localhost") || url.contains("127.0.0.1") || url.contains("[::1]")
}

/// A connection can make requests: it has a key, or it talks to a server that
/// needs none (a local provider, or any localhost URL).
pub fn connection_is_usable(conn: &Connection) -> bool {
    !conn.api_key.trim().is_empty() || conn.provider.is_local() || is_localhost(&conn.base_url)
}

/// Fill unset tuning from the provider's defaults — what the UI shows for an
/// untouched Tuning section — so a run uses the values the user sees.
fn with_tuning_defaults(mut conn: Connection) -> Connection {
    let t = providers::provider_info(conn.provider).tuning;
    conn.max_tokens.get_or_insert(t.max_tokens);
    conn.batch_dialogue_limit.get_or_insert(t.batch_dialogue_limit);
    conn.timeout.get_or_insert(t.timeout);
    conn.concurrency.get_or_insert(t.concurrency);
    conn
}

/// The translation connection, ready to run, iff it is usable.
pub fn usable_translation(cfg: &AppConfig) -> Option<Connection> {
    cfg.translation
        .as_ref()
        .filter(|c| connection_is_usable(c))
        .cloned()
        .map(with_tuning_defaults)
}

/// First-run check (O21): can anything translate yet?
pub fn has_usable_connection(cfg: &AppConfig) -> bool {
    usable_translation(cfg).is_some()
}

/// The connection that answers web-search calls, with its runtime
/// `web_search` flag on — iff the role is set, the connection is usable and
/// its model can search.
pub fn resolve_web_search(cfg: &AppConfig) -> Option<Connection> {
    let conn = match &cfg.web_search {
        WebSearch::Off => return None,
        WebSearch::Translation => cfg.translation.as_ref()?,
        WebSearch::Separate { connection } => connection,
    };
    if !connection_is_usable(conn) || !capabilities(conn.provider, &conn.model).web_search {
        return None;
    }
    let mut conn = with_tuning_defaults(conn.clone());
    conn.web_search = true;
    Some(conn)
}

/// Label of the resolved web-search model ("Claude Sonnet 5.5"), if any.
pub fn web_search_label(cfg: &AppConfig) -> Option<String> {
    resolve_web_search(cfg).map(|c| model_label(c.provider, &c.model))
}

fn can_search(conn: &Connection) -> bool {
    capabilities(conn.provider, &conn.model).web_search
}

// ---- mutations --------------------------------------------------------------

/// Cloud keys are account-wide, so remember them for the next connection to
/// the same provider (prefilled in the form, reused by a separate search model).
fn remember_key(cfg: &mut AppConfig, conn: &Connection) {
    if conn.provider.is_cloud() && !conn.api_key.is_empty() {
        cfg.provider_keys.insert(conn.provider.id().to_string(), conn.api_key.clone());
    }
}

/// Save the translation connection (Connect, a model change, a Tuning save).
pub fn set_translation(cfg: &mut AppConfig, conn: Connection) {
    let mut conn = providers::canonicalize(conn);
    conn.thinking = normalize_thinking(conn.provider, &conn.model, conn.thinking);
    conn.web_search = false;
    remember_key(cfg, &conn);
    let searches = can_search(&conn);
    match cfg.web_search {
        // First setup: web search starts on when the model can do it.
        WebSearch::Off if cfg.translation.is_none() && searches => {
            cfg.web_search = WebSearch::Translation;
        }
        WebSearch::Translation if !searches => cfg.web_search = WebSearch::Off,
        _ => {}
    }
    cfg.translation = Some(conn);
}

/// Remove the translation connection; a web search riding on it goes too.
pub fn clear_translation(cfg: &mut AppConfig) {
    cfg.translation = None;
    if cfg.web_search == WebSearch::Translation {
        cfg.web_search = WebSearch::Off;
    }
}

/// Set the web-search role. `Translation` needs a translation model that can
/// search; a separate model must be able to search itself.
pub fn set_web_search(cfg: &mut AppConfig, web_search: WebSearch) -> AppResult<()> {
    let web_search = match web_search {
        WebSearch::Off => WebSearch::Off,
        WebSearch::Translation => {
            if !cfg.translation.as_ref().is_some_and(can_search) {
                return Err(AppError::Other("your translation model can't search the web".into()));
            }
            WebSearch::Translation
        }
        WebSearch::Separate { connection } => {
            let mut conn = providers::canonicalize(connection);
            if !can_search(&conn) {
                return Err(AppError::Other(format!(
                    "{} can't search the web",
                    model_label(conn.provider, &conn.model)
                )));
            }
            // It makes one call per glossary build: no Thinking control here.
            conn.thinking = None;
            conn.web_search = false;
            remember_key(cfg, &conn);
            WebSearch::Separate { connection: conn }
        }
    };
    cfg.web_search = web_search;
    Ok(())
}

/// Update the global default source/target languages. These seed a newly opened
/// folder that has no saved per-folder preferences (and persist across sessions).
pub fn set_default_languages(cfg: &mut AppConfig, source: &str, target: &str) {
    cfg.default_source = source.to_string();
    cfg.default_target = target.to_string();
}

// ---- loading: tolerant parsing + v1 migration -------------------------------

/// Parse the stored config, keeping whatever parses and defaulting the rest, so a
/// config written by another build (or edited by hand) never blocks startup.
/// Fallbacks are logged.
pub fn config_from_stored(value: Value) -> AppConfig {
    match value.as_object() {
        Some(stored) => config_from_v2(stored),
        None => {
            tracing::warn!("config is not an object; using the defaults");
            default_config()
        }
    }
}

/// One stored field: `None` when it is absent or doesn't parse (logged).
/// `what` names the object for the log ("config", "connection `x`").
fn field<T: serde::de::DeserializeOwned>(
    stored: &Map<String, Value>,
    key: &str,
    what: &str,
) -> Option<T> {
    serde_json::from_value(stored.get(key)?.clone())
        .map_err(|e| tracing::warn!("{what} field `{key}` is invalid ({e}); using the default"))
        .ok()
}

/// An optional numeric/enum field: absent, null and invalid all mean unset.
fn optional<T: serde::de::DeserializeOwned>(
    stored: &Map<String, Value>,
    key: &str,
    what: &str,
) -> Option<T> {
    field::<Option<T>>(stored, key, what).flatten()
}

/// `translation` with no search-capable translation behind it means nothing.
fn normalize_roles(cfg: &mut AppConfig) {
    if cfg.web_search == WebSearch::Translation && !cfg.translation.as_ref().is_some_and(can_search) {
        tracing::warn!("web search was set to the translation model, which can't search; turning it off");
        cfg.web_search = WebSearch::Off;
    }
}

/// A stored v2 connection, field by field: the identifying fields must parse,
/// the rest fall back to unset. `None` (logged) when it can't be a connection.
fn connection_from_value(value: &Value, what: &str) -> Option<Connection> {
    let Some(map) = value.as_object() else {
        tracing::warn!("{what} is not an object; dropping it");
        return None;
    };
    let (Some(provider), Some(driver), Some(base_url), Some(model)) = (
        field(map, "provider", what),
        field(map, "driver", what),
        field(map, "base_url", what),
        field(map, "model", what),
    ) else {
        tracing::warn!("{what} lacks a valid provider, driver, base_url or model; dropping it");
        return None;
    };
    Some(Connection {
        provider,
        driver,
        base_url,
        api_key: field(map, "api_key", what).unwrap_or_default(),
        model,
        thinking: optional(map, "thinking", what),
        max_tokens: optional(map, "max_tokens", what),
        batch_dialogue_limit: optional(map, "batch_dialogue_limit", what),
        timeout: optional(map, "timeout", what),
        connect_timeout: optional(map, "connect_timeout", what),
        concurrency: optional(map, "concurrency", what),
        web_search: false,
    })
}

fn web_search_from_value(value: &Value) -> WebSearch {
    match value.get("mode").and_then(Value::as_str) {
        Some("off") => WebSearch::Off,
        Some("translation") => WebSearch::Translation,
        Some("separate") => value
            .get("connection")
            .and_then(|c| connection_from_value(c, "web search connection"))
            .map_or(WebSearch::Off, |connection| WebSearch::Separate { connection }),
        _ => {
            tracing::warn!("config field `web_search` is invalid; using the default");
            WebSearch::Off
        }
    }
}

/// Remembered keys, entry by entry (a bad entry drops only itself).
fn provider_keys_from(stored: &Map<String, Value>) -> BTreeMap<String, String> {
    match stored.get("provider_keys") {
        Some(Value::Object(entries)) => entries
            .iter()
            .filter_map(|(provider, key)| match key.as_str() {
                Some(key) => Some((provider.clone(), key.to_string())),
                None => {
                    tracing::warn!("dropping invalid remembered key for `{provider}`");
                    None
                }
            })
            .collect(),
        None | Some(Value::Null) => BTreeMap::new(),
        Some(_) => {
            tracing::warn!("config field `provider_keys` is invalid; using the default");
            BTreeMap::new()
        }
    }
}

fn config_from_v2(stored: &Map<String, Value>) -> AppConfig {
    if let Some(v) = field::<u64>(stored, "version", "config").filter(|v| *v > CONFIG_VERSION as u64) {
        tracing::warn!("config is v{v}, newer than this build (v{CONFIG_VERSION}); loading what it understands");
    }
    let defaults = default_config();
    let mut cfg = AppConfig {
        version: CONFIG_VERSION,
        default_source: field(stored, "default_source", "config").unwrap_or(defaults.default_source),
        default_target: field(stored, "default_target", "config").unwrap_or(defaults.default_target),
        default_workdir: optional(stored, "default_workdir", "config"),
        translation: stored
            .get("translation")
            .filter(|v| !v.is_null())
            .and_then(|v| connection_from_value(v, "translation connection")),
        web_search: stored.get("web_search").map(web_search_from_value).unwrap_or_default(),
        provider_keys: provider_keys_from(stored),
    };
    normalize_roles(&mut cfg);
    cfg
}

// ---- Tauri store adapter (thin; not unit-tested) ---------------------------

/// Load the config from the store, seeding + persisting defaults on first run.
pub fn load<R: Runtime>(app: &AppHandle<R>) -> AppResult<AppConfig> {
    let store = app.store(STORE_FILE).map_err(|e| AppError::Other(e.to_string()))?;
    match store.get(STORE_KEY) {
        Some(value) => Ok(config_from_stored(value)),
        None => {
            let cfg = default_config();
            store.set(STORE_KEY, serde_json::to_value(&cfg)?);
            store.save().map_err(|e| AppError::Other(e.to_string()))?;
            Ok(cfg)
        }
    }
}

/// Persist the whole config.
pub fn save<R: Runtime>(app: &AppHandle<R>, cfg: &AppConfig) -> AppResult<()> {
    let store = app.store(STORE_FILE).map_err(|e| AppError::Other(e.to_string()))?;
    store.set(STORE_KEY, serde_json::to_value(cfg)?);
    store.save().map_err(|e| AppError::Other(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{sample, Driver, Provider};
    use serde_json::json;
    use crate::config::ThinkingLevel::{High, Medium};

    fn with_translation(conn: Connection) -> AppConfig {
        AppConfig { translation: Some(conn), ..default_config() }
    }

    // ---- usability & resolution ----

    #[test]
    fn connection_is_usable_rules() {
        let mut c = sample(Provider::Custom, "m");
        c.base_url = "https://api.example.com".into();
        c.api_key.clear();
        assert!(!connection_is_usable(&c), "no key, remote URL");
        c.api_key = "sk-test".into();
        assert!(connection_is_usable(&c), "key");
        c.api_key = "   ".into();
        assert!(!connection_is_usable(&c), "whitespace isn't a key");
        c.base_url = "http://localhost:8000/v1".into();
        assert!(connection_is_usable(&c), "localhost");
        c.base_url = "http://127.0.0.1:8000/v1".into();
        assert!(connection_is_usable(&c), "127.0.0.1");
        // Local providers need no key, even on another machine.
        let mut ollama = sample(Provider::Ollama, "qwen3");
        ollama.api_key.clear();
        ollama.base_url = "http://192.168.1.5:11434/v1".into();
        assert!(connection_is_usable(&ollama));
    }

    #[test]
    fn usable_translation_requires_a_usable_connection_and_fills_tuning() {
        assert!(usable_translation(&default_config()).is_none());
        assert!(!has_usable_connection(&default_config()));

        let mut keyless = sample(Provider::Anthropic, "claude-sonnet-5-5");
        keyless.api_key.clear();
        assert!(usable_translation(&with_translation(keyless)).is_none());

        let mut conn = sample(Provider::Ollama, "qwen3");
        conn.concurrency = Some(4);
        let cfg = with_translation(conn);
        assert!(has_usable_connection(&cfg));
        let run = usable_translation(&cfg).unwrap();
        // Unset tuning comes from the provider defaults; explicit values stay.
        assert_eq!(run.batch_dialogue_limit, Some(40));
        assert_eq!(run.timeout, Some(600));
        assert_eq!(run.max_tokens, Some(8192));
        assert_eq!(run.concurrency, Some(4));
        assert!(!run.web_search);
    }

    #[test]
    fn resolve_web_search_rules() {
        let claude = sample(Provider::Anthropic, "claude-sonnet-5-5");
        let mut cfg = with_translation(claude.clone());
        assert_eq!(resolve_web_search(&cfg), None, "off");

        cfg.web_search = WebSearch::Translation;
        let resolved = resolve_web_search(&cfg).unwrap();
        assert!(resolved.web_search);
        assert_eq!(resolved.model, "claude-sonnet-5-5");
        assert_eq!(web_search_label(&cfg).as_deref(), Some("Claude Sonnet 5.5"));

        // Unusable translation → nothing resolves.
        cfg.translation.as_mut().unwrap().api_key.clear();
        assert_eq!(resolve_web_search(&cfg), None);

        // A separate model resolves on its own, translation or not.
        let gpt = sample(Provider::Openai, "gpt-5.5");
        cfg.web_search = WebSearch::Separate { connection: gpt };
        let resolved = resolve_web_search(&cfg).unwrap();
        assert_eq!(resolved.provider, Provider::Openai);
        assert!(resolved.web_search);
        assert_eq!(resolved.timeout, Some(180), "tuning defaults filled");
        assert_eq!(web_search_label(&cfg).as_deref(), Some("GPT-5.5"));

        // A model that can't search never resolves (e.g. hand-edited config).
        cfg.web_search = WebSearch::Separate { connection: sample(Provider::Ollama, "qwen3") };
        assert_eq!(resolve_web_search(&cfg), None);
        assert_eq!(web_search_label(&cfg), None);
    }

    // ---- mutations ----

    #[test]
    fn first_setup_turns_web_search_on_when_the_model_can_search() {
        let mut cfg = default_config();
        set_translation(&mut cfg, sample(Provider::Anthropic, "claude-sonnet-5-5"));
        assert_eq!(cfg.web_search, WebSearch::Translation);

        let mut local = default_config();
        set_translation(&mut local, sample(Provider::Ollama, "qwen3"));
        assert_eq!(local.web_search, WebSearch::Off);
    }

    #[test]
    fn later_saves_keep_a_user_chosen_web_search() {
        let mut cfg = with_translation(sample(Provider::Anthropic, "claude-sonnet-5-5"));
        // The user turned it off: a model change doesn't turn it back on.
        set_translation(&mut cfg, sample(Provider::Anthropic, "claude-opus-5-5"));
        assert_eq!(cfg.web_search, WebSearch::Off);

        // A separate model survives any translation change.
        let sep = WebSearch::Separate { connection: sample(Provider::Openai, "gpt-5.5") };
        cfg.web_search = sep.clone();
        set_translation(&mut cfg, sample(Provider::Ollama, "qwen3"));
        assert_eq!(cfg.web_search, sep);
    }

    #[test]
    fn translation_web_search_turns_off_when_the_new_model_cant_search() {
        let mut cfg = with_translation(sample(Provider::Anthropic, "claude-sonnet-5-5"));
        cfg.web_search = WebSearch::Translation;
        set_translation(&mut cfg, sample(Provider::Openai, "gpt-5.5"));
        assert_eq!(cfg.web_search, WebSearch::Translation, "still searches");
        set_translation(&mut cfg, sample(Provider::Ollama, "qwen3"));
        assert_eq!(cfg.web_search, WebSearch::Off);
    }

    #[test]
    fn set_translation_normalises_and_remembers_cloud_keys() {
        let mut cfg = default_config();
        let mut conn = sample(Provider::Google, "gemini-3-flash-preview");
        conn.thinking = Some(Medium); // not a Gemini 3 level
        conn.api_key = " AIza-key ".into();
        conn.driver = Driver::Openai;
        conn.web_search = true;
        set_translation(&mut cfg, conn);
        let saved = cfg.translation.as_ref().unwrap();
        assert_eq!(saved.thinking, None);
        assert_eq!(saved.driver, Driver::Gemini);
        assert_eq!(saved.api_key, "AIza-key");
        assert!(!saved.web_search, "the runtime flag is never stored");
        assert_eq!(cfg.provider_keys.get("google").map(String::as_str), Some("AIza-key"));

        // Adaptive Claude always has a level selected.
        set_translation(&mut cfg, sample(Provider::Anthropic, "claude-opus-5-5"));
        assert_eq!(cfg.translation.as_ref().unwrap().thinking, Some(Medium));

        // Local and custom keys aren't account keys.
        let mut custom = sample(Provider::Custom, "m");
        custom.base_url = "https://gw.example.com/v1".into();
        custom.api_key = "gw-key".into();
        set_translation(&mut cfg, custom);
        assert!(!cfg.provider_keys.contains_key("custom"));
    }

    #[test]
    fn clear_translation_drops_a_web_search_riding_on_it() {
        let mut cfg = with_translation(sample(Provider::Anthropic, "claude-sonnet-5-5"));
        cfg.web_search = WebSearch::Translation;
        clear_translation(&mut cfg);
        assert_eq!(cfg.translation, None);
        assert_eq!(cfg.web_search, WebSearch::Off);

        let sep = WebSearch::Separate { connection: sample(Provider::Openai, "gpt-5.5") };
        let mut cfg = AppConfig { web_search: sep.clone(), ..with_translation(sample(Provider::Ollama, "q")) };
        clear_translation(&mut cfg);
        assert_eq!(cfg.web_search, sep);
    }

    #[test]
    fn set_web_search_translation_needs_a_searching_translation() {
        let mut cfg = default_config();
        assert!(set_web_search(&mut cfg, WebSearch::Translation).is_err(), "no translation");
        let mut cfg = with_translation(sample(Provider::Ollama, "qwen3"));
        assert!(set_web_search(&mut cfg, WebSearch::Translation).is_err(), "can't search");
        assert_eq!(cfg.web_search, WebSearch::Off);

        let mut cfg = with_translation(sample(Provider::Openai, "gpt-5.5"));
        set_web_search(&mut cfg, WebSearch::Translation).unwrap();
        assert_eq!(cfg.web_search, WebSearch::Translation);
        set_web_search(&mut cfg, WebSearch::Off).unwrap();
        assert_eq!(cfg.web_search, WebSearch::Off);
    }

    #[test]
    fn set_web_search_separate_drops_thinking_and_remembers_the_key() {
        let mut cfg = with_translation(sample(Provider::Ollama, "qwen3"));
        let mut gpt = sample(Provider::Openai, "gpt-5.5");
        gpt.thinking = Some(High);
        gpt.api_key = "sk-search".into();
        set_web_search(&mut cfg, WebSearch::Separate { connection: gpt }).unwrap();
        let WebSearch::Separate { connection } = &cfg.web_search else { panic!("not separate") };
        assert_eq!(connection.thinking, None);
        assert_eq!(cfg.provider_keys.get("openai").map(String::as_str), Some("sk-search"));

        let err = set_web_search(&mut cfg, WebSearch::Separate { connection: sample(Provider::Lmstudio, "m") });
        assert!(err.is_err());
        assert!(matches!(cfg.web_search, WebSearch::Separate { .. }), "unchanged on error");
    }

    #[test]
    fn set_default_languages_updates_config() {
        let mut cfg = default_config();
        set_default_languages(&mut cfg, "ja", "en");
        assert_eq!(cfg.default_source, "ja");
        assert_eq!(cfg.default_target, "en");
    }

    // ---- v2 parsing ----

    fn full_v2() -> AppConfig {
        let mut translation = sample(Provider::Anthropic, "claude-sonnet-5-5");
        translation.thinking = Some(High);
        translation.max_tokens = Some(20000);
        translation.connect_timeout = Some(15);
        let mut provider_keys = BTreeMap::new();
        provider_keys.insert("anthropic".to_string(), "k".to_string());
        AppConfig {
            version: CONFIG_VERSION,
            default_source: "ko".into(),
            default_target: "en".into(),
            default_workdir: Some("/work".into()),
            translation: Some(translation),
            web_search: WebSearch::Separate { connection: sample(Provider::Google, "gemini-2.5-flash") },
            provider_keys,
        }
    }

    #[test]
    fn v2_round_trips_unchanged() {
        let cfg = full_v2();
        assert_eq!(config_from_stored(serde_json::to_value(&cfg).unwrap()), cfg);
        let mut translation_mode = full_v2();
        translation_mode.web_search = WebSearch::Translation;
        assert_eq!(config_from_stored(serde_json::to_value(&translation_mode).unwrap()), translation_mode);
        let fresh = default_config();
        assert_eq!(config_from_stored(serde_json::to_value(&fresh).unwrap()), fresh);
    }

    #[test]
    fn v2_invalid_fields_fall_back_alone() {
        let mut stored = serde_json::to_value(full_v2()).unwrap();
        stored["default_source"] = json!(5);
        stored["default_workdir"] = json!(["x"]);
        stored["translation"]["timeout"] = json!("slow");
        stored["translation"]["thinking"] = json!("maximum");
        stored["translation"]["unknown"] = json!(true);
        stored["provider_keys"]["openai"] = json!(42);
        let cfg = config_from_stored(stored);
        assert_eq!(cfg.default_source, "zh");
        assert_eq!(cfg.default_target, "en");
        assert_eq!(cfg.default_workdir, None);
        let t = cfg.translation.unwrap();
        assert_eq!(t.timeout, None);
        assert_eq!(t.thinking, None);
        assert_eq!(t.max_tokens, Some(20000));
        assert_eq!(cfg.provider_keys.len(), 1, "the bad key entry dropped alone");
        assert!(matches!(cfg.web_search, WebSearch::Separate { .. }));
    }

    #[test]
    fn v2_unparseable_connections_and_roles_are_dropped() {
        let mut stored = serde_json::to_value(full_v2()).unwrap();
        stored["translation"]["provider"] = json!("carrier-pigeon");
        stored["web_search"]["connection"]["model"] = json!(null);
        let cfg = config_from_stored(stored.clone());
        assert_eq!(cfg.translation, None);
        assert_eq!(cfg.web_search, WebSearch::Off);

        stored["web_search"] = json!({ "mode": "sometimes" });
        assert_eq!(config_from_stored(stored.clone()).web_search, WebSearch::Off);
        stored["web_search"] = json!("translation");
        assert_eq!(config_from_stored(stored).web_search, WebSearch::Off);
    }

    #[test]
    fn v2_translation_mode_without_a_searching_translation_turns_off() {
        let mut cfg = full_v2();
        cfg.web_search = WebSearch::Translation;
        cfg.translation = Some(sample(Provider::Ollama, "qwen3"));
        assert_eq!(config_from_stored(serde_json::to_value(&cfg).unwrap()).web_search, WebSearch::Off);
        cfg.translation = None;
        assert_eq!(config_from_stored(serde_json::to_value(&cfg).unwrap()).web_search, WebSearch::Off);
    }

    #[test]
    fn newer_versions_load_best_effort_as_v2() {
        let mut stored = serde_json::to_value(full_v2()).unwrap();
        stored["version"] = json!(3);
        stored["something_new"] = json!({ "a": 1 });
        assert_eq!(config_from_stored(stored), full_v2());
    }

    #[test]
    fn non_object_config_loads_the_defaults() {
        assert_eq!(config_from_stored(json!(42)), default_config());
    }
}
