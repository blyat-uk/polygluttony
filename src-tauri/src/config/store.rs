//! Config persistence. Pure `AppConfig` helpers (unit-tested), the tolerant
//! loader that also migrates v1 configs in place, and a thin Tauri store
//! adapter (`load`/`save`).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use serde_json::{Map, Value};
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

use crate::config::providers::{self, capabilities, default_config, model_label, normalize_thinking};
use crate::config::{AppConfig, Connection, Driver, Provider, ThinkingLevel, WebSearch, CONFIG_VERSION};
use crate::error::{AppError, AppResult};

const STORE_FILE: &str = "config.json";
const STORE_KEY: &str = "app";
/// One-time copy of a v1 config, kept next to `config.json` (the store plugin
/// resolves both against the app data dir) before the v2 config replaces it.
const V1_BACKUP_FILE: &str = "config.v1.backup.json";

// ---- usability & role resolution -------------------------------------------

fn is_localhost(base_url: &str) -> bool {
    let url = base_url.to_ascii_lowercase();
    url.contains("localhost") || url.contains("127.0.0.1") || url.contains("[::1]")
}

/// A connection can make requests: it has a key, or it talks to a server that
/// needs none (a local provider, any localhost URL, or a Custom endpoint —
/// its key is optional, e.g. a keyless server elsewhere on the LAN; Connect's
/// test is what proves it works).
pub fn connection_is_usable(conn: &Connection) -> bool {
    !conn.api_key.trim().is_empty()
        || conn.provider.is_local()
        || conn.provider == Provider::Custom
        || is_localhost(&conn.base_url)
}

/// Fill unset tuning from the provider's defaults — what the UI shows for an
/// untouched Tuning section — so a run uses the values the user sees. The
/// response cap is then held to what the model can write (GPT-4o can't take
/// the 32000 cloud default); only the run's copy is clamped, never the config.
fn with_tuning_defaults(mut conn: Connection) -> Connection {
    let t = providers::provider_info(conn.provider).tuning;
    let max_tokens = conn.max_tokens.unwrap_or(t.max_tokens);
    let cap = providers::max_output_tokens(conn.provider, &conn.model);
    conn.max_tokens = Some(cap.map_or(max_tokens, |cap| max_tokens.min(cap)));
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
/// Returns the key this one replaced, if any.
fn remember_key(cfg: &mut AppConfig, conn: &Connection) -> Option<String> {
    if conn.provider.is_cloud() && !conn.api_key.is_empty() {
        cfg.provider_keys.insert(conn.provider.id().to_string(), conn.api_key.clone())
    } else {
        None
    }
}

/// After `conn` replaced the remembered key `replaced`, the other role on the
/// same provider follows when it was still on that old key: it was reusing the
/// account key, and a rotated (revoked) key would otherwise leave it failing.
/// A deliberately different key on the other role stays as it is.
fn follow_new_key(other: Option<&mut Connection>, conn: &Connection, replaced: Option<String>) {
    let Some(replaced) = replaced.filter(|old| *old != conn.api_key) else { return };
    if let Some(other) = other.filter(|o| o.provider == conn.provider && o.api_key == replaced) {
        other.api_key = conn.api_key.clone();
    }
}

fn separate(web_search: &WebSearch) -> Option<&Connection> {
    match web_search {
        WebSearch::Separate { connection } => Some(connection),
        WebSearch::Off | WebSearch::Translation => None,
    }
}

fn separate_mut(web_search: &mut WebSearch) -> Option<&mut Connection> {
    match web_search {
        WebSearch::Separate { connection } => Some(connection),
        WebSearch::Off | WebSearch::Translation => None,
    }
}

/// Removing a connection is how a user takes its key off this computer, so its
/// remembered key goes too — unless the other role still talks to the same
/// provider (same account key). Call it once `removed` is out of `cfg`.
/// (Switching a role to another provider keeps the old key remembered: that's
/// what refills the form when the user switches back.)
fn forget_key(cfg: &mut AppConfig, removed: &Connection) {
    let still_used = cfg
        .translation
        .iter()
        .chain(separate(&cfg.web_search))
        .any(|c| c.provider == removed.provider);
    if !still_used {
        cfg.provider_keys.remove(removed.provider.id());
    }
}

/// Save the translation connection (Connect, a model change, a Tuning save).
pub fn set_translation(cfg: &mut AppConfig, conn: Connection) {
    let mut conn = providers::canonicalize(conn);
    conn.thinking = normalize_thinking(conn.provider, &conn.model, conn.thinking);
    conn.web_search = false;
    let replaced = remember_key(cfg, &conn);
    follow_new_key(separate_mut(&mut cfg.web_search), &conn, replaced);
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

/// Remove the translation connection; a web search riding on it goes too, and
/// so does its remembered key (see `forget_key`).
pub fn clear_translation(cfg: &mut AppConfig) {
    let removed = cfg.translation.take();
    if cfg.web_search == WebSearch::Translation {
        cfg.web_search = WebSearch::Off;
    }
    if let Some(removed) = removed {
        forget_key(cfg, &removed);
    }
}

/// Set the web-search role. `Translation` needs a translation model that can
/// search; a separate model must be able to search itself. Leaving a separate
/// model for Off or Translation removes it, remembered key included.
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
            let replaced = remember_key(cfg, &conn);
            follow_new_key(cfg.translation.as_mut(), &conn, replaced);
            WebSearch::Separate { connection: conn }
        }
    };
    let previous = std::mem::replace(&mut cfg.web_search, web_search);
    if let WebSearch::Separate { connection: removed } = previous {
        // Replacing it with another separate model is a switch, not a removal.
        if separate(&cfg.web_search).is_none() {
            forget_key(cfg, &removed);
        }
    }
    Ok(())
}

