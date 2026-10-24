//! Config persistence. Pure `AppConfig` helpers (unit-tested): step
//! resolution, the invariant-keeping `normalize`, the mutations behind the
//! Connections commands, and the tolerant loader that also migrates v1 (and
//! pre-release v2) configs in place; plus a thin Tauri store adapter
//! (`load`/`save`).

use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::Path;

use serde_json::{Map, Value};
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

use crate::config::providers::{self, capabilities, default_config, normalize_thinking};
use crate::config::{
    AppConfig, Connection, Driver, Model, Provider, Step, StepSettings, Steps, ThinkingLevel,
    CONFIG_VERSION,
};
use crate::error::{AppError, AppResult};

const STORE_FILE: &str = "config.json";
const STORE_KEY: &str = "app";
/// One-time copy of a v1 config, kept next to `config.json` (the store plugin
/// resolves both against the app data dir) before the v2 config replaces it.
const V1_BACKUP_FILE: &str = "config.v1.backup.json";

// ---- usability & step resolution --------------------------------------------

fn is_localhost(base_url: &str) -> bool {
    let url = base_url.to_ascii_lowercase();
    url.contains("localhost") || url.contains("127.0.0.1") || url.contains("[::1]")
}

/// Requests can go out: there's a key, or the server needs none (a local
/// provider, any localhost URL, or a Custom endpoint — its key is optional,
/// e.g. a keyless server elsewhere on the LAN; Connect's test is what proves
/// it works).
fn reachable(provider: Provider, api_key: &str, base_url: &str) -> bool {
    !api_key.trim().is_empty()
        || provider.is_local()
        || provider == Provider::Custom
        || is_localhost(base_url)
}

/// A library model can make requests (see `reachable`). The page mirrors this
/// to mark models that still need a key.
pub fn model_is_usable(model: &Model) -> bool {
    reachable(model.provider, &model.api_key, &model.base_url)
}

/// The same rule for a v1 entry, before it becomes a library model.
fn connection_is_usable(conn: &Connection) -> bool {
    reachable(conn.provider, &conn.api_key, &conn.base_url)
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

/// `model` with `settings`, ready to run: tuning filled in and clamped (see
/// `with_tuning_defaults`), no web-search tool.
pub fn run_connection(model: &Model, settings: &StepSettings) -> Connection {
    with_tuning_defaults(Connection::from_step(model, settings))
}

fn find_model<'a>(models: &'a [Model], id: &str) -> Option<&'a Model> {
    models.iter().find(|m| m.id == id)
}

/// The connection `step` runs on: its library model merged with the step's
/// settings, ready to run. `NoActiveConnection` while nothing is set up; when
/// the step's model has no key, a message that names the step the way the
/// Models page does, so the user knows which row to fix.
pub fn step_connection(cfg: &AppConfig, step: Step) -> AppResult<Connection> {
    let steps = cfg.steps.as_ref().ok_or(AppError::NoActiveConnection)?;
    let Some(settings) = steps.get(step) else {
        return Err(AppError::Other("Look up names online is off. Turn it on on the Models page.".into()));
    };
    // `normalize` keeps every step on a library model; a dangling id can only
    // come from a config that skipped it, and runs nothing.
    let model = find_model(&cfg.models, &settings.model).ok_or(AppError::NoActiveConnection)?;
    if !model_is_usable(model) {
        return Err(AppError::StepNeedsKey { step: step.label(), model: model.label() });
    }
    Ok(run_connection(model, settings))
}

/// First-run check (O21): can a translation run? It takes both of its steps.
pub fn has_usable_connection(cfg: &AppConfig) -> bool {
    step_connection(cfg, Step::Translate).is_ok() && step_connection(cfg, Step::Verify).is_ok()
}

/// The connection that answers web-search calls, with its runtime
/// `web_search` flag on — iff the step is on, its model is usable and can
/// search. `None` makes the glossary skip the lookup.
pub fn resolve_web_search(cfg: &AppConfig) -> Option<Connection> {
    let mut conn = step_connection(cfg, Step::WebSearch).ok()?;
    if !capabilities(conn.provider, &conn.model).web_search {
        return None;
    }
    conn.web_search = true;
    Some(conn)
}

/// Label of the resolved web-search model ("Claude Sonnet 5.5"), if any.
pub fn web_search_label(cfg: &AppConfig) -> Option<String> {
    resolve_web_search(cfg).map(|c| providers::model_label(c.provider, &c.model))
}

// ---- invariants ---------------------------------------------------------------

/// Restore the config's invariants after a load and after every mutation, so
/// every reader (the page, the runs) can rely on them:
/// 1. model ids are non-empty and unique (repaired to a fresh `m{n}`);
/// 2. `steps` exists exactly when the library isn't empty — a library without
///    steps puts every step on its first model (web search only if that model
///    can search); an empty library clears `steps` and `per_step`;
/// 3. every step names a library model — an unknown id moves to the first
///    model (web search: the first model that can search, else off);
/// 4. web search runs on a model that can search, or is off;
/// 5. each step's Thinking is a level its model offers, else the model's default;
/// 6. `per_step` is on whenever the four text steps differ (the one-model view
///    could only show one of them);
/// 7. one key per cloud provider, shared by all its models.
pub fn normalize(cfg: &mut AppConfig) {
    repair_model_ids(&mut cfg.models);
    sync_cloud_keys(cfg);
    let AppConfig { models, steps, per_step, .. } = cfg;
    let Some(first) = models.first() else {
        *steps = None;
        *per_step = false;
        return;
    };
    let steps = steps.get_or_insert_with(|| {
        let text = StepSettings::new(first.id.clone());
        Steps::all(text.clone(), first.can_search().then_some(text))
    });
    for step in Step::TEXT {
        if let Some(settings) = steps.get_mut(step) {
            if find_model(models, &settings.model).is_none() {
                settings.model = first.id.clone();
            }
        }
    }
    if let Some(web_search) = steps.web_search.take() {
        // A known model that can't search turns web search off; it doesn't hop
        // to another model behind the user's back.
        let model = match find_model(models, &web_search.model) {
            Some(m) => Some(m).filter(|m| m.can_search()),
            None => models.iter().find(|m| m.can_search()),
        };
        steps.web_search = model.map(|m| StepSettings { model: m.id.clone(), ..web_search });
    }
    for settings in steps.iter_mut() {
        if let Some(m) = find_model(models, &settings.model) {
            settings.thinking = normalize_thinking(m.provider, &m.model, settings.thinking);
        }
    }
    if !*per_step && !steps.text_steps_shared() {
        *per_step = true;
    }
}

/// Invariant 1. The first holder of an id keeps it, so steps that named it
/// stay where they were.
fn repair_model_ids(models: &mut [Model]) {
    let mut seen = HashSet::new();
    let broken: Vec<usize> = models
        .iter()
        .enumerate()
        .filter(|(_, m)| m.id.trim().is_empty() || !seen.insert(m.id.clone()))
        .map(|(i, _)| i)
        .collect();
    for i in broken {
        let id = next_model_id(models);
        tracing::warn!("model `{}` has a missing or repeated id; it becomes `{id}`", models[i].model);
        models[i].id = id;
    }
}

/// `m{n}` with the smallest n ≥ 1 no model uses.
fn next_model_id(models: &[Model]) -> String {
    (1..)
        .map(|n| format!("m{n}"))
        .find(|id| find_model(models, id).is_none())
        .unwrap_or_default()
}

/// The smallest palette index no model uses, so a new model gets a colour of
/// its own while one is free.
fn next_color(models: &[Model]) -> u8 {
    (0..=u8::MAX).find(|c| models.iter().all(|m| m.color != *c)).unwrap_or(0)
}

/// Invariant 7. A cloud key belongs to the account, not to one model, so every
/// model of the provider uses the remembered key (a rotated key reaches all of
/// them). A key the map lacks (a hand-edited config) is adopted, not lost.
fn sync_cloud_keys(cfg: &mut AppConfig) {
    for model in cfg.models.iter_mut().filter(|m| m.provider.is_cloud()) {
        let provider = model.provider.id();
        match cfg.provider_keys.get(provider).filter(|k| !k.is_empty()) {
            Some(key) => model.api_key = key.clone(),
            None if !model.api_key.is_empty() => {
                cfg.provider_keys.insert(provider.to_string(), model.api_key.clone());
            }
            None => {}
        }
    }
}

// ---- mutations (each ends in `normalize`) -------------------------------------

/// The page acted on a model that has gone (e.g. removed in another window).
fn unknown_model() -> AppError {
    AppError::Other("That model isn't in your library any more.".into())
}

fn cant_search(model: &Model) -> AppError {
    AppError::Other(format!("{} can't search the web", model.label()))
}

fn library_model<'a>(models: &'a [Model], id: &str) -> AppResult<&'a Model> {
    find_model(models, id).ok_or_else(unknown_model)
}

/// The steps, or `NoActiveConnection` while nothing is set up.
fn steps_mut(cfg: &mut AppConfig) -> AppResult<&mut Steps> {
    cfg.steps.as_mut().ok_or(AppError::NoActiveConnection)
}

/// Removing a model is how a user takes its key off this computer, so the
/// provider's remembered key goes too — unless a model that stays still talks
/// to that provider (same account key). (Switching a model to another provider
/// keeps the old key: that's what refills the form when the user switches back.)
fn forget_unused_key(cfg: &mut AppConfig, provider: Provider) {
    if !cfg.models.iter().any(|m| m.provider == provider) {
        cfg.provider_keys.remove(provider.id());
    }
}

/// The same provider, model id and address: one library entry.
fn same_endpoint(a: &Model, b: &Model) -> bool {
    a.provider == b.provider && a.model == b.model && a.base_url == b.base_url
}