/// Update the global default source/target languages. These seed a newly opened
/// folder that has no saved per-folder preferences (and persist across sessions).
pub fn set_default_languages(cfg: &mut AppConfig, source: &str, target: &str) {
    cfg.default_source = source.to_string();
    cfg.default_target = target.to_string();
}

// ---- loading: tolerant parsing + v1 migration -------------------------------

/// A stored config without `version` was written by v1.
pub fn is_v1(value: &Value) -> bool {
    value.as_object().is_some_and(|o| !o.contains_key("version"))
}

/// Parse the stored config, keeping whatever parses and defaulting the rest, so a
/// config written by another build (or edited by hand) never blocks startup.
/// v1 values are migrated to v2; fallbacks are logged.
pub fn config_from_stored(value: Value) -> AppConfig {
    match value.as_object() {
        Some(stored) if stored.contains_key("version") => config_from_v2(stored),
        Some(stored) => migrate_v1(stored),
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

/// A v1 connection entry — only the fields migration reads.
struct LegacyConnection {
    driver: Driver,
    base_url: String,
    api_key: String,
    model: String,
    max_tokens: Option<u32>,
    batch_dialogue_limit: Option<u32>,
    timeout: Option<u32>,
    connect_timeout: Option<u32>,
    concurrency: Option<u32>,
    thinking_enabled: bool,
    web_search: bool,
}

/// Parse a v1 entry tolerantly: `driver`, `base_url` and `model` must parse
/// (v1 required them too); every other field falls back on its own.
fn legacy_from_value(value: &Value, what: &str) -> Option<LegacyConnection> {
    let Some(map) = value.as_object() else {
        tracing::warn!("dropping {what} from the v1 config: not an object");
        return None;
    };
    let (Some(driver), Some(base_url), Some(model)) =
        (field(map, "driver", what), field(map, "base_url", what), field(map, "model", what))
    else {
        tracing::warn!("dropping {what} from the v1 config: no valid driver, base_url or model");
        return None;
    };
    Some(LegacyConnection {
        driver,
        base_url,
        api_key: field(map, "api_key", what).unwrap_or_default(),
        model,
        max_tokens: optional(map, "max_tokens", what),
        batch_dialogue_limit: optional(map, "batch_dialogue_limit", what),
        timeout: optional(map, "timeout", what),
        connect_timeout: optional(map, "connect_timeout", what),
        concurrency: optional(map, "concurrency", what),
        thinking_enabled: optional(map, "thinking_enabled", what).unwrap_or(false),
        web_search: optional(map, "web_search", what).unwrap_or(false),
    })
}

/// A v1 entry as a v2 connection. The provider is inferred from the URL, and
/// cloud providers move onto their own driver and endpoint (Google from its
/// OpenAI-compat endpoint to the native Gemini API). The old on/off thinking
/// switch becomes Medium, then whatever the model actually offers.
fn convert(legacy: &LegacyConnection) -> Connection {
    let mut conn = providers::canonicalize(Connection {
        provider: providers::infer_provider(&legacy.base_url),
        driver: legacy.driver,
        base_url: legacy.base_url.clone(),
        api_key: legacy.api_key.clone(),
        model: legacy.model.clone(),
        thinking: legacy.thinking_enabled.then_some(ThinkingLevel::Medium),
        max_tokens: legacy.max_tokens,
        batch_dialogue_limit: legacy.batch_dialogue_limit,
        timeout: legacy.timeout,
        connect_timeout: legacy.connect_timeout,
        concurrency: legacy.concurrency,
        web_search: false,
    });
    conn.thinking = normalize_thinking(conn.provider, &conn.model, conn.thinking);
    conn
}

/// v1 shipped a seeded Ollama entry that is usable by its URL alone, so local
/// usability says nothing about what the user set up. Only a remote entry with
/// its own key does.
fn set_up_by_user(conn: &Connection) -> bool {
    !conn.api_key.is_empty() && !conn.provider.is_local() && !is_localhost(&conn.base_url)
}

/// v1 → v2: pick the translation connection and the web-search role out of the
/// named list, and harvest cloud keys.
fn migrate_v1(stored: &Map<String, Value>) -> AppConfig {
    tracing::info!("migrating the v1 config to v{CONFIG_VERSION}");
    let legacy: BTreeMap<String, LegacyConnection> = match stored.get("connections") {
        Some(Value::Object(entries)) => entries
            .iter()
            .filter_map(|(name, v)| {
                legacy_from_value(v, &format!("connection `{name}`")).map(|l| (name.clone(), l))
            })
            .collect(),
        None | Some(Value::Null) => BTreeMap::new(),
        Some(_) => {
            tracing::warn!("v1 config field `connections` is invalid; migrating without it");
            BTreeMap::new()
        }
    };
    let converted: BTreeMap<&str, Connection> =
        legacy.iter().map(|(name, l)| (name.as_str(), convert(l))).collect();

    // v1 translated with the active connection; when that couldn't run, fall
    // back to the first entry (by name) the user evidently set up.
    let translation_name: Option<String> = field::<String>(stored, "active_connection", "config")
        .filter(|name| converted.get(name.as_str()).is_some_and(connection_is_usable))
        .or_else(|| {
            converted.iter().find(|(_, c)| set_up_by_user(c)).map(|(name, _)| name.to_string())
        });
    let translation = translation_name.as_deref().map(|name| converted[name].clone());

    // Only an openai-responses entry with its web-search switch on ever
    // searched in v1.
    let personalization = optional::<String>(stored, "personalization_model", "config");
    let web_search = match personalization.as_deref() {
        Some(p) if legacy.get(p).is_some_and(|l| l.driver == Driver::OpenaiResponses && l.web_search) => {
            if translation_name.as_deref() == Some(p) {
                WebSearch::Translation
            } else if connection_is_usable(&converted[p]) && can_search(&converted[p]) {
                let mut connection = converted[p].clone();
                connection.thinking = None;
                WebSearch::Separate { connection }
            } else {
                WebSearch::Off
            }
        }
        _ => WebSearch::Off,
    };

    // Every cloud key is worth remembering; the translation's wins a conflict.
    let mut provider_keys = BTreeMap::new();
    for conn in converted.values().chain(translation.iter()) {
        if conn.provider.is_cloud() && !conn.api_key.is_empty() {
            provider_keys.insert(conn.provider.id().to_string(), conn.api_key.clone());
        }
    }

    let defaults = default_config();
    let mut cfg = AppConfig {
        version: CONFIG_VERSION,
        default_source: field(stored, "default_source", "config").unwrap_or(defaults.default_source),
        default_target: field(stored, "default_target", "config").unwrap_or(defaults.default_target),
        default_workdir: optional(stored, "default_workdir", "config"),
        translation,
        web_search,
        provider_keys,
    };
    normalize_roles(&mut cfg);
    cfg
}

/// Write the v1 value in the store's own file shape (restoring is renaming the
/// file back to `config.json`) — unless a backup already exists: the first v1
/// config seen is the one worth keeping.
fn backup_v1_once(path: &Path, value: &Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(e) => return Err(e),
    };
    let mut doc = Map::new();
    doc.insert(STORE_KEY.to_string(), value.clone());
    let written = serde_json::to_vec_pretty(&doc)
        .map_err(std::io::Error::from)
        .and_then(|json| file.write_all(&json));
    if written.is_err() {
        // A truncated backup would block every later attempt.
        let _ = std::fs::remove_file(path);
    }
    written
}

/// Whether `save` may replace the stored value. `load` leaves a v1 config on
/// disk when its backup fails, so until a backup exists, saving (any connection
/// change, the default languages) would destroy the only copy of the old
/// settings. `backup` is where the backup belongs, or why that couldn't be
/// worked out.
fn check_v1_backed_up(stored: Option<&Value>, backup: Result<&Path, &str>) -> AppResult<()> {
    if !stored.is_some_and(is_v1) {
        return Ok(());
    }
    match backup {
        Ok(path) if path.exists() => Ok(()),
        Ok(path) => Err(AppError::Other(format!(
            "Your settings from the previous version couldn't be backed up to {}, so they \
             haven't been replaced. Check that the folder can be written to, then restart \
             Polygluttony.",
            path.display()
        ))),
        Err(e) => Err(AppError::Other(format!(
            "Your settings from the previous version couldn't be backed up ({e}), so they \
             haven't been replaced."
        ))),
    }
}

// ---- Tauri store adapter (thin; not unit-tested) ---------------------------

/// Load the config from the store. First run seeds and persists the defaults;
/// a v1 config is backed up once and replaced on disk by its v2 migration.
pub fn load<R: Runtime>(app: &AppHandle<R>) -> AppResult<AppConfig> {
    let store = app.store(STORE_FILE).map_err(|e| AppError::Other(e.to_string()))?;
    let Some(value) = store.get(STORE_KEY) else {
        let cfg = default_config();
        store.set(STORE_KEY, serde_json::to_value(&cfg)?);
        store.save().map_err(|e| AppError::Other(e.to_string()))?;
        return Ok(cfg);
    };
    if !is_v1(&value) {
        return Ok(config_from_stored(value));
    }
    let cfg = config_from_stored(value.clone());
    let backup = tauri_plugin_store::resolve_store_path(app, V1_BACKUP_FILE)
        .map_err(|e| e.to_string())
        .and_then(|path| backup_v1_once(&path, &value).map_err(|e| e.to_string()));
    match backup {
        Ok(()) => {
            store.set(STORE_KEY, serde_json::to_value(&cfg)?);
            store.save().map_err(|e| AppError::Other(e.to_string()))?;
        }
        // Without a backup the v1 value stays on disk, the migration reruns on
        // the next load, and `save` refuses to replace it meanwhile.
        Err(e) => tracing::warn!("couldn't back up the v1 config ({e}); not replacing it yet"),
    }
    Ok(cfg)
}

/// Persist the whole config — refused while a v1 config without a backup is
/// still on disk (see `check_v1_backed_up`).
pub fn save<R: Runtime>(app: &AppHandle<R>, cfg: &AppConfig) -> AppResult<()> {
    let store = app.store(STORE_FILE).map_err(|e| AppError::Other(e.to_string()))?;
    let backup = tauri_plugin_store::resolve_store_path(app, V1_BACKUP_FILE).map_err(|e| e.to_string());
    check_v1_backed_up(store.get(STORE_KEY).as_ref(), backup.as_deref().map_err(String::as_str))?;
    store.set(STORE_KEY, serde_json::to_value(cfg)?);
    store.save().map_err(|e| AppError::Other(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{sample, Provider};
    use serde_json::json;
    use ThinkingLevel::{High, Medium, Off};

    fn with_translation(conn: Connection) -> AppConfig {
        AppConfig { translation: Some(conn), ..default_config() }
    }

    // ---- usability & resolution ----

    #[test]
    fn connection_is_usable_rules() {
        let mut c = sample(Provider::Openai, "m");
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
        // Custom's key is optional: a keyless server on the LAN is usable.
        let mut custom = sample(Provider::Custom, "m");
        custom.api_key.clear();
        custom.base_url = "http://192.168.1.20:8000/v1".into();
        assert!(connection_is_usable(&custom));
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
    fn a_run_holds_max_tokens_to_the_models_output_cap() {
        // The 32000 cloud default is more than GPT-4o can write.
        let cfg = with_translation(sample(Provider::Openai, "gpt-4o"));
        assert_eq!(usable_translation(&cfg).unwrap().max_tokens, Some(16384));
        assert_eq!(cfg.translation.as_ref().unwrap().max_tokens, None, "the config is untouched");

        // An explicit value is clamped too, and one under the cap is kept.
        let mut claude = sample(Provider::Anthropic, "claude-3-5-sonnet-20241022");
        claude.max_tokens = Some(20000);
        assert_eq!(usable_translation(&with_translation(claude.clone())).unwrap().max_tokens, Some(8192));
        claude.max_tokens = Some(4000);
        assert_eq!(usable_translation(&with_translation(claude)).unwrap().max_tokens, Some(4000));

        // The search connection gets the same treatment.
        let cfg = AppConfig {
            web_search: WebSearch::Separate { connection: sample(Provider::Openai, "gpt-4o-mini") },
            ..default_config()
        };
        assert_eq!(resolve_web_search(&cfg).unwrap().max_tokens, Some(16384));

        // Models without a known cap keep the provider default.
        let cfg = with_translation(sample(Provider::Openai, "gpt-5.5"));
        assert_eq!(usable_translation(&cfg).unwrap().max_tokens, Some(32000));
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

        // Adaptive Claude always has a level selected: the model's API default.
        set_translation(&mut cfg, sample(Provider::Anthropic, "claude-opus-5-5"));
        assert_eq!(cfg.translation.as_ref().unwrap().thinking, Some(Medium));
        set_translation(&mut cfg, sample(Provider::Anthropic, "claude-sonnet-5-5"));
        assert_eq!(cfg.translation.as_ref().unwrap().thinking, Some(High));

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

    fn keyed(provider: Provider, model: &str, key: &str) -> Connection {
        Connection { api_key: key.into(), ..sample(provider, model) }
    }

    fn remembered(cfg: &AppConfig, provider: &str) -> Option<String> {
        cfg.provider_keys.get(provider).cloned()
    }

    fn separate_key(cfg: &AppConfig) -> &str {
        let WebSearch::Separate { connection } = &cfg.web_search else { panic!("not separate") };
        &connection.api_key
    }

    #[test]
    fn removing_the_translation_forgets_its_key() {
        let mut cfg = default_config();
        set_translation(&mut cfg, keyed(Provider::Openai, "gpt-5.5", "sk-oai"));
        // Switching providers keeps the old key: switching back refills it.
        set_translation(&mut cfg, keyed(Provider::Anthropic, "claude-sonnet-5-5", "sk-ant"));
        assert_eq!(remembered(&cfg, "openai").as_deref(), Some("sk-oai"));

        clear_translation(&mut cfg);
        assert_eq!(remembered(&cfg, "anthropic"), None, "removed with its connection");
        assert_eq!(remembered(&cfg, "openai").as_deref(), Some("sk-oai"), "merely switched away from");
    }

    #[test]
    fn removing_the_translation_keeps_a_key_the_search_model_still_uses() {
        let mut cfg = default_config();
        set_translation(&mut cfg, keyed(Provider::Anthropic, "claude-sonnet-5-5", "sk-ant"));
        let haiku = keyed(Provider::Anthropic, "claude-haiku-5-5", "sk-ant");
        set_web_search(&mut cfg, WebSearch::Separate { connection: haiku }).unwrap();
        clear_translation(&mut cfg);
        assert_eq!(remembered(&cfg, "anthropic").as_deref(), Some("sk-ant"));
    }

    #[test]
    fn removing_the_separate_search_model_forgets_its_key() {
        let mut cfg = with_translation(sample(Provider::Ollama, "qwen3"));
        let gpt = keyed(Provider::Openai, "gpt-5.5", "sk-search");
        set_web_search(&mut cfg, WebSearch::Separate { connection: gpt.clone() }).unwrap();
        set_web_search(&mut cfg, WebSearch::Off).unwrap();
        assert_eq!(remembered(&cfg, "openai"), None);

        // Moving on to the translation model keeps a key the translation uses.
        let mut cfg = default_config();
        set_translation(&mut cfg, keyed(Provider::Openai, "gpt-5.4-mini", "sk-search"));
        set_web_search(&mut cfg, WebSearch::Separate { connection: gpt.clone() }).unwrap();
        set_web_search(&mut cfg, WebSearch::Translation).unwrap();
        assert_eq!(remembered(&cfg, "openai").as_deref(), Some("sk-search"));

        // Replacing it with another separate model is a switch: the key stays.
        let mut cfg = with_translation(sample(Provider::Ollama, "qwen3"));
        set_web_search(&mut cfg, WebSearch::Separate { connection: gpt }).unwrap();
        let gemini = keyed(Provider::Google, "gemini-2.5-flash", "AIza-1");
        set_web_search(&mut cfg, WebSearch::Separate { connection: gemini }).unwrap();
        assert_eq!(remembered(&cfg, "openai").as_deref(), Some("sk-search"));
        assert_eq!(remembered(&cfg, "google").as_deref(), Some("AIza-1"));
    }

    #[test]
    fn a_new_key_follows_to_the_other_role_still_on_the_old_one() {
        let mut cfg = default_config();
        set_translation(&mut cfg, keyed(Provider::Anthropic, "claude-sonnet-5-5", "sk-old"));
        let haiku = keyed(Provider::Anthropic, "claude-haiku-5-5", "sk-old");
        set_web_search(&mut cfg, WebSearch::Separate { connection: haiku.clone() }).unwrap();

        // A rotated key on the translation carries over to the search model...
        set_translation(&mut cfg, keyed(Provider::Anthropic, "claude-sonnet-5-5", "sk-new"));
        assert_eq!(separate_key(&cfg), "sk-new");
        assert_eq!(remembered(&cfg, "anthropic").as_deref(), Some("sk-new"));

        // ...and one saved on the search model carries over to the translation.
        let rotated = keyed(Provider::Anthropic, "claude-haiku-5-5", "sk-newer");
        set_web_search(&mut cfg, WebSearch::Separate { connection: rotated }).unwrap();
        assert_eq!(cfg.translation.as_ref().unwrap().api_key, "sk-newer");
    }

    #[test]
    fn a_different_key_on_the_other_role_stays() {
        // As a v1 migration leaves it: two keys for one provider, the
        // translation's remembered.
        let mut cfg = AppConfig {
            web_search: WebSearch::Separate {
                connection: keyed(Provider::Openai, "gpt-5.4-mini", "sk-own"),
            },
            ..with_translation(keyed(Provider::Openai, "gpt-5.5", "sk-team"))
        };
        cfg.provider_keys.insert("openai".into(), "sk-team".into());
        set_translation(&mut cfg, keyed(Provider::Openai, "gpt-5.5", "sk-team-2"));
        assert_eq!(separate_key(&cfg), "sk-own", "it wasn't on the replaced key");

        // Another provider's connection never follows.
        let mut cfg = default_config();
        set_translation(&mut cfg, keyed(Provider::Anthropic, "claude-sonnet-5-5", "sk-1"));
        let gpt = keyed(Provider::Openai, "gpt-5.5", "sk-1");
        set_web_search(&mut cfg, WebSearch::Separate { connection: gpt }).unwrap();
        set_translation(&mut cfg, keyed(Provider::Anthropic, "claude-sonnet-5-5", "sk-2"));
        assert_eq!(separate_key(&cfg), "sk-1");
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

    // ---- v1 migration ----

    fn v1_entry(driver: &str, base_url: &str, api_key: &str, model: &str) -> Value {
        json!({
            "driver": driver, "base_url": base_url, "api_key": api_key, "model": model,
            "max_tokens": 16000, "batch_dialogue_limit": 100, "timeout": 120,
            "connect_timeout": 10, "concurrency": 5, "thinking_enabled": null,
            "thinking_budget": null, "thinking_glossary_budget": null,
            "thinking_glossary_norm_budget": null, "web_search": null
        })
    }

    /// Exactly what v1's `default_config()` seeded on first run.
    fn v1_seeded() -> Value {
        json!({
            "default_source": "zh", "default_target": "en", "active_connection": "anthropic",
            "personalization_model": "openai", "default_workdir": null,
            "connections": {
                "anthropic": v1_entry("anthropic", "https://api.anthropic.com", "", "claude-opus-4-5"),
                "google": v1_entry("openai", "https://generativelanguage.googleapis.com/v1beta/openai/", "", "gemini-2.5-pro"),
                "openai": v1_entry("openai-responses", "https://api.openai.com/v1", "", "gpt-5.2"),
                "ollama": v1_entry("openai", "http://localhost:11434/v1", "ollama", "llama3.1")
            }
        })
    }

    #[test]
    fn is_v1_means_no_version_field() {
        assert!(is_v1(&v1_seeded()));
        assert!(!is_v1(&serde_json::to_value(default_config()).unwrap()));
        assert!(!is_v1(&json!(42)));
    }

    #[test]
    fn seeded_v1_lands_on_the_provider_picker() {
        let cfg = config_from_stored(v1_seeded());
        assert_eq!(cfg.version, CONFIG_VERSION);
        // The seeded Ollama entry is usable by URL, but the user never set it up.
        assert_eq!(cfg.translation, None);
        assert_eq!(cfg.web_search, WebSearch::Off);
        assert!(cfg.provider_keys.is_empty());
        assert_eq!((cfg.default_source.as_str(), cfg.default_target.as_str()), ("zh", "en"));
    }

    #[test]
    fn active_anthropic_with_separate_openai_search_migrates_to_both_roles() {
        let mut v1 = v1_seeded();
        v1["connections"]["anthropic"]["api_key"] = json!("sk-ant-1");
        v1["connections"]["openai"]["api_key"] = json!("sk-oai-1");
        v1["connections"]["openai"]["web_search"] = json!(true);
        v1["connections"]["openai"]["thinking_enabled"] = json!(true);
        let cfg = config_from_stored(v1);

        let t = cfg.translation.as_ref().unwrap();
        assert_eq!(t.provider, Provider::Anthropic);
        assert_eq!(t.driver, Driver::Anthropic);
        assert_eq!(t.model, "claude-opus-4-5");
        assert_eq!(t.api_key, "sk-ant-1");
        assert_eq!(t.max_tokens, Some(16000), "tuning copied");
        let WebSearch::Separate { connection } = &cfg.web_search else { panic!("{:?}", cfg.web_search) };
        assert_eq!(connection.provider, Provider::Openai);
        assert_eq!(connection.driver, Driver::OpenaiResponses);
        assert_eq!(connection.api_key, "sk-oai-1");
        assert_eq!(connection.thinking, None, "a separate search model has no thinking");
    }

    #[test]
    fn personalization_on_the_translation_entry_becomes_translation() {
        let mut v1 = v1_seeded();
        v1["active_connection"] = json!("openai");
        v1["connections"]["openai"]["api_key"] = json!("sk-oai-1");
        v1["connections"]["openai"]["web_search"] = json!(true);
        let cfg = config_from_stored(v1);
        assert_eq!(cfg.translation.as_ref().unwrap().provider, Provider::Openai);
        assert_eq!(cfg.web_search, WebSearch::Translation);
    }

    #[test]
    fn personalization_without_web_search_migrates_to_off() {
        let mut v1 = v1_seeded();
        v1["connections"]["anthropic"]["api_key"] = json!("sk-ant-1");
        v1["connections"]["openai"]["api_key"] = json!("sk-oai-1"); // web_search null
        assert_eq!(config_from_stored(v1.clone()).web_search, WebSearch::Off);
        // Wrong driver: v1 never searched on chat completions.
        v1["connections"]["openai"]["web_search"] = json!(true);
        v1["connections"]["openai"]["driver"] = json!("openai");
        assert_eq!(config_from_stored(v1.clone()).web_search, WebSearch::Off);
        // Unusable (no key).
        v1["connections"]["openai"]["driver"] = json!("openai-responses");
        v1["connections"]["openai"]["api_key"] = json!("");
        assert_eq!(config_from_stored(v1).web_search, WebSearch::Off);
    }

    #[test]
    fn google_moves_to_the_native_gemini_api() {
        let mut v1 = v1_seeded();
        v1["active_connection"] = json!("google");
        v1["connections"]["google"]["api_key"] = json!("AIza-1");
        let cfg = config_from_stored(v1);
        let t = cfg.translation.as_ref().unwrap();
        assert_eq!(t.provider, Provider::Google);
        assert_eq!(t.driver, Driver::Gemini);
        assert_eq!(t.base_url, "https://generativelanguage.googleapis.com/v1beta");
        assert_eq!(t.model, "gemini-2.5-pro");
    }

    #[test]
    fn local_and_custom_entries_keep_their_driver_and_url() {
        let mut v1 = v1_seeded();
        v1["connections"]["mine"] = v1_entry("anthropic", "https://api.z.ai/api/anthropic", "z-key", "glm-4.6");
        v1["active_connection"] = json!("mine");
        let t = config_from_stored(v1.clone()).translation.unwrap();
        assert_eq!((t.provider, t.driver), (Provider::Custom, Driver::Anthropic));
        assert_eq!(t.base_url, "https://api.z.ai/api/anthropic");

        v1["active_connection"] = json!("ollama");
        let t = config_from_stored(v1).translation.unwrap();
        assert_eq!((t.provider, t.driver), (Provider::Ollama, Driver::Openai));
        assert_eq!(t.base_url, "http://localhost:11434/v1");
    }

    #[test]
    fn unusable_active_falls_back_to_the_first_keyed_entry_by_name() {
        let mut v1 = v1_seeded(); // active = anthropic, no key
        v1["connections"]["zeta"] = v1_entry("openai", "https://openrouter.ai/api/v1", "or-key", "x/y");
        v1["connections"]["google"]["api_key"] = json!("AIza-1");
        let t = config_from_stored(v1).translation.unwrap();
        assert_eq!(t.provider, Provider::Google, "google < zeta, and ollama doesn't count");
    }

    #[test]
    fn keys_are_harvested_and_the_translations_key_wins() {
        let mut v1 = v1_seeded();
        v1["connections"]["anthropic"]["api_key"] = json!("sk-ant-active");
        v1["connections"]["claude-alt"] = v1_entry("anthropic", "https://api.anthropic.com", "sk-ant-other", "claude-haiku-4-5");
        v1["connections"]["google"]["api_key"] = json!("AIza-1");
        v1["connections"]["mine"] = v1_entry("openai", "https://openrouter.ai/api/v1", "or-key", "x/y");
        let cfg = config_from_stored(v1);
        let keys: Vec<(&str, &str)> =
            cfg.provider_keys.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        // "claude-alt" sorts after "anthropic" but the translation still wins;
        // custom and local keys aren't remembered.
        assert_eq!(keys, [("anthropic", "sk-ant-active"), ("google", "AIza-1")]);
    }

    #[test]
    fn thinking_enabled_becomes_medium_then_what_the_model_offers() {
        let migrate = |driver: &str, base: &str, model: &str, enabled: bool| {
            let mut entry = v1_entry(driver, base, "key", model);
            entry["thinking_enabled"] = json!(enabled);
            entry["thinking_budget"] = json!(6000);
            let v1 = json!({ "active_connection": "c", "connections": { "c": entry } });
            config_from_stored(v1).translation.unwrap().thinking
        };
        let ant = "https://api.anthropic.com";
        assert_eq!(migrate("anthropic", ant, "claude-haiku-4-5", true), Some(Medium));
        assert_eq!(migrate("anthropic", ant, "claude-haiku-4-5", false), Some(Off));
        assert_eq!(migrate("anthropic", ant, "claude-sonnet-4-6", true), Some(Medium));
        // Adaptive Claude has no Off; it lands on its default.
        assert_eq!(migrate("anthropic", ant, "claude-sonnet-4-6", false), Some(High));
        // Gemini 3 has no Medium: the model's own default.
        let google = "https://generativelanguage.googleapis.com/v1beta/openai/";
        assert_eq!(migrate("openai", google, "gemini-3-flash-preview", true), None);
        assert_eq!(migrate("openai", google, "gemini-2.5-pro", true), Some(Medium));
        // Stale flags on models without a Thinking control vanish.
        assert_eq!(migrate("openai", "http://localhost:11434/v1", "qwen3", true), None);
    }

    #[test]
    fn invalid_v1_entries_are_dropped_and_bad_fields_default_alone() {
        let mut ok = v1_entry("anthropic", "https://api.anthropic.com", "sk-ant-1", "claude-opus-4-5");
        ok["max_tokens"] = json!("lots");
        let v1 = json!({
            "active_connection": "bad",
            "connections": {
                "bad": { "driver": "carrier-pigeon", "base_url": "u", "model": "m", "api_key": "k" },
                "no-model": { "driver": "openai", "base_url": "https://x", "api_key": "k" },
                "not-an-object": 7,
                "ok": ok
            }
        });
        let t = config_from_stored(v1).translation.unwrap();
        assert_eq!(t.api_key, "sk-ant-1");
        assert_eq!(t.max_tokens, None, "only the bad field fell back");
        assert_eq!(t.timeout, Some(120));
    }

    #[test]
    fn v1_scalars_are_copied_and_old_keys_dropped() {
        let mut v1 = v1_seeded();
        v1["default_source"] = json!("ja");
        v1["default_target"] = json!("ru");
        v1["default_workdir"] = json!("/work");
        v1["connections"] = json!("oops");
        let cfg = config_from_stored(v1);
        assert_eq!(cfg.default_source, "ja");
        assert_eq!(cfg.default_target, "ru");
        assert_eq!(cfg.default_workdir.as_deref(), Some("/work"));
        assert_eq!(cfg.translation, None);
        let stored = serde_json::to_value(&cfg).unwrap();
        let keys: Vec<&str> = stored.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            ["default_source", "default_target", "default_workdir", "provider_keys", "translation",
             "version", "web_search"]
        );
    }

    #[test]
    fn partial_v1_config_fills_missing_fields_from_defaults() {
        let cfg = config_from_stored(json!({ "default_target": "ru" }));
        assert_eq!(cfg.default_source, "zh");
        assert_eq!(cfg.default_target, "ru");
        assert_eq!(cfg.translation, None);
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

    // ---- v1 backup ----

    #[test]
    fn v1_backup_is_written_once_in_the_store_file_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(V1_BACKUP_FILE);
        backup_v1_once(&path, &v1_seeded()).unwrap();
        let written: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(written, json!({ "app": v1_seeded() }));

        // A second v1 value never replaces the first backup.
        backup_v1_once(&path, &json!({ "default_source": "ja" })).unwrap();
        let again: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(again, written);
    }

    #[test]
    fn saving_over_a_v1_config_needs_its_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(V1_BACKUP_FILE);
        let v1 = v1_seeded();

        // The backup failed: the v1 value is still the only copy.
        let err = check_v1_backed_up(Some(&v1), Ok(&path)).unwrap_err().to_string();
        assert!(err.contains("couldn't be backed up"), "{err}");
        assert!(err.contains(&path.display().to_string()), "names where: {err}");
        let err = check_v1_backed_up(Some(&v1), Err("no app data dir")).unwrap_err().to_string();
        assert!(err.contains("no app data dir"), "{err}");

        // Once the backup exists, the v1 value may go.
        backup_v1_once(&path, &v1).unwrap();
        assert!(check_v1_backed_up(Some(&v1), Ok(&path)).is_ok());

        // A v2 value, or nothing stored yet, never needs one.
        let missing = dir.path().join("nope.json");
        let v2 = serde_json::to_value(default_config()).unwrap();
        assert!(check_v1_backed_up(Some(&v2), Ok(&missing)).is_ok());
        assert!(check_v1_backed_up(None, Err("no app data dir")).is_ok());
    }
}