/// Add a model to the library or update one (Connect, Edit, an inline model
/// change). Returns its id.
///
/// An empty id is a new model — unless the library already has one with the
/// same provider, model id and address, which is then updated rather than
/// duplicated. A new model gets the first free `m{n}` id and the first colour
/// no model has; an updated one keeps both. A cloud key with a value becomes
/// the provider's key for every model of that provider; an empty one is filled
/// from the remembered key.
///
/// Editing a model into a twin of another library entry merges the two: the
/// twin goes, and its steps move to the edited model with their settings.
///
/// The first model does every step (web search too, when it can search).
/// `assign` also hands `step` to this model, with the model's own Thinking
/// default and the step's tuning kept (web search coming from off starts on
/// default tuning). Steps already on an updated model keep their settings;
/// `normalize` re-fits their Thinking and turns web search off if the model
/// can't search any more.
pub fn save_model(cfg: &mut AppConfig, model: Model, assign: Option<Step>) -> AppResult<String> {
    let mut model = providers::canonicalize(model);
    if assign == Some(Step::WebSearch) && !model.can_search() {
        return Err(cant_search(&model));
    }
    let existing = if model.id.is_empty() {
        cfg.models.iter().position(|m| same_endpoint(m, &model))
    } else {
        Some(cfg.models.iter().position(|m| m.id == model.id).ok_or_else(unknown_model)?)
    };
    let first = cfg.models.is_empty();
    match existing {
        Some(i) => {
            model.id = cfg.models[i].id.clone();
            model.color = cfg.models[i].color;
            cfg.models[i] = model.clone();
            // Edited into a twin of another library model: keep one entry, not
            // two. The twin's steps move to this one with their settings.
            if let Some(j) = cfg.models.iter().position(|m| m.id != model.id && same_endpoint(m, &model)) {
                let twin = cfg.models.remove(j).id;
                if let Some(steps) = cfg.steps.as_mut() {
                    for step in Step::ALL {
                        if let Some(s) = steps.get_mut(step).filter(|s| s.model == twin) {
                            s.model = model.id.clone();
                        }
                    }
                }
            }
        }
        None => {
            model.id = next_model_id(&cfg.models);
            model.color = next_color(&cfg.models);
            cfg.models.push(model.clone());
        }
    }
    if model.provider.is_cloud() && !model.api_key.is_empty() {
        cfg.provider_keys.insert(model.provider.id().to_string(), model.api_key.clone());
    }
    if first {
        let text = StepSettings::new(model.id.clone());
        cfg.steps = Some(Steps::all(text.clone(), model.can_search().then_some(text)));
        cfg.per_step = false;
    }
    if let (Some(step), Some(steps)) = (assign, cfg.steps.as_mut()) {
        let tuning = steps.get(step).cloned().unwrap_or_else(|| StepSettings::new(""));
        steps.set(step, Some(StepSettings { model: model.id.clone(), thinking: None, ..tuning }));
    }
    normalize(cfg);
    Ok(model.id)
}

/// Remove a library model. Its steps move to the first remaining model
/// (Thinking re-fitted, tuning kept); web search moves to the first remaining
/// model that can search, else turns off; removing the last model leaves
/// nothing set up. Its provider's remembered key goes unless another model
/// still uses that provider.
pub fn remove_model(cfg: &mut AppConfig, id: &str) -> AppResult<()> {
    let i = cfg.models.iter().position(|m| m.id == id).ok_or_else(unknown_model)?;
    let removed = cfg.models.remove(i);
    forget_unused_key(cfg, removed.provider);
    // Its steps now name no model: invariants 2–5 move them.
    normalize(cfg);
    Ok(())
}

/// One step's model and settings (the per-step view). Only web search can be
/// switched off (`None`), and its model must be able to search. Thinking is
/// re-fitted to the model.
pub fn set_step(cfg: &mut AppConfig, step: Step, settings: Option<StepSettings>) -> AppResult<()> {
    steps_mut(cfg)?;
    match &settings {
        None if step != Step::WebSearch => {
            return Err(AppError::Other(format!("{} can't be turned off", step.label())));
        }
        None => {}
        Some(s) => {
            let model = library_model(&cfg.models, &s.model)?;
            if step == Step::WebSearch && !model.can_search() {
                return Err(cant_search(model));
            }
        }
    }
    steps_mut(cfg)?.set(step, settings);
    normalize(cfg);
    Ok(())
}

/// The one-model view's Thinking and Tuning: `settings` for every text step,
/// and for web search too while it runs on the model the text steps were on —
/// there it rides on "your model"; a separate search model keeps its own
/// settings. Moving the shared settings to a model that can't search turns a
/// riding web search off.
pub fn set_shared_steps(cfg: &mut AppConfig, settings: StepSettings) -> AppResult<()> {
    steps_mut(cfg)?;
    let searches = library_model(&cfg.models, &settings.model)?.can_search();
    let steps = steps_mut(cfg)?;
    let shared_before = steps.translate.model.clone();
    for step in Step::TEXT {
        steps.set(step, Some(settings.clone()));
    }
    if steps.web_search.as_ref().is_some_and(|ws| ws.model == shared_before) {
        steps.web_search = searches.then_some(settings);
    }
    normalize(cfg);
    Ok(())
}

/// Switch the page to the per-step view. Nothing else changes.
pub fn show_every_step(cfg: &mut AppConfig) -> AppResult<()> {
    steps_mut(cfg)?;
    cfg.per_step = true;
    normalize(cfg);
    Ok(())
}

/// Back to one model ("Use one model for everything"). The text steps take the
/// settings `model_id` already has on the first of Translate, Extract,
/// Normalize, Verify that runs on it (else fresh defaults). Web search follows
/// when it was on and this model can search, stays on another search model it
/// was already on, and otherwise turns off. Models no step uses then leave the
/// library, taking their unused keys with them.
pub fn use_one_model(cfg: &mut AppConfig, model_id: &str) -> AppResult<()> {
    let steps = cfg.steps.as_ref().ok_or(AppError::NoActiveConnection)?;
    let searches = library_model(&cfg.models, model_id)?.can_search();
    let source = [Step::Translate, Step::Extract, Step::Normalize, Step::Verify]
        .into_iter()
        .filter_map(|s| steps.get(s))
        .find(|s| s.model == model_id)
        .cloned()
        .unwrap_or_else(|| StepSettings::new(model_id));
    let web_search = match &steps.web_search {
        Some(_) if searches => Some(source.clone()),
        // `normalize` guarantees that other model can search.
        Some(other) if other.model != model_id => Some(other.clone()),
        _ => None,
    };
    let steps = Steps::all(source, web_search);
    let (kept, dropped): (Vec<Model>, Vec<Model>) =
        std::mem::take(&mut cfg.models).into_iter().partition(|m| steps.uses(&m.id));
    cfg.models = kept;
    cfg.steps = Some(steps);
    cfg.per_step = false;
    for model in dropped {
        forget_unused_key(cfg, model.provider);
    }
    normalize(cfg);
    Ok(())
}

/// Update the global default source/target languages. These seed a newly opened
/// folder that has no saved per-folder preferences (and persist across sessions).
pub fn set_default_languages(cfg: &mut AppConfig, source: &str, target: &str) {
    cfg.default_source = source.to_string();
    cfg.default_target = target.to_string();
}

// ---- loading: tolerant parsing + migration ------------------------------------

/// A stored config without `version` was written by v1.
pub fn is_v1(value: &Value) -> bool {
    value.as_object().is_some_and(|o| !o.contains_key("version"))
}

/// Written by earlier builds of the per-step work: versioned, but still the
/// two roles (`translation` + `web_search`) instead of the model library.
fn is_pre_release_v2(stored: &Map<String, Value>) -> bool {
    !stored.contains_key("models")
        && (stored.contains_key("translation") || stored.contains_key("web_search"))
}

/// Parse the stored config, keeping whatever parses and defaulting the rest, so a
/// config written by another build (or edited by hand) never blocks startup.
/// v1 and pre-release v2 values are migrated; fallbacks are logged. The result
/// always holds the invariants (`normalize`).
pub fn config_from_stored(value: Value) -> AppConfig {
    let mut cfg = match value.as_object() {
        Some(stored) if !stored.contains_key("version") => migrate_v1(stored),
        Some(stored) if is_pre_release_v2(stored) => migrate_pre_release_v2(stored),
        Some(stored) => config_from_v2(stored),
        None => {
            tracing::warn!("config is not an object; using the defaults");
            default_config()
        }
    };
    normalize(&mut cfg);
    cfg
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

/// The fields every stored shape shares — languages, work folder, remembered
/// keys — around an empty library.
fn base_config(stored: &Map<String, Value>) -> AppConfig {
    if let Some(v) = field::<u64>(stored, "version", "config").filter(|v| *v > CONFIG_VERSION as u64) {
        tracing::warn!("config is v{v}, newer than this build (v{CONFIG_VERSION}); loading what it understands");
    }
    let defaults = default_config();
    AppConfig {
        default_source: field(stored, "default_source", "config").unwrap_or(defaults.default_source),
        default_target: field(stored, "default_target", "config").unwrap_or(defaults.default_target),
        default_workdir: optional(stored, "default_workdir", "config"),
        provider_keys: provider_keys_from(stored),
        ..defaults
    }
}

// -- v2 --

/// A stored library model, field by field: the identifying fields must parse
/// (as for a connection), the rest fall back. `None` (logged) drops the entry;
/// a missing id is left for `normalize` to repair.
fn model_from_value(value: &Value, what: &str) -> Option<Model> {
    let conn = connection_from_value(value, what)?;
    let map = value.as_object()?;
    let id: String = field(map, "id", what).unwrap_or_default();
    let color = field(map, "color", what).unwrap_or(0);
    Some(conn.split(&id, color).0)
}

/// The library, entry by entry (a bad entry drops only itself; the steps that
/// used it fall back through the invariants).
fn models_from(stored: &Map<String, Value>) -> Vec<Model> {
    match stored.get("models") {
        Some(Value::Array(entries)) => entries
            .iter()
            .enumerate()
            .filter_map(|(i, v)| model_from_value(v, &format!("model #{}", i + 1)))
            .collect(),
        None | Some(Value::Null) => Vec::new(),
        Some(_) => {
            tracing::warn!("config field `models` is invalid; starting with an empty library");
            Vec::new()
        }
    }
}

/// One stored step: it must name a model; every other field falls back alone.
fn step_settings_from_value(value: &Value, what: &str) -> Option<StepSettings> {
    let Some(map) = value.as_object() else {
        tracing::warn!("{what} is not an object; ignoring it");
        return None;
    };
    let Some(model) = field::<String>(map, "model", what) else {
        tracing::warn!("{what} names no model; ignoring it");
        return None;
    };
    Some(StepSettings {
        model,
        thinking: optional(map, "thinking", what),
        max_tokens: optional(map, "max_tokens", what),
        batch_dialogue_limit: optional(map, "batch_dialogue_limit", what),
        timeout: optional(map, "timeout", what),
        concurrency: optional(map, "concurrency", what),
    })
}

/// The stored steps. One that doesn't parse falls back through the
/// invariants: a text step to the first model on default settings, web search
/// to off (its default, as for any invalid field). `None` (no steps at all)
/// puts every step on the first model.
fn steps_from(stored: &Map<String, Value>) -> Option<Steps> {
    let map = match stored.get("steps") {
        Some(Value::Object(map)) => map,
        None | Some(Value::Null) => return None,
        Some(_) => {
            tracing::warn!("config field `steps` is invalid; every step goes to the first model");
            return None;
        }
    };
    let step = |key: &str| {
        map.get(key)
            .filter(|v| !v.is_null())
            .and_then(|v| step_settings_from_value(v, &format!("step `{key}`")))
    };
    // An empty id names no model, so `normalize` moves the step to the first one.
    let text = |key: &str| {
        step(key).unwrap_or_else(|| {
            tracing::warn!("step `{key}` is missing or invalid; it goes to the first model");
            StepSettings::new("")
        })
    };
    Some(Steps {
        extract: text("extract"),
        normalize: text("normalize"),
        web_search: step("web_search"),
        translate: text("translate"),
        verify: text("verify"),
    })
}

fn config_from_v2(stored: &Map<String, Value>) -> AppConfig {
    AppConfig {
        models: models_from(stored),
        steps: steps_from(stored),
        per_step: field(stored, "per_step", "config").unwrap_or(false),
        ..base_config(stored)
    }
}

// -- role-based configs (v1, pre-release v2) --

/// Who answered the glossary's web-search call before the model library.
/// Migration-only.
enum LegacyWebSearch {
    Off,
    /// The translation connection also searched.
    Translation,
    /// A dedicated search-capable connection.
    Separate(Connection),
}

/// The library and steps a role-based config stood for — one rule for both
/// the v1 and the pre-release v2 migration. The translation connection becomes
/// model `m1`, and its Thinking + Tuning the shared settings of every text
/// step. Web search becomes a step on `m1` (with the shared settings, or the
/// separate connection's own when that was the same model) or on a second
/// model `m2`. No translation leaves nothing set up — a separate search model
/// included, since it never worked on its own.
fn library_from_roles(
    translation: Option<Connection>,
    web_search: LegacyWebSearch,
) -> (Vec<Model>, Option<Steps>) {
    let Some(translation) = translation else { return (Vec::new(), None) };
    let (first, shared) = translation.split("m1", 0);
    let mut models = vec![first];
    let web_search = match web_search {
        LegacyWebSearch::Off => None,
        // `normalize` turns it off when the translation model can't search.
        LegacyWebSearch::Translation => Some(shared.clone()),
        LegacyWebSearch::Separate(conn) => {
            let first = &models[0];
            if conn.provider == first.provider && conn.model == first.model && conn.base_url == first.base_url {
                Some(conn.split("m1", 0).1)
            } else {
                let (second, settings) = conn.split("m2", 1);
                models.push(second);
                Some(settings)
            }
        }
    };
    (models, Some(Steps::all(shared, web_search)))
}

/// A stored role connection, field by field: the identifying fields must
/// parse, the rest fall back to unset. `None` (logged) when it can't be a
/// connection. Also reads the identifying fields of a library model.
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

fn web_search_from_value(value: &Value) -> LegacyWebSearch {
    match value.get("mode").and_then(Value::as_str) {
        Some("off") => LegacyWebSearch::Off,
        Some("translation") => LegacyWebSearch::Translation,
        Some("separate") => value
            .get("connection")
            .and_then(|c| connection_from_value(c, "web search connection"))
            .map_or(LegacyWebSearch::Off, LegacyWebSearch::Separate),
        _ => {
            tracing::warn!("config field `web_search` is invalid; using the default");
            LegacyWebSearch::Off
        }
    }
}

/// Earlier builds of the per-step work stored the two roles under v2.
fn migrate_pre_release_v2(stored: &Map<String, Value>) -> AppConfig {
    tracing::info!("converting a pre-release v2 config to the model library");
    let translation = stored
        .get("translation")
        .filter(|v| !v.is_null())
        .and_then(|v| connection_from_value(v, "translation connection"));
    let web_search = stored.get("web_search").map_or(LegacyWebSearch::Off, web_search_from_value);
    let (models, steps) = library_from_roles(translation, web_search);
    AppConfig { models, steps, ..base_config(stored) }
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

/// A v1 entry as a connection. The provider is inferred from the URL, and
/// cloud providers move onto their own driver and endpoint (Google from its
/// OpenAI-compat endpoint to the native Gemini API). The old on/off thinking
/// switch becomes Medium, then whatever the model actually offers.
fn convert(legacy: &LegacyConnection) -> Connection {
    let model = providers::canonicalize(Model {
        id: String::new(),
        provider: providers::infer_provider(&legacy.base_url),
        driver: legacy.driver,
        base_url: legacy.base_url.clone(),
        api_key: legacy.api_key.clone(),
        model: legacy.model.clone(),
        color: 0,
        connect_timeout: legacy.connect_timeout,
    });
    let thinking = legacy.thinking_enabled.then_some(ThinkingLevel::Medium);
    let settings = StepSettings {
        model: String::new(),
        thinking: normalize_thinking(model.provider, &model.model, thinking),
        max_tokens: legacy.max_tokens,
        batch_dialogue_limit: legacy.batch_dialogue_limit,
        timeout: legacy.timeout,
        concurrency: legacy.concurrency,
    };
    Connection::from_step(&model, &settings)
}

/// v1 shipped a seeded Ollama entry that is usable by its URL alone, so local
/// usability says nothing about what the user set up. Only a remote entry with
/// its own key does.
fn set_up_by_user(conn: &Connection) -> bool {
    !conn.api_key.is_empty() && !conn.provider.is_local() && !is_localhost(&conn.base_url)
}

/// v1 → v2: pick the translation connection and the web-search role out of the
/// named list, harvest cloud keys, and turn the roles into a library.
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
    let can_search = |c: &Connection| capabilities(c.provider, &c.model).web_search;
    let personalization = optional::<String>(stored, "personalization_model", "config");
    let web_search = match personalization.as_deref() {
        Some(p) if legacy.get(p).is_some_and(|l| l.driver == Driver::OpenaiResponses && l.web_search) => {
            if translation_name.as_deref() == Some(p) {
                LegacyWebSearch::Translation
            } else if connection_is_usable(&converted[p]) && can_search(&converted[p]) {
                // v1's separate search model had no Thinking control.
                let mut connection = converted[p].clone();
                connection.thinking = None;
                LegacyWebSearch::Separate(connection)
            } else {
                LegacyWebSearch::Off
            }
        }
        _ => LegacyWebSearch::Off,
    };

    // Every cloud key is worth remembering; the translation's wins a conflict.
    let mut provider_keys = BTreeMap::new();
    for conn in converted.values().chain(translation.iter()) {
        if conn.provider.is_cloud() && !conn.api_key.is_empty() {
            provider_keys.insert(conn.provider.id().to_string(), conn.api_key.clone());
        }
    }

    let (models, steps) = library_from_roles(translation, web_search);
    AppConfig { models, steps, provider_keys, ..base_config(stored) }
}

// ---- v1 backup + save guard -----------------------------------------------------

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
        // A pre-release v2 config (earlier builds of this version) is rewritten
        // in the library shape straight away, like a v1 one: no backup, since
        // only those builds ever wrote it.
        let rewrite = value.as_object().is_some_and(is_pre_release_v2);
        let cfg = config_from_stored(value);
        if rewrite {
            store.set(STORE_KEY, serde_json::to_value(&cfg)?);
            store.save().map_err(|e| AppError::Other(e.to_string()))?;
        }
        return Ok(cfg);
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
    use crate::config::sample_model;
    use serde_json::json;
    use ThinkingLevel::{High, Low, Medium, Off};

    // ---- fixtures ----

    fn keyed(provider: Provider, model: &str, key: &str) -> Model {
        Model { api_key: key.into(), ..sample_model(provider, model) }
    }

    fn keyless(provider: Provider, model: &str) -> Model {
        keyed(provider, model, "")
    }

    /// A library built through `save_model`, in order (ids m1, m2, …).
    fn library(models: &[Model]) -> AppConfig {
        let mut cfg = default_config();
        for m in models {
            save_model(&mut cfg, m.clone(), None).unwrap();
        }
        cfg
    }

    fn on(id: &str) -> StepSettings {
        StepSettings::new(id)
    }

    fn step(cfg: &AppConfig, s: Step) -> &StepSettings {
        cfg.steps.as_ref().expect("steps").get(s).unwrap_or_else(|| panic!("{s:?} is off"))
    }

    fn web_search(cfg: &AppConfig) -> Option<&StepSettings> {
        cfg.steps.as_ref().expect("steps").web_search.as_ref()
    }

    fn model<'a>(cfg: &'a AppConfig, id: &str) -> &'a Model {
        find_model(&cfg.models, id).unwrap_or_else(|| panic!("no model {id}"))
    }

    fn ids(cfg: &AppConfig) -> Vec<&str> {
        cfg.models.iter().map(|m| m.id.as_str()).collect()
    }

    fn remembered(cfg: &AppConfig, provider: &str) -> Option<String> {
        cfg.provider_keys.get(provider).cloned()
    }

    /// Set a step directly, skipping the commands' checks and `normalize`.
    fn put(cfg: &mut AppConfig, s: Step, settings: StepSettings) {
        cfg.steps.as_mut().expect("steps").set(s, Some(settings));
    }

    /// The per-step view after some use: Sonnet translates (timeout 100) and
    /// verifies, Opus extracts (200) and normalizes (300), Gemini searches,
    /// and Qwen sits unused in the library.
    fn mixed() -> AppConfig {
        let mut cfg = library(&[
            keyed(Provider::Anthropic, "claude-sonnet-5-5", "sk-ant"),
            keyed(Provider::Anthropic, "claude-opus-5-5", "sk-ant"),
            keyed(Provider::Google, "gemini-3-flash-preview", "AIza"),
            sample_model(Provider::Ollama, "qwen3"),
        ]);
        set_step(&mut cfg, Step::Translate, Some(StepSettings { timeout: Some(100), ..on("m1") })).unwrap();
        set_step(&mut cfg, Step::Extract, Some(StepSettings { timeout: Some(200), ..on("m2") })).unwrap();
        set_step(&mut cfg, Step::Normalize, Some(StepSettings { timeout: Some(300), ..on("m2") })).unwrap();
        set_step(&mut cfg, Step::WebSearch, Some(on("m3"))).unwrap();
        cfg
    }

    // ---- usability & step resolution ----

    #[test]
    fn model_is_usable_rules() {
        let mut m = sample_model(Provider::Openai, "m");
        m.base_url = "https://api.example.com".into();
        m.api_key.clear();
        assert!(!model_is_usable(&m), "no key, remote URL");
        m.api_key = "sk-test".into();
        assert!(model_is_usable(&m), "key");
        m.api_key = "   ".into();
        assert!(!model_is_usable(&m), "whitespace isn't a key");
        m.api_key.clear();
        m.base_url = "http://localhost:8000/v1".into();
        assert!(model_is_usable(&m), "localhost");
        m.base_url = "http://127.0.0.1:8000/v1".into();
        assert!(model_is_usable(&m), "127.0.0.1");
        // Local providers need no key, even on another machine.
        let mut ollama = keyless(Provider::Ollama, "qwen3");
        ollama.base_url = "http://192.168.1.5:11434/v1".into();
        assert!(model_is_usable(&ollama));
        // Custom's key is optional: a keyless server on the LAN is usable.
        let mut custom = keyless(Provider::Custom, "m");
        custom.base_url = "http://192.168.1.20:8000/v1".into();
        assert!(model_is_usable(&custom));
    }

    #[test]
    fn nothing_set_up_is_no_active_connection() {
        let cfg = default_config();
        for s in Step::ALL {
            assert!(matches!(step_connection(&cfg, s), Err(AppError::NoActiveConnection)), "{s:?}");
        }
        assert!(!has_usable_connection(&cfg));
        assert_eq!(resolve_web_search(&cfg), None);
    }

    #[test]
    fn step_connection_merges_the_model_with_the_steps_settings() {
        let sonnet = Model { connect_timeout: Some(5), ..sample_model(Provider::Anthropic, "claude-sonnet-5-5") };
        let mut cfg = library(&[sonnet, sample_model(Provider::Anthropic, "claude-opus-5-5")]);
        put(&mut cfg, Step::Normalize, StepSettings { thinking: Some(High), timeout: Some(300), ..on("m2") });
        let norm = step_connection(&cfg, Step::Normalize).unwrap();
        assert_eq!((norm.model.as_str(), norm.thinking, norm.timeout), ("claude-opus-5-5", Some(High), Some(300)));
        assert_eq!(norm.connect_timeout, None, "each model's own");
        let translate = step_connection(&cfg, Step::Translate).unwrap();
        assert_eq!((translate.model.as_str(), translate.connect_timeout), ("claude-sonnet-5-5", Some(5)));
        assert_eq!(translate.thinking, Some(High));
        assert!(!translate.web_search && !norm.web_search, "only web search attaches the tool");
    }

    #[test]
    fn step_connection_fills_unset_tuning_from_the_provider() {
        let mut cfg = library(&[sample_model(Provider::Ollama, "qwen3")]);
        put(&mut cfg, Step::Translate, StepSettings { concurrency: Some(4), ..on("m1") });
        let run = step_connection(&cfg, Step::Translate).unwrap();
        assert_eq!(run.batch_dialogue_limit, Some(40));
        assert_eq!(run.timeout, Some(600));
        assert_eq!(run.max_tokens, Some(8192));
        assert_eq!(run.concurrency, Some(4), "explicit values stay");
        assert_eq!(step(&cfg, Step::Translate).timeout, None, "the config is untouched");
    }

    #[test]
    fn a_run_holds_max_tokens_to_the_models_output_cap() {
        // The 32000 cloud default is more than GPT-4o can write.
        let cfg = library(&[sample_model(Provider::Openai, "gpt-4o")]);
        assert_eq!(step_connection(&cfg, Step::Translate).unwrap().max_tokens, Some(16384));
        assert_eq!(step(&cfg, Step::Translate).max_tokens, None, "the config is untouched");

        // An explicit value is clamped too, and one under the cap is kept.
        let mut cfg = library(&[sample_model(Provider::Anthropic, "claude-3-5-sonnet-20241022")]);
        put(&mut cfg, Step::Verify, StepSettings { max_tokens: Some(20000), ..on("m1") });
        put(&mut cfg, Step::Extract, StepSettings { max_tokens: Some(4000), ..on("m1") });
        assert_eq!(step_connection(&cfg, Step::Verify).unwrap().max_tokens, Some(8192));
        assert_eq!(step_connection(&cfg, Step::Extract).unwrap().max_tokens, Some(4000));

        // The search step gets the same treatment.
        let cfg = library(&[sample_model(Provider::Openai, "gpt-4o-mini")]);
        assert_eq!(resolve_web_search(&cfg).unwrap().max_tokens, Some(16384));

        // Models without a known cap keep the provider default.
        let cfg = library(&[sample_model(Provider::Openai, "gpt-5.5")]);
        assert_eq!(step_connection(&cfg, Step::Translate).unwrap().max_tokens, Some(32000));
    }

    #[test]
    fn a_step_on_a_model_without_a_key_names_the_step_and_the_model() {
        let mut cfg = library(&[
            sample_model(Provider::Ollama, "qwen3"),
            keyless(Provider::Anthropic, "claude-opus-5-5"),
        ]);
        put(&mut cfg, Step::Normalize, on("m2"));
        let err = step_connection(&cfg, Step::Normalize).unwrap_err().to_string();
        assert_eq!(err, "Tidy up uses Claude Opus 5.5, which needs an API key. Add it on the Models page.");
        assert!(step_connection(&cfg, Step::Translate).is_ok(), "the other steps still run");
        // Every step by its name on the Models page.
        for s in Step::ALL {
            put(&mut cfg, s, on("m2"));
            let err = step_connection(&cfg, s).unwrap_err().to_string();
            assert_eq!(
                err,
                format!("{} uses Claude Opus 5.5, which needs an API key. Add it on the Models page.", s.label())
            );
        }
        assert!(step_connection(&cfg, Step::Extract).unwrap_err().to_string().starts_with("Find names uses"));
    }

    #[test]
    fn web_search_off_has_no_connection() {
        let cfg = library(&[sample_model(Provider::Ollama, "qwen3")]);
        assert!(step_connection(&cfg, Step::WebSearch).is_err());
        assert_eq!(resolve_web_search(&cfg), None);
        assert_eq!(web_search_label(&cfg), None);
    }

    #[test]
    fn a_translation_needs_both_translate_and_verify() {
        let mut cfg = library(&[
            sample_model(Provider::Anthropic, "claude-sonnet-5-5"),
            keyless(Provider::Openai, "gpt-5.5"),
        ]);
        assert!(has_usable_connection(&cfg));
        put(&mut cfg, Step::Verify, on("m2"));
        assert!(!has_usable_connection(&cfg), "Verify's model has no key");
        put(&mut cfg, Step::Verify, on("m1"));
        put(&mut cfg, Step::Extract, on("m2"));
        assert!(has_usable_connection(&cfg), "the glossary steps don't gate translating");
    }

    #[test]
    fn resolve_web_search_rules() {
        let mut cfg = library(&[sample_model(Provider::Anthropic, "claude-sonnet-5-5")]);
        let resolved = resolve_web_search(&cfg).unwrap();
        assert!(resolved.web_search);
        assert_eq!(resolved.model, "claude-sonnet-5-5");
        assert_eq!(resolved.timeout, Some(180), "tuning defaults filled");
        assert_eq!(web_search_label(&cfg).as_deref(), Some("Claude Sonnet 5.5"));

        // Without a key, nothing resolves.
        cfg.models[0].api_key.clear();
        cfg.provider_keys.clear();
        assert_eq!(resolve_web_search(&cfg), None);

        // A separate model resolves on its own, whatever the text steps run on.
        save_model(&mut cfg, sample_model(Provider::Openai, "gpt-5.5"), Some(Step::WebSearch)).unwrap();
        let resolved = resolve_web_search(&cfg).unwrap();
        assert_eq!((resolved.provider, resolved.web_search), (Provider::Openai, true));
        assert_eq!(web_search_label(&cfg).as_deref(), Some("GPT-5.5"));

        // A model that can't search never resolves (a config that skipped `normalize`).
        cfg.models.push(Model { id: "m3".into(), ..sample_model(Provider::Ollama, "qwen3") });
        put(&mut cfg, Step::WebSearch, on("m3"));
        assert_eq!(resolve_web_search(&cfg), None);
        assert_eq!(web_search_label(&cfg), None);
    }

    // ---- normalize: one test per invariant ----

    #[test]
    fn missing_and_repeated_ids_are_repaired() {
        let mut cfg = default_config();
        cfg.models = vec![
            Model { id: "".into(), color: 4, ..sample_model(Provider::Ollama, "a") },
            Model { id: "m1".into(), ..sample_model(Provider::Ollama, "b") },
            Model { id: "m1".into(), ..sample_model(Provider::Ollama, "c") },
            Model { id: "m3".into(), color: 4, ..sample_model(Provider::Ollama, "d") },
        ];
        cfg.steps = Some(Steps::all(on("m1"), None));
        normalize(&mut cfg);
        assert_eq!(ids(&cfg), ["m2", "m1", "m4", "m3"]);
        let colors: Vec<u8> = cfg.models.iter().map(|m| m.color).collect();
        assert_eq!(colors, [4, 0, 0, 4], "colours are kept, repeats and all");
        // The first holder kept its id, so the steps still find it.
        assert_eq!(model(&cfg, &step(&cfg, Step::Translate).model).model, "b");
    }

    #[test]
    fn a_library_without_steps_puts_every_step_on_its_first_model() {
        let mut cfg = default_config();
        cfg.models = vec![
            Model { id: "a".into(), ..sample_model(Provider::Anthropic, "claude-sonnet-5-5") },
            Model { id: "b".into(), ..sample_model(Provider::Ollama, "qwen3") },
        ];
        normalize(&mut cfg);
        for s in Step::ALL {
            assert_eq!(step(&cfg, s), &StepSettings { thinking: Some(High), ..on("a") }, "{s:?}");
        }
        assert!(!cfg.per_step);

        // Web search only when that first model can search.
        cfg.models.reverse();
        cfg.steps = None;
        normalize(&mut cfg);
        assert_eq!(step(&cfg, Step::Translate), &on("b"));
        assert_eq!(web_search(&cfg), None);
    }

    #[test]
    fn an_empty_library_has_no_steps() {
        let mut cfg = library(&[sample_model(Provider::Anthropic, "claude-sonnet-5-5")]);
        cfg.per_step = true;
        cfg.models.clear();
        normalize(&mut cfg);
        assert_eq!((cfg.steps.as_ref(), cfg.per_step), (None, false));
    }

    #[test]
    fn steps_on_unknown_models_move_to_the_first_one() {
        let mut cfg = library(&[
            sample_model(Provider::Ollama, "qwen3"),
            sample_model(Provider::Google, "gemini-2.5-flash"),
            sample_model(Provider::Anthropic, "claude-haiku-4-5"),
        ]);
        put(&mut cfg, Step::Verify, StepSettings { timeout: Some(42), ..on("gone") });
        put(&mut cfg, Step::WebSearch, StepSettings { timeout: Some(7), ..on("gone") });
        normalize(&mut cfg);
        assert_eq!(step(&cfg, Step::Verify), &StepSettings { timeout: Some(42), ..on("m1") }, "tuning kept");
        // Web search: the first model that can search...
        assert_eq!(web_search(&cfg), Some(&StepSettings { timeout: Some(7), ..on("m2") }));

        // ...or off when none can.
        let mut cfg = library(&[sample_model(Provider::Ollama, "qwen3")]);
        put(&mut cfg, Step::WebSearch, on("gone"));
        normalize(&mut cfg);
        assert_eq!(web_search(&cfg), None);
    }

    #[test]
    fn web_search_on_a_model_that_cant_search_turns_off() {
        let mut cfg = library(&[sample_model(Provider::Ollama, "qwen3"), sample_model(Provider::Openai, "gpt-5.5")]);
        put(&mut cfg, Step::WebSearch, on("m1"));
        normalize(&mut cfg);
        assert_eq!(web_search(&cfg), None, "off — not moved to the model that can search");
    }

    #[test]
    fn each_steps_thinking_fits_its_model() {
        let mut cfg = library(&[
            sample_model(Provider::Google, "gemini-3-flash-preview"),
            sample_model(Provider::Anthropic, "claude-opus-5-5"),
            sample_model(Provider::Anthropic, "claude-haiku-4-5"),
        ]);
        put(&mut cfg, Step::Extract, StepSettings { thinking: Some(Medium), ..on("m1") }); // not a Gemini 3 level
        put(&mut cfg, Step::Translate, on("m2"));
        put(&mut cfg, Step::Verify, StepSettings { thinking: Some(High), ..on("m3") });
        put(&mut cfg, Step::WebSearch, StepSettings { thinking: Some(Off), ..on("m2") }); // adaptive Claude has no Off
        normalize(&mut cfg);
        assert_eq!(step(&cfg, Step::Extract).thinking, None, "Gemini 3's own default");
        assert_eq!(step(&cfg, Step::Translate).thinking, Some(Medium), "Opus 5.5's own default");
        assert_eq!(step(&cfg, Step::Verify).thinking, Some(High), "offered, so kept");
        assert_eq!(step(&cfg, Step::WebSearch).thinking, Some(Medium));
    }

    #[test]
    fn per_step_turns_on_when_the_text_steps_differ() {
        let mut cfg = library(&[sample_model(Provider::Anthropic, "claude-sonnet-5-5")]);
        assert!(!cfg.per_step);
        // Web search isn't a text step.
        let searching = StepSettings { timeout: Some(9), ..step(&cfg, Step::WebSearch).clone() };
        put(&mut cfg, Step::WebSearch, searching);
        normalize(&mut cfg);
        assert!(!cfg.per_step);
        let verifying = StepSettings { timeout: Some(9), ..step(&cfg, Step::Verify).clone() };
        put(&mut cfg, Step::Verify, verifying);
        normalize(&mut cfg);
        assert!(cfg.per_step);
        // Never switched back off by itself: that's what "Use one model" is for.
        let same = step(&cfg, Step::Translate).clone();
        put(&mut cfg, Step::Verify, same);
        normalize(&mut cfg);
        assert!(cfg.per_step);
    }

    #[test]
    fn one_key_per_cloud_provider() {
        let mut cfg = default_config();
        cfg.provider_keys.insert("anthropic".into(), "sk-map".into());
        cfg.provider_keys.insert("google".into(), "".into());
        cfg.models = vec![
            Model { id: "m1".into(), ..keyed(Provider::Anthropic, "claude-sonnet-5-5", "sk-own") },
            Model { id: "m2".into(), ..keyed(Provider::Openai, "gpt-5.5", "sk-oai") },
            Model { id: "m3".into(), ..keyed(Provider::Openai, "gpt-5.4-mini", "sk-oai-2") },
            Model { id: "m4".into(), ..keyed(Provider::Google, "gemini-2.5-pro", "AIza-1") },
            Model {
                id: "m5".into(),
                base_url: "https://gw.example.com/v1".into(),
                ..keyed(Provider::Custom, "m", "gw-key")
            },
        ];
        normalize(&mut cfg);
        let keys: Vec<&str> = cfg.models.iter().map(|m| m.api_key.as_str()).collect();
        // The remembered key wins; a key the map lacked is adopted (the first one
        // seen); an empty remembered key counts as none; custom keys are per model.
        assert_eq!(keys, ["sk-map", "sk-oai", "sk-oai", "AIza-1", "gw-key"]);
        assert_eq!(remembered(&cfg, "openai").as_deref(), Some("sk-oai"));
        assert_eq!(remembered(&cfg, "google").as_deref(), Some("AIza-1"));
        assert_eq!(remembered(&cfg, "custom"), None, "custom keys aren't account keys");
    }

    #[test]
    fn normalize_is_idempotent() {
        let mut cfg = mixed();
        cfg.models.push(Model { id: "m1".into(), ..keyed(Provider::Openai, "gpt-5.5", "sk-oai") });
        put(&mut cfg, Step::Translate, StepSettings { thinking: Some(Off), ..on("gone") });
        put(&mut cfg, Step::WebSearch, on("m4"));
        normalize(&mut cfg);
        let once = cfg.clone();
        normalize(&mut cfg);
        assert_eq!(cfg, once);
    }

    // ---- save_model ----

    #[test]
    fn the_first_model_does_every_step() {
        let mut cfg = default_config();
        let id = save_model(&mut cfg, sample_model(Provider::Anthropic, "claude-sonnet-5-5"), None).unwrap();
        assert_eq!(id, "m1");
        assert_eq!((cfg.models[0].id.as_str(), cfg.models[0].color), ("m1", 0));
        // Unset tuning, and the model's own Thinking default.
        let shared = StepSettings { thinking: Some(High), ..on("m1") };
        for s in Step::ALL {
            assert_eq!(step(&cfg, s), &shared, "{s:?}");
        }
        assert!(!cfg.per_step);

        let mut local = default_config();
        save_model(&mut local, sample_model(Provider::Ollama, "qwen3"), None).unwrap();
        assert_eq!(step(&local, Step::Translate), &on("m1"));
        assert_eq!(web_search(&local), None, "it can't search");
    }

    #[test]
    fn new_models_get_the_first_free_id_and_colour() {
        let mut cfg = library(&[
            sample_model(Provider::Ollama, "a"),
            sample_model(Provider::Ollama, "b"),
            sample_model(Provider::Ollama, "c"),
        ]);
        let colors = |cfg: &AppConfig| cfg.models.iter().map(|m| m.color).collect::<Vec<_>>();
        assert_eq!(ids(&cfg), ["m1", "m2", "m3"]);
        assert_eq!(colors(&cfg), [0, 1, 2]);
        remove_model(&mut cfg, "m2").unwrap();
        // The form's colour is ignored: the backend picks the first free one.
        let id = save_model(&mut cfg, Model { color: 9, ..sample_model(Provider::Ollama, "d") }, None).unwrap();
        assert_eq!(id, "m2");
        assert_eq!(ids(&cfg), ["m1", "m3", "m2"]);
        assert_eq!(colors(&cfg), [0, 2, 1]);
    }

    #[test]
    fn saving_a_model_the_library_has_updates_it_instead_of_adding_a_copy() {
        let mut cfg = library(&[sample_model(Provider::Anthropic, "claude-sonnet-5-5")]);
        let again = Model { connect_timeout: Some(5), ..sample_model(Provider::Anthropic, "claude-sonnet-5-5") };
        assert_eq!(save_model(&mut cfg, again, None).unwrap(), "m1");
        assert_eq!(cfg.models.len(), 1);
        assert_eq!(cfg.models[0].connect_timeout, Some(5));

        // Another model id, or the same one at another address, is another model.
        save_model(&mut cfg, sample_model(Provider::Anthropic, "claude-opus-5-5"), None).unwrap();
        let a = Model { base_url: "https://a.example.com/v1".into(), ..sample_model(Provider::Custom, "claude-sonnet-5-5") };
        let b = Model { base_url: "https://b.example.com/v1".into(), ..a.clone() };
        save_model(&mut cfg, a.clone(), None).unwrap();
        save_model(&mut cfg, b, None).unwrap();
        assert_eq!(save_model(&mut cfg, a, None).unwrap(), "m3");
        assert_eq!(ids(&cfg), ["m1", "m2", "m3", "m4"]);
    }

    #[test]
    fn updating_a_model_keeps_its_id_colour_and_the_settings_of_its_steps() {
        let mut cfg = library(&[sample_model(Provider::Anthropic, "claude-sonnet-5-5")]);
        let tuned = StepSettings { thinking: Some(Low), timeout: Some(300), ..on("m1") };
        set_shared_steps(&mut cfg, tuned.clone()).unwrap();

        // The model menu: another model of the same provider, which offers Low too.
        let opus = Model { id: "m1".into(), color: 5, ..sample_model(Provider::Anthropic, "claude-opus-5-5") };
        save_model(&mut cfg, opus, None).unwrap();
        assert_eq!((cfg.models[0].color, cfg.models[0].model.as_str()), (0, "claude-opus-5-5"));
        for s in Step::ALL {
            assert_eq!(step(&cfg, s), &tuned, "{s:?}");
        }

        // A model without that level gets its default; one that can't search
        // drops web search.
        let qwen = Model { id: "m1".into(), ..sample_model(Provider::Ollama, "qwen3") };
        save_model(&mut cfg, qwen, None).unwrap();
        assert_eq!(step(&cfg, Step::Translate), &StepSettings { thinking: None, ..tuned });
        assert_eq!(web_search(&cfg), None);
    }

    #[test]
    fn updating_a_model_that_is_gone_fails() {
        let mut cfg = library(&[sample_model(Provider::Ollama, "qwen3")]);
        let ghost = Model { id: "m9".into(), ..sample_model(Provider::Ollama, "x") };
        let err = save_model(&mut cfg, ghost, None).unwrap_err().to_string();
        assert_eq!(err, "That model isn't in your library any more.");
        assert_eq!(ids(&cfg), ["m1"]);
    }

    #[test]
    fn assign_hands_the_step_over_with_the_models_default_thinking() {
        let mut cfg = library(&[sample_model(Provider::Anthropic, "claude-sonnet-5-5")]);
        set_shared_steps(&mut cfg, StepSettings { thinking: Some(Low), timeout: Some(300), ..on("m1") }).unwrap();
        let id = save_model(&mut cfg, sample_model(Provider::Anthropic, "claude-opus-5-5"), Some(Step::Normalize))
            .unwrap();
        // Tuning kept; Thinking reset to Opus 5.5's own default.
        assert_eq!(step(&cfg, Step::Normalize), &StepSettings { thinking: Some(Medium), timeout: Some(300), ..on(&id) });
        assert_eq!(step(&cfg, Step::Translate).model, "m1");
        assert!(cfg.per_step, "the text steps differ now");
    }

    #[test]
    fn assigning_web_search_from_off_starts_on_default_tuning() {
        // The one-model view's "Add a model that can" for web search.
        let mut cfg = library(&[sample_model(Provider::Ollama, "qwen3")]);
        set_shared_steps(&mut cfg, StepSettings { timeout: Some(900), ..on("m1") }).unwrap();
        let id = save_model(&mut cfg, sample_model(Provider::Google, "gemini-3-flash-preview"), Some(Step::WebSearch))
            .unwrap();
        assert_eq!(web_search(&cfg), Some(&on(&id)));
        assert!(!cfg.per_step, "web search isn't a text step");
    }

    #[test]
    fn web_search_can_only_go_to_a_model_that_searches() {
        let mut cfg = library(&[sample_model(Provider::Anthropic, "claude-sonnet-5-5")]);
        let before = cfg.clone();
        let err = save_model(&mut cfg, sample_model(Provider::Ollama, "qwen3"), Some(Step::WebSearch)).unwrap_err();
        assert_eq!(err.to_string(), "qwen3 can't search the web");
        assert_eq!(cfg, before, "nothing saved");
        // Without the assignment it joins the library as usual.
        save_model(&mut cfg, sample_model(Provider::Ollama, "qwen3"), None).unwrap();
        assert_eq!(cfg.models.len(), 2);
    }

    #[test]
    fn a_cloud_key_is_shared_by_every_model_of_its_provider() {
        let mut cfg = library(&[keyed(Provider::Anthropic, "claude-sonnet-5-5", "sk-1")]);
        // A new model of the provider takes the remembered key...
        save_model(&mut cfg, keyless(Provider::Anthropic, "claude-opus-5-5"), Some(Step::Normalize)).unwrap();
        assert_eq!(cfg.models[1].api_key, "sk-1");
        // ...and a key saved on any of them reaches all of them.
        let rotated = Model { id: "m2".into(), ..keyed(Provider::Anthropic, "claude-opus-5-5", " sk-2\n") };
        save_model(&mut cfg, rotated, None).unwrap();
        let keys: Vec<&str> = cfg.models.iter().map(|m| m.api_key.as_str()).collect();
        assert_eq!(keys, ["sk-2", "sk-2"]);
        assert_eq!(remembered(&cfg, "anthropic").as_deref(), Some("sk-2"));

        // Another provider's models never follow; custom keys aren't remembered.
        save_model(&mut cfg, keyed(Provider::Openai, "gpt-5.5", "sk-oai"), None).unwrap();
        let custom = Model { base_url: "https://gw.example.com/v1".into(), ..keyed(Provider::Custom, "m", "gw-key") };
        save_model(&mut cfg, custom, None).unwrap();
        assert_eq!((cfg.models[2].api_key.as_str(), cfg.models[3].api_key.as_str()), ("sk-oai", "gw-key"));
        assert_eq!(remembered(&cfg, "anthropic").as_deref(), Some("sk-2"));
        assert_eq!(remembered(&cfg, "custom"), None);
    }

    #[test]
    fn editing_a_model_into_a_twin_of_another_merges_them() {
        // Sonnet does everything but Verify, which runs on Haiku.
        let mut cfg = library(&[
            keyed(Provider::Anthropic, "claude-sonnet-5-5", "k"),
            keyed(Provider::Anthropic, "claude-haiku-5-5", "k"),
        ]);
        set_step(&mut cfg, Step::Verify, Some(on("m2"))).unwrap();
        set_step(&mut cfg, Step::Translate, Some(StepSettings { thinking: Some(Low), ..on("m1") })).unwrap();
        // Haiku edited into a second Sonnet: one Sonnet entry remains (Haiku's id and colour).
        let edited = Model { id: "m2".into(), ..keyed(Provider::Anthropic, "claude-sonnet-5-5", "k") };
        save_model(&mut cfg, edited, None).unwrap();
        assert_eq!(ids(&cfg), ["m2"]);
        assert_eq!(model(&cfg, "m2").color, 1);
        // Every step is on it now, each keeping its own settings.
        for s in [Step::Extract, Step::Normalize, Step::Translate, Step::Verify] {
            assert_eq!(step(&cfg, s).model, "m2", "{s:?}");
        }
        assert_eq!(step(&cfg, Step::Translate).thinking, Some(Low));
    }

    #[test]
    fn save_model_canonicalizes_what_the_form_sent() {
        let mut cfg = default_config();
        let mut m = sample_model(Provider::Google, " gemini-3-flash-preview ");
        m.driver = Driver::Openai;
        m.base_url = "https://generativelanguage.googleapis.com/v1beta/openai/".into();
        m.api_key = " AIza-key ".into();
        save_model(&mut cfg, m, None).unwrap();
        let saved = &cfg.models[0];
        assert_eq!(
            (saved.driver, saved.model.as_str(), saved.api_key.as_str()),
            (Driver::Gemini, "gemini-3-flash-preview", "AIza-key")
        );
        assert_eq!(saved.base_url, "https://generativelanguage.googleapis.com/v1beta");
        assert_eq!(remembered(&cfg, "google").as_deref(), Some("AIza-key"));
    }

    // ---- remove_model ----

    #[test]
    fn removing_a_model_moves_its_steps_to_the_first_remaining_one() {
        let mut cfg = library(&[
            sample_model(Provider::Anthropic, "claude-sonnet-5-5"),
            sample_model(Provider::Anthropic, "claude-opus-5-5"),
            sample_model(Provider::Google, "gemini-3-flash-preview"),
        ]);
        let normalizing = StepSettings { thinking: Some(High), timeout: Some(300), ..on("m2") };
        set_step(&mut cfg, Step::Normalize, Some(normalizing)).unwrap();
        set_step(&mut cfg, Step::WebSearch, Some(on("m3"))).unwrap();
        remove_model(&mut cfg, "m2").unwrap();
        assert_eq!(step(&cfg, Step::Normalize), &StepSettings { thinking: Some(High), timeout: Some(300), ..on("m1") });
        // Web search: the first remaining model that can search.
        remove_model(&mut cfg, "m3").unwrap();
        assert_eq!(web_search(&cfg).map(|s| s.model.as_str()), Some("m1"));

        // Thinking is re-fitted to the model the step lands on; with no
        // remaining model that can search, web search turns off.
        let mut cfg = library(&[sample_model(Provider::Ollama, "qwen3"), sample_model(Provider::Anthropic, "claude-sonnet-5-5")]);
        set_step(&mut cfg, Step::Verify, Some(StepSettings { thinking: Some(Low), max_tokens: Some(999), ..on("m2") })).unwrap();
        set_step(&mut cfg, Step::WebSearch, Some(on("m2"))).unwrap();
        remove_model(&mut cfg, "m2").unwrap();
        assert_eq!(step(&cfg, Step::Verify), &StepSettings { max_tokens: Some(999), ..on("m1") });
        assert_eq!(web_search(&cfg), None);
    }

    #[test]
    fn removing_the_last_model_leaves_nothing_set_up() {
        let mut cfg = library(&[sample_model(Provider::Anthropic, "claude-sonnet-5-5")]);
        show_every_step(&mut cfg).unwrap();
        remove_model(&mut cfg, "m1").unwrap();
        assert!(cfg.models.is_empty());
        assert_eq!((cfg.steps.as_ref(), cfg.per_step), (None, false));
    }

    #[test]
    fn removing_a_model_forgets_its_key_unless_another_model_uses_it() {
        let mut cfg = library(&[
            keyed(Provider::Anthropic, "claude-sonnet-5-5", "sk-ant"),
            keyed(Provider::Anthropic, "claude-haiku-5-5", "sk-ant"),
            keyed(Provider::Openai, "gpt-5.5", "sk-oai"),
        ]);
        cfg.provider_keys.insert("google".into(), "AIza-switched-away".into());
        remove_model(&mut cfg, "m3").unwrap();
        assert_eq!(remembered(&cfg, "openai"), None, "gone with its last model");
        remove_model(&mut cfg, "m2").unwrap();
        assert_eq!(remembered(&cfg, "anthropic").as_deref(), Some("sk-ant"), "m1 still uses it");
        remove_model(&mut cfg, "m1").unwrap();
        assert_eq!(remembered(&cfg, "anthropic"), None);
        // A key merely switched away from stays: it refills the form.
        assert_eq!(remembered(&cfg, "google").as_deref(), Some("AIza-switched-away"));
    }

    #[test]
    fn removing_a_model_that_is_gone_fails() {
        let mut cfg = library(&[sample_model(Provider::Ollama, "qwen3")]);
        let before = cfg.clone();
        assert!(remove_model(&mut cfg, "m7").is_err());
        assert_eq!(cfg, before);
    }

    // ---- set_step ----

    #[test]
    fn set_step_changes_one_step() {
        let mut cfg = library(&[
            sample_model(Provider::Anthropic, "claude-sonnet-5-5"),
            sample_model(Provider::Anthropic, "claude-opus-5-5"),
        ]);
        set_step(&mut cfg, Step::Normalize, Some(StepSettings { thinking: Some(High), ..on("m2") })).unwrap();
        assert_eq!(step(&cfg, Step::Normalize), &StepSettings { thinking: Some(High), ..on("m2") });
        assert_eq!(step(&cfg, Step::Extract).model, "m1");
        assert!(cfg.per_step);
        // A model change sends no Thinking: the model's own default.
        set_step(&mut cfg, Step::Verify, Some(on("m2"))).unwrap();
        assert_eq!(step(&cfg, Step::Verify).thinking, Some(Medium));
    }

    #[test]
    fn set_step_rules() {
        let mut empty = default_config();
        assert!(matches!(set_step(&mut empty, Step::Translate, Some(on("m1"))), Err(AppError::NoActiveConnection)));

        let mut cfg = library(&[sample_model(Provider::Ollama, "qwen3"), sample_model(Provider::Openai, "gpt-5.5")]);
        let before = cfg.clone();
        let off = set_step(&mut cfg, Step::Translate, None).unwrap_err();
        assert_eq!(off.to_string(), "Translate can't be turned off");
        assert!(set_step(&mut cfg, Step::Verify, Some(on("m9"))).is_err(), "unknown model");
        let cant = set_step(&mut cfg, Step::WebSearch, Some(on("m1"))).unwrap_err();
        assert_eq!(cant.to_string(), "qwen3 can't search the web");
        assert_eq!(cfg, before, "unchanged on error");

        set_step(&mut cfg, Step::WebSearch, Some(on("m2"))).unwrap();
        assert_eq!(web_search(&cfg).map(|s| s.model.as_str()), Some("m2"));
        set_step(&mut cfg, Step::WebSearch, None).unwrap();
        assert_eq!(web_search(&cfg), None);
    }

    // ---- set_shared_steps / show_every_step ----

    #[test]
    fn shared_settings_reach_every_text_step_and_a_web_search_on_the_same_model() {
        let mut cfg = library(&[sample_model(Provider::Anthropic, "claude-sonnet-5-5")]);
        let shared = StepSettings { thinking: Some(Low), timeout: Some(99), concurrency: Some(2), ..on("m1") };
        set_shared_steps(&mut cfg, shared.clone()).unwrap();
        for s in Step::ALL {
            assert_eq!(step(&cfg, s), &shared, "{s:?}");
        }
        assert!(!cfg.per_step);
    }

    #[test]
    fn shared_settings_leave_a_separate_search_model_alone() {
        let mut cfg = library(&[sample_model(Provider::Anthropic, "claude-sonnet-5-5")]);
        save_model(&mut cfg, sample_model(Provider::Google, "gemini-3-flash-preview"), Some(Step::WebSearch)).unwrap();
        let separate = web_search(&cfg).cloned();
        set_shared_steps(&mut cfg, StepSettings { timeout: Some(99), ..on("m1") }).unwrap();
        assert_eq!(web_search(&cfg).cloned(), separate);
        assert_eq!(step(&cfg, Step::Verify).timeout, Some(99));
    }

    #[test]
    fn shared_settings_on_another_model_take_a_riding_web_search_along_if_it_can_search() {
        let mut cfg = library(&[sample_model(Provider::Anthropic, "claude-sonnet-5-5"), sample_model(Provider::Ollama, "qwen3")]);
        set_shared_steps(&mut cfg, on("m2")).unwrap();
        assert_eq!(step(&cfg, Step::Translate), &on("m2"));
        assert_eq!(web_search(&cfg), None, "Qwen can't search");

        let mut cfg = library(&[sample_model(Provider::Anthropic, "claude-sonnet-5-5"), sample_model(Provider::Openai, "gpt-5.5")]);
        set_shared_steps(&mut cfg, on("m2")).unwrap();
        assert_eq!(web_search(&cfg), Some(&on("m2")));
    }

    #[test]
    fn shared_settings_need_a_library_model() {
        assert!(matches!(set_shared_steps(&mut default_config(), on("m1")), Err(AppError::NoActiveConnection)));
        let mut cfg = library(&[sample_model(Provider::Ollama, "qwen3")]);
        assert!(set_shared_steps(&mut cfg, on("m2")).is_err());
    }

    #[test]
    fn show_every_step_only_switches_the_view() {
        assert!(matches!(show_every_step(&mut default_config()), Err(AppError::NoActiveConnection)));
        let mut cfg = library(&[sample_model(Provider::Anthropic, "claude-sonnet-5-5")]);
        let steps = cfg.steps.clone();
        show_every_step(&mut cfg).unwrap();
        assert!(cfg.per_step);
        assert_eq!(cfg.steps, steps);
    }

    // ---- use_one_model ----

    #[test]
    fn use_one_model_takes_the_settings_the_model_already_has() {
        let mut cfg = mixed();
        use_one_model(&mut cfg, "m2").unwrap();
        // Translate isn't on Opus; Extract is the first step that is.
        let shared = StepSettings { thinking: Some(Medium), timeout: Some(200), ..on("m2") };
        for s in Step::TEXT {
            assert_eq!(step(&cfg, s), &shared, "{s:?}");
        }
        assert!(!cfg.per_step);

        let mut cfg = mixed();
        use_one_model(&mut cfg, "m1").unwrap();
        assert_eq!(step(&cfg, Step::Verify).timeout, Some(100), "Translate's settings come first");

        // A model no step uses starts on its defaults.
        let mut cfg = mixed();
        use_one_model(&mut cfg, "m4").unwrap();
        assert_eq!(step(&cfg, Step::Extract), &on("m4"));
    }

    #[test]
    fn use_one_model_web_search_rules() {
        // On, and the chosen model can search: it follows, with the shared settings.
        let mut cfg = mixed();
        use_one_model(&mut cfg, "m1").unwrap();
        assert_eq!(web_search(&cfg), Some(step(&cfg, Step::Translate)));
        assert_eq!(ids(&cfg), ["m1"], "Gemini does nothing any more");

        // The chosen model can't search: web search stays on the model it was on.
        let mut cfg = mixed();
        use_one_model(&mut cfg, "m4").unwrap();
        assert_eq!(web_search(&cfg), Some(&on("m3")));
        assert_eq!(ids(&cfg), ["m3", "m4"]);

        // Even when that was the old shared model.
        let mut cfg = library(&[sample_model(Provider::Anthropic, "claude-sonnet-5-5"), sample_model(Provider::Ollama, "qwen3")]);
        let searching = step(&cfg, Step::WebSearch).clone();
        use_one_model(&mut cfg, "m2").unwrap();
        assert_eq!(web_search(&cfg), Some(&searching));
        assert_eq!(ids(&cfg), ["m1", "m2"]);

        // Off stays off.
        let mut cfg = mixed();
        set_step(&mut cfg, Step::WebSearch, None).unwrap();
        use_one_model(&mut cfg, "m2").unwrap();
        assert_eq!(web_search(&cfg), None);
    }

    #[test]
    fn use_one_model_drops_unused_models_and_their_keys() {
        let mut cfg = mixed();
        use_one_model(&mut cfg, "m1").unwrap();
        assert_eq!(ids(&cfg), ["m1"]);
        assert_eq!(remembered(&cfg, "anthropic").as_deref(), Some("sk-ant"), "Sonnet still uses it");
        assert_eq!(remembered(&cfg, "google"), None, "gone with Gemini");
    }

    #[test]
    fn use_one_model_needs_a_library_model() {
        assert!(matches!(use_one_model(&mut default_config(), "m1"), Err(AppError::NoActiveConnection)));
        let mut cfg = mixed();
        let before = cfg.clone();
        assert!(use_one_model(&mut cfg, "m9").is_err());
        assert_eq!(cfg, before);
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
        assert!(cfg.models.is_empty());
        assert_eq!((cfg.steps.as_ref(), cfg.per_step), (None, false));
        assert!(cfg.provider_keys.is_empty());
        assert_eq!((cfg.default_source.as_str(), cfg.default_target.as_str()), ("zh", "en"));
    }

    #[test]
    fn active_anthropic_with_separate_openai_search_migrates_to_two_models() {
        let mut v1 = v1_seeded();
        v1["connections"]["anthropic"]["api_key"] = json!("sk-ant-1");
        v1["connections"]["openai"]["api_key"] = json!("sk-oai-1");
        v1["connections"]["openai"]["web_search"] = json!(true);
        v1["connections"]["openai"]["thinking_enabled"] = json!(true);
        let cfg = config_from_stored(v1);

        assert_eq!(ids(&cfg), ["m1", "m2"]);
        let t = model(&cfg, "m1");
        assert_eq!((t.provider, t.driver, t.model.as_str()), (Provider::Anthropic, Driver::Anthropic, "claude-opus-4-5"));
        assert_eq!((t.api_key.as_str(), t.color, t.connect_timeout), ("sk-ant-1", 0, Some(10)));
        let shared = step(&cfg, Step::Translate);
        assert_eq!(shared.max_tokens, Some(16000), "tuning copied");
        for s in Step::TEXT {
            assert_eq!(step(&cfg, s), shared, "{s:?}");
        }
        let s = model(&cfg, "m2");
        assert_eq!((s.provider, s.driver), (Provider::Openai, Driver::OpenaiResponses));
        assert_eq!((s.api_key.as_str(), s.color), ("sk-oai-1", 1));
        let ws = web_search(&cfg).unwrap();
        assert_eq!(ws.model, "m2");
        assert_eq!(ws.thinking, None, "v1's separate search model had no thinking");
        assert!(!cfg.per_step);
    }

    #[test]
    fn personalization_on_the_translation_entry_searches_with_the_shared_settings() {
        let mut v1 = v1_seeded();
        v1["active_connection"] = json!("openai");
        v1["connections"]["openai"]["api_key"] = json!("sk-oai-1");
        v1["connections"]["openai"]["web_search"] = json!(true);
        let cfg = config_from_stored(v1);
        assert_eq!(ids(&cfg), ["m1"]);
        assert_eq!(model(&cfg, "m1").provider, Provider::Openai);
        assert_eq!(web_search(&cfg), Some(step(&cfg, Step::Translate)));
    }

    #[test]
    fn a_separate_search_entry_on_the_translation_model_shares_its_library_model() {
        let main = v1_entry("openai-responses", "https://api.openai.com/v1", "sk-oai-1", "gpt-5.2");
        let mut search = main.clone();
        search["timeout"] = json!(60);
        search["web_search"] = json!(true);
        search["api_key"] = json!("sk-oai-2");
        let v1 = json!({
            "active_connection": "main", "personalization_model": "search",
            "connections": { "main": main, "search": search }
        });
        let cfg = config_from_stored(v1);
        assert_eq!(ids(&cfg), ["m1"]);
        let ws = web_search(&cfg).unwrap();
        assert_eq!((ws.model.as_str(), ws.timeout), ("m1", Some(60)), "with its own settings");
        assert_eq!(step(&cfg, Step::Translate).timeout, Some(120));
        assert_eq!(model(&cfg, "m1").api_key, "sk-oai-1", "one key per provider: the translation's");
    }

    #[test]
    fn personalization_without_web_search_migrates_to_off() {
        let mut v1 = v1_seeded();
        v1["connections"]["anthropic"]["api_key"] = json!("sk-ant-1");
        v1["connections"]["openai"]["api_key"] = json!("sk-oai-1"); // web_search null
        assert_eq!(web_search(&config_from_stored(v1.clone())), None);
        // Wrong driver: v1 never searched on chat completions.
        v1["connections"]["openai"]["web_search"] = json!(true);
        v1["connections"]["openai"]["driver"] = json!("openai");
        assert_eq!(web_search(&config_from_stored(v1.clone())), None);
        // Unusable (no key).
        v1["connections"]["openai"]["driver"] = json!("openai-responses");
        v1["connections"]["openai"]["api_key"] = json!("");
        let cfg = config_from_stored(v1);
        assert_eq!(web_search(&cfg), None);
        assert_eq!(ids(&cfg), ["m1"]);
    }

    #[test]
    fn google_moves_to_the_native_gemini_api() {
        let mut v1 = v1_seeded();
        v1["active_connection"] = json!("google");
        v1["connections"]["google"]["api_key"] = json!("AIza-1");
        let cfg = config_from_stored(v1);
        let t = model(&cfg, "m1");
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
        let cfg = config_from_stored(v1.clone());
        let t = model(&cfg, "m1");
        assert_eq!((t.provider, t.driver), (Provider::Custom, Driver::Anthropic));
        assert_eq!((t.base_url.as_str(), t.api_key.as_str()), ("https://api.z.ai/api/anthropic", "z-key"));

        v1["active_connection"] = json!("ollama");
        let cfg = config_from_stored(v1);
        let t = model(&cfg, "m1");
        assert_eq!((t.provider, t.driver), (Provider::Ollama, Driver::Openai));
        assert_eq!(t.base_url, "http://localhost:11434/v1");
    }

    #[test]
    fn unusable_active_falls_back_to_the_first_keyed_entry_by_name() {
        let mut v1 = v1_seeded(); // active = anthropic, no key
        v1["connections"]["zeta"] = v1_entry("openai", "https://openrouter.ai/api/v1", "or-key", "x/y");
        v1["connections"]["google"]["api_key"] = json!("AIza-1");
        let cfg = config_from_stored(v1);
        assert_eq!(model(&cfg, "m1").provider, Provider::Google, "google < zeta, and ollama doesn't count");
    }

    #[test]
    fn keys_are_harvested_and_the_translations_key_wins() {
        let mut v1 = v1_seeded();
        v1["connections"]["anthropic"]["api_key"] = json!("sk-ant-active");
        v1["connections"]["claude-alt"] =
            v1_entry("anthropic", "https://api.anthropic.com", "sk-ant-other", "claude-haiku-4-5");
        v1["connections"]["google"]["api_key"] = json!("AIza-1");
        v1["connections"]["mine"] = v1_entry("openai", "https://openrouter.ai/api/v1", "or-key", "x/y");
        let cfg = config_from_stored(v1);
        let keys: Vec<(&str, &str)> =
            cfg.provider_keys.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        // "claude-alt" sorts after "anthropic" but the translation still wins;
        // custom and local keys aren't remembered; Google's is, without a model.
        assert_eq!(keys, [("anthropic", "sk-ant-active"), ("google", "AIza-1")]);
        assert_eq!(model(&cfg, "m1").api_key, "sk-ant-active");
    }

    #[test]
    fn thinking_enabled_becomes_medium_then_what_the_model_offers() {
        let migrate = |driver: &str, base: &str, model: &str, enabled: bool| {
            let mut entry = v1_entry(driver, base, "key", model);
            entry["thinking_enabled"] = json!(enabled);
            entry["thinking_budget"] = json!(6000);
            let v1 = json!({ "active_connection": "c", "connections": { "c": entry } });
            step(&config_from_stored(v1), Step::Translate).thinking
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
        let cfg = config_from_stored(v1);
        assert_eq!(model(&cfg, "m1").api_key, "sk-ant-1");
        let t = step(&cfg, Step::Translate);
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
        assert!(cfg.models.is_empty());
        let stored = serde_json::to_value(&cfg).unwrap();
        let keys: Vec<&str> = stored.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            ["default_source", "default_target", "default_workdir", "models", "per_step", "provider_keys",
             "steps", "version"]
        );
    }

    #[test]
    fn partial_v1_config_fills_missing_fields_from_defaults() {
        let cfg = config_from_stored(json!({ "default_target": "ru" }));
        assert_eq!(cfg.default_source, "zh");
        assert_eq!(cfg.default_target, "ru");
        assert!(cfg.models.is_empty());
    }

    // ---- pre-release v2 (two roles under version 2) ----

    fn pre_release_v2() -> Value {
        json!({
            "version": 2, "default_source": "ko", "default_target": "en", "default_workdir": "/work",
            "translation": {
                "provider": "anthropic", "driver": "anthropic", "base_url": "https://api.anthropic.com",
                "api_key": "k", "model": "claude-sonnet-5-5", "thinking": "high", "max_tokens": 20000,
                "batch_dialogue_limit": null, "timeout": null, "connect_timeout": 15, "concurrency": null
            },
            "web_search": { "mode": "separate", "connection": {
                "provider": "google", "driver": "gemini",
                "base_url": "https://generativelanguage.googleapis.com/v1beta",
                "api_key": "AIza", "model": "gemini-2.5-flash", "thinking": null, "timeout": 60
            }},
            "provider_keys": { "anthropic": "k", "google": "AIza" }
        })
    }

    #[test]
    fn pre_release_v2_roles_become_a_library() {
        let cfg = config_from_stored(pre_release_v2());
        assert_eq!((cfg.default_source.as_str(), cfg.default_workdir.as_deref()), ("ko", Some("/work")));
        assert_eq!(ids(&cfg), ["m1", "m2"]);
        let m1 = model(&cfg, "m1");
        assert_eq!((m1.model.as_str(), m1.connect_timeout, m1.color), ("claude-sonnet-5-5", Some(15), 0));
        let shared = StepSettings { thinking: Some(High), max_tokens: Some(20000), ..on("m1") };
        for s in Step::TEXT {
            assert_eq!(step(&cfg, s), &shared, "{s:?}");
        }
        assert_eq!(model(&cfg, "m2").color, 1);
        assert_eq!(web_search(&cfg), Some(&StepSettings { timeout: Some(60), ..on("m2") }));
        assert!(!cfg.per_step);
        assert_eq!(remembered(&cfg, "google").as_deref(), Some("AIza"));
    }

    #[test]
    fn pre_release_v2_translation_mode_searches_on_m1_when_it_can() {
        let mut stored = pre_release_v2();
        stored["web_search"] = json!({ "mode": "translation" });
        let cfg = config_from_stored(stored.clone());
        assert_eq!(ids(&cfg), ["m1"]);
        assert_eq!(web_search(&cfg), Some(step(&cfg, Step::Translate)));

        stored["translation"]["provider"] = json!("ollama");
        stored["translation"]["driver"] = json!("openai");
        stored["translation"]["base_url"] = json!("http://localhost:11434/v1");
        stored["translation"]["model"] = json!("qwen3");
        assert_eq!(web_search(&config_from_stored(stored)), None, "it can't search");
    }

    #[test]
    fn pre_release_v2_separate_search_on_the_translation_model_reuses_m1() {
        let mut stored = pre_release_v2();
        stored["web_search"]["connection"] = stored["translation"].clone();
        stored["web_search"]["connection"]["timeout"] = json!(45);
        let cfg = config_from_stored(stored);
        assert_eq!(ids(&cfg), ["m1"]);
        let ws = web_search(&cfg).unwrap();
        assert_eq!((ws.model.as_str(), ws.timeout, ws.max_tokens), ("m1", Some(45), Some(20000)));
    }

    #[test]
    fn pre_release_v2_without_a_translation_sets_nothing_up() {
        let mut stored = pre_release_v2();
        stored["translation"] = json!(null);
        let cfg = config_from_stored(stored);
        assert!(cfg.models.is_empty());
        assert_eq!(cfg.steps, None);
        assert_eq!(remembered(&cfg, "google").as_deref(), Some("AIza"), "keys stay remembered");
    }

    #[test]
    fn pre_release_v2_invalid_fields_fall_back_alone() {
        let mut stored = pre_release_v2();
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
        let shared = step(&cfg, Step::Translate);
        assert_eq!(shared.timeout, None);
        assert_eq!(shared.thinking, Some(High), "unset, so the model's default");
        assert_eq!(shared.max_tokens, Some(20000));
        assert_eq!(cfg.provider_keys.len(), 2, "the bad key entry dropped alone");
        assert!(web_search(&cfg).is_some());
    }

    #[test]
    fn pre_release_v2_unparseable_roles_are_dropped() {
        let mut stored = pre_release_v2();
        stored["web_search"]["connection"]["model"] = json!(null);
        assert_eq!(web_search(&config_from_stored(stored.clone())), None);
        stored["web_search"] = json!({ "mode": "sometimes" });
        assert_eq!(web_search(&config_from_stored(stored.clone())), None);
        stored["web_search"] = json!("translation");
        assert_eq!(web_search(&config_from_stored(stored.clone())), None);
        stored["translation"]["provider"] = json!("carrier-pigeon");
        assert!(config_from_stored(stored).models.is_empty());
    }

    // ---- v2 parsing ----

    /// A per-step v2 config as the page leaves it.
    fn full_v2() -> AppConfig {
        let mut cfg = mixed();
        cfg.default_source = "ko".into();
        cfg.default_workdir = Some("/work".into());
        cfg.models[0].connect_timeout = Some(15);
        cfg
    }

    #[test]
    fn v2_round_trips_unchanged() {
        let cfg = full_v2();
        assert_eq!(config_from_stored(serde_json::to_value(&cfg).unwrap()), cfg);
        let one = library(&[sample_model(Provider::Anthropic, "claude-sonnet-5-5")]);
        assert_eq!(config_from_stored(serde_json::to_value(&one).unwrap()), one);
        let fresh = default_config();
        assert_eq!(config_from_stored(serde_json::to_value(&fresh).unwrap()), fresh);
    }

    #[test]
    fn v2_invalid_fields_fall_back_alone() {
        let mut stored = serde_json::to_value(full_v2()).unwrap();
        stored["default_source"] = json!(5);
        stored["default_workdir"] = json!(["x"]);
        stored["models"][1]["color"] = json!("red");
        stored["models"][0]["connect_timeout"] = json!("slow");
        stored["models"][0]["unknown"] = json!(true);
        stored["steps"]["extract"]["timeout"] = json!("slow");
        stored["steps"]["normalize"]["thinking"] = json!("maximum");
        stored["provider_keys"]["openai"] = json!(42);
        stored["per_step"] = json!("yes");
        let cfg = config_from_stored(stored);
        assert_eq!(cfg.default_source, "zh");
        assert_eq!(cfg.default_workdir, None);
        assert_eq!(ids(&cfg), ["m1", "m2", "m3", "m4"]);
        assert_eq!(model(&cfg, "m2").color, 0);
        assert_eq!(model(&cfg, "m1").connect_timeout, None);
        assert_eq!(step(&cfg, Step::Extract), &StepSettings { thinking: Some(Medium), ..on("m2") }, "only the timeout fell back");
        assert_eq!(
            step(&cfg, Step::Normalize),
            &StepSettings { thinking: Some(Medium), timeout: Some(300), ..on("m2") },
            "unset Thinking is the model's default"
        );
        assert_eq!(cfg.provider_keys.len(), 2, "the bad key entry dropped alone");
        assert!(cfg.per_step, "the text steps still differ");
    }

    #[test]
    fn v2_bad_models_drop_alone_and_their_steps_fall_back() {
        let mut stored = serde_json::to_value(full_v2()).unwrap();
        stored["models"][1]["provider"] = json!("carrier-pigeon"); // Opus
        stored["models"][3] = json!(7); // Qwen
        let cfg = config_from_stored(stored);
        assert_eq!(ids(&cfg), ["m1", "m3"]);
        // Extract and Normalize were on Opus: the first model now, tuning kept.
        assert_eq!(step(&cfg, Step::Normalize).model, "m1");
        assert_eq!(step(&cfg, Step::Normalize).timeout, Some(300));
        assert_eq!(web_search(&cfg).map(|s| s.model.as_str()), Some("m3"));
    }

    #[test]
    fn v2_bad_steps_fall_back_through_the_invariants() {
        let mut stored = serde_json::to_value(full_v2()).unwrap();
        stored["steps"]["extract"] = json!(5);
        stored["steps"]["verify"] = json!({ "timeout": 10 }); // names no model
        stored["steps"].as_object_mut().unwrap().remove("translate");
        stored["steps"]["web_search"] = json!("gemini");
        let cfg = config_from_stored(stored.clone());
        let first = StepSettings { thinking: Some(High), ..on("m1") };
        for s in [Step::Extract, Step::Verify, Step::Translate] {
            assert_eq!(step(&cfg, s), &first, "{s:?}");
        }
        assert_eq!(step(&cfg, Step::Normalize).model, "m2", "the good step stays");
        assert_eq!(web_search(&cfg), None, "an invalid web search is off");

        // No usable steps at all: every step on the first model.
        stored["steps"] = json!("oops");
        let cfg = config_from_stored(stored);
        for s in Step::ALL {
            assert_eq!(step(&cfg, s), &first, "{s:?}");
        }
    }

    #[test]
    fn v2_models_without_steps_put_every_step_on_the_first() {
        let mut stored = serde_json::to_value(full_v2()).unwrap();
        stored.as_object_mut().unwrap().remove("steps");
        let cfg = config_from_stored(stored);
        assert!(Step::ALL.iter().all(|s| step(&cfg, *s).model == "m1"));
    }

    #[test]
    fn v2_steps_without_models_are_dropped() {
        let mut stored = serde_json::to_value(full_v2()).unwrap();
        stored["models"] = json!({ "not": "a list" });
        let cfg = config_from_stored(stored);
        assert!(cfg.models.is_empty());
        assert_eq!((cfg.steps.as_ref(), cfg.per_step), (None, false));
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
