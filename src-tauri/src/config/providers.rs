//! Provider & capability catalog: the provider table behind the Connections
//! page, which controls a (provider, model) pair supports, how a thinking level
//! becomes request parameters, and the fresh-install default config.
//!
//! Capabilities are matched on model *families* rather than exact ids so that
//! new point releases (and ids typed by hand or fetched live) get the right
//! controls without a catalog update.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::config::{
    AppConfig, Connection, Driver, Provider, ThinkingLevel, WebSearch, CONFIG_VERSION,
};

/// A curated model suggestion. The live `/models` list supplements these.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    pub note: Option<String>,
}

/// Tuning a connection falls back to when it leaves a field unset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct TuningDefaults {
    pub max_tokens: u32,
    pub batch_dialogue_limit: u32,
    pub timeout: u32,
    pub concurrency: u32,
}

/// One provider tile on the Connections page and everything its form needs.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct ProviderInfo {
    pub provider: Provider,
    pub label: String,
    pub tagline: String,
    /// `None` = detect the API format on Connect (Custom).
    pub driver: Option<Driver>,
    /// Default address; editable for local and custom providers only.
    pub base_url: String,
    /// `false` for local servers, and for Custom where the key is optional.
    pub needs_key: bool,
    pub local: bool,
    pub key_url: Option<String>,
    pub key_hint: Option<String>,
    pub models: Vec<ModelOption>,
    /// Preselected model; empty when the provider's models are live-only.
    pub default_model: String,
    pub can_search: bool,
    pub can_think: bool,
    pub tuning: TuningDefaults,
}

/// The controls a model supports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct ModelCapabilities {
    /// Levels offered by the Thinking control; empty = no Thinking control.
    pub thinking_levels: Vec<ThinkingLevel>,
    /// The level a connection gets when it hasn't picked a supported one.
    pub default_thinking: Option<ThinkingLevel>,
    pub web_search: bool,
}

/// A thinking level resolved to the request parameters a driver sends.
/// Engine-internal (not exported to TS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingParam {
    /// Send nothing: the model's own default.
    None,
    /// `"thinking":{"type":"adaptive"}` + `"output_config":{"effort":…}`.
    AnthropicAdaptive(&'static str),
    /// `"thinking":{"type":"enabled","budget_tokens":n}` (pre-4.6 Claude).
    AnthropicBudget(u32),
    /// `"reasoning":{"effort":…}` (OpenAI Responses).
    OpenAiEffort(&'static str),
    /// `generationConfig.thinkingConfig.thinkingLevel` (Gemini 3).
    GeminiLevel(&'static str),
    /// `generationConfig.thinkingConfig.thinkingBudget` (Gemini 2.5).
    GeminiBudget(u32),
}

const ANTHROPIC_BASE: &str = "https://api.anthropic.com";
const OPENAI_BASE: &str = "https://api.openai.com/v1";
const GOOGLE_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";
const OLLAMA_BASE: &str = "http://localhost:11434/v1";
const LMSTUDIO_BASE: &str = "http://localhost:1234/v1";

const CLOUD_TUNING: TuningDefaults =
    TuningDefaults { max_tokens: 32000, batch_dialogue_limit: 100, timeout: 180, concurrency: 5 };
// Local models are slower and usually serve one request at a time: smaller
// batches, no parallelism, and a long timeout for the first (model-loading) call.
const LOCAL_TUNING: TuningDefaults =
    TuningDefaults { max_tokens: 8192, batch_dialogue_limit: 40, timeout: 600, concurrency: 1 };
const CUSTOM_TUNING: TuningDefaults =
    TuningDefaults { max_tokens: 16000, batch_dialogue_limit: 100, timeout: 120, concurrency: 3 };

/// Tile order on the Connections page.
const ALL_PROVIDERS: [Provider; 6] = [
    Provider::Anthropic,
    Provider::Openai,
    Provider::Google,
    Provider::Ollama,
    Provider::Lmstudio,
    Provider::Custom,
];

fn model(id: &str, label: &str, note: Option<&str>) -> ModelOption {
    ModelOption { id: id.into(), label: label.into(), note: note.map(Into::into) }
}

/// Every provider, in tile order.
pub fn providers() -> Vec<ProviderInfo> {
    ALL_PROVIDERS.into_iter().map(provider_info).collect()
}

pub fn provider_info(p: Provider) -> ProviderInfo {
    // Fields every cloud provider shares; local/custom override below.
    let cloud = |label: &str, tagline: &str, driver: Driver, base_url: &str, key_url: &str,
                 key_hint: &str, models: Vec<ModelOption>| ProviderInfo {
        provider: p,
        label: label.into(),
        tagline: tagline.into(),
        driver: Some(driver),
        base_url: base_url.into(),
        needs_key: true,
        local: false,
        key_url: Some(key_url.into()),
        key_hint: Some(key_hint.into()),
        default_model: models[0].id.clone(),
        models,
        can_search: true,
        can_think: true,
        tuning: CLOUD_TUNING,
    };
    let keyless = |label: &str, tagline: &str, driver: Option<Driver>, base_url: &str,
                   local: bool, tuning: TuningDefaults| ProviderInfo {
        provider: p,
        label: label.into(),
        tagline: tagline.into(),
        driver,
        base_url: base_url.into(),
        needs_key: false,
        local,
        key_url: None,
        key_hint: None,
        models: Vec::new(),
        default_model: String::new(),
        can_search: false,
        can_think: false,
        tuning,
    };
    match p {
        Provider::Anthropic => cloud(
            "Anthropic", "Claude models", Driver::Anthropic, ANTHROPIC_BASE,
            "https://console.anthropic.com/settings/keys", "Anthropic keys start with sk-ant-",
            vec![
                model("claude-sonnet-5-5", "Claude Sonnet 5.5", Some("Recommended")),
                model("claude-opus-5-5", "Claude Opus 5.5", Some("Best quality")),
                model("claude-haiku-5-5", "Claude Haiku 5.5", Some("Fastest & cheapest")),
            ],
        ),
        Provider::Openai => cloud(
            "OpenAI", "GPT models", Driver::OpenaiResponses, OPENAI_BASE,
            "https://platform.openai.com/api-keys", "OpenAI keys start with sk-",
            vec![
                model("gpt-5.5", "GPT-5.5", Some("Recommended")),
                model("gpt-5.4-mini", "GPT-5.4 mini", Some("Fastest & cheapest")),
                model("gpt-4.1", "GPT-4.1", None),
            ],
        ),
        Provider::Google => cloud(
            "Google", "Gemini models", Driver::Gemini, GOOGLE_BASE,
            "https://aistudio.google.com/apikey", "Google AI Studio keys start with AIza",
            vec![
                model("gemini-3.1-pro-preview", "Gemini 3.1 Pro", Some("Recommended")),
                model("gemini-3-flash-preview", "Gemini 3 Flash", Some("Fastest")),
                model("gemini-2.5-pro", "Gemini 2.5 Pro", None),
                model("gemini-2.5-flash", "Gemini 2.5 Flash", None),
            ],
        ),
        Provider::Ollama => keyless(
            "Ollama", "Runs on your computer", Some(Driver::Openai), OLLAMA_BASE, true, LOCAL_TUNING,
        ),
        Provider::Lmstudio => keyless(
            "LM Studio", "Runs on your computer", Some(Driver::Openai), LMSTUDIO_BASE, true,
            LOCAL_TUNING,
        ),
        Provider::Custom => keyless(
            "Other", "Any OpenAI- or Anthropic-compatible URL", None, "", false, CUSTOM_TUNING,
        ),
    }
}

// ---- capability rules -------------------------------------------------------

/// Which row of the capability table a (provider, model) pair falls in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    /// Claude 4.6+ (`thinking: adaptive` + effort; budgets are rejected).
    ClaudeAdaptive,
    /// Older Claude (`budget_tokens`).
    ClaudeLegacy,
    OpenAiReasoning,
    OpenAiOther,
    Gemini3,
    Gemini25,
    GeminiOther,
    /// No thinking control, no web search.
    Plain,
}

/// Families with current-style ids (`claude-<family>-<major>[-<minor>]…`).
const CLAUDE_FAMILIES: &[&str] = &["opus", "sonnet", "haiku", "fable", "mythos"];

/// `(family, major, minor)` of a lowercase current-style Claude id
/// (`claude-sonnet-5-5`, `claude-opus-4-1-20250805`, `claude-opus-4-1@20250805`).
/// Old-style ids (`claude-3-7-sonnet-…`) put the version first: `None`.
fn claude_generation(model: &str) -> Option<(&str, u32, u32)> {
    // A one- or two-digit number at the start of a segment. Longer runs are
    // date snapshots (`20250805`), not version numbers.
    fn version_part(segment: &str) -> Option<u32> {
        let end = segment.find(|c: char| !c.is_ascii_digit()).unwrap_or(segment.len());
        (1..=2).contains(&end).then(|| segment[..end].parse().ok()).flatten()
    }
    let rest = &model[model.find("claude-")? + "claude-".len()..];
    let mut parts = rest.split('-');
    let family = parts.next().filter(|f| CLAUDE_FAMILIES.contains(f))?;
    let major = version_part(parts.next()?)?;
    let minor = parts.next().and_then(version_part).unwrap_or(0);
    Some((family, major, minor))
}

fn family(p: Provider, model: &str) -> Family {
    let m = model.trim().to_ascii_lowercase();
    match p {
        Provider::Anthropic => match claude_generation(&m) {
            Some((_, major, minor)) if (major, minor) >= (4, 6) => Family::ClaudeAdaptive,
            _ if m.contains("claude-") => Family::ClaudeLegacy,
            _ => Family::Plain,
        },
        Provider::Openai => {
            if ["gpt-5", "o1", "o3", "o4"].iter().any(|prefix| m.starts_with(prefix)) {
                Family::OpenAiReasoning
            } else {
                Family::OpenAiOther
            }
        }
        Provider::Google => {
            // The Gemini API also names models `models/<id>`.
            let m = m.strip_prefix("models/").unwrap_or(&m);
            if m.starts_with("gemini-3") {
                Family::Gemini3
            } else if m.starts_with("gemini-2.5") {
                Family::Gemini25
            } else if m.starts_with("gemini-") {
                Family::GeminiOther
            } else {
                Family::Plain
            }
        }
        Provider::Ollama | Provider::Lmstudio | Provider::Custom => Family::Plain,
    }
}

impl Family {
    fn capabilities(self) -> ModelCapabilities {
        use ThinkingLevel::{High, Low, Medium, Off};
        let (thinking_levels, default_thinking, web_search) = match self {
            Family::ClaudeAdaptive => (vec![Low, Medium, High], Some(Medium), true),
            Family::ClaudeLegacy => (vec![Off, Low, Medium, High], Some(Off), true),
            Family::OpenAiReasoning => (vec![Low, Medium, High], None, true),
            Family::Gemini3 => (vec![Low, High], None, true),
            Family::Gemini25 => (vec![Low, Medium, High], None, true),
            Family::OpenAiOther | Family::GeminiOther => (vec![], None, true),
            Family::Plain => (vec![], None, false),
        };
        ModelCapabilities { thinking_levels, default_thinking, web_search }
    }
}

pub fn capabilities(p: Provider, model: &str) -> ModelCapabilities {
    family(p, model).capabilities()
}

/// A level the model offers: the chosen one if supported, else the model's
/// default. So a model change never leaves a level the new model can't take,
/// and models with a default always show a selected level.
pub fn normalize_thinking(
    p: Provider,
    model: &str,
    level: Option<ThinkingLevel>,
) -> Option<ThinkingLevel> {
    let caps = capabilities(p, model);
    match level {
        Some(l) if caps.thinking_levels.contains(&l) => Some(l),
        _ => caps.default_thinking,
    }
}

/// The request parameters for `level` on this model. `None`, `Off` and levels
/// the model doesn't offer send nothing. `max_tokens` is the request's own cap
/// (legacy Claude budgets are a share of it and must stay below it).
pub fn thinking_param(
    p: Provider,
    model: &str,
    level: Option<ThinkingLevel>,
    max_tokens: u32,
) -> ThinkingParam {
    use ThinkingLevel::{Low, Medium};
    let fam = family(p, model);
    let level = match level {
        Some(l) if l != ThinkingLevel::Off && fam.capabilities().thinking_levels.contains(&l) => l,
        _ => return ThinkingParam::None,
    };
    let effort = match level { Low => "low", Medium => "medium", _ => "high" };
    match fam {
        Family::ClaudeAdaptive => ThinkingParam::AnthropicAdaptive(effort),
        Family::ClaudeLegacy => {
            // The API floor is 1024 and the budget must be below max_tokens.
            let divisor = match level { Low => 8, Medium => 4, _ => 2 };
            let budget = (max_tokens / divisor).max(1024);
            if budget >= max_tokens {
                ThinkingParam::None
            } else {
                ThinkingParam::AnthropicBudget(budget)
            }
        }
        Family::OpenAiReasoning => ThinkingParam::OpenAiEffort(effort),
        Family::Gemini3 => ThinkingParam::GeminiLevel(effort),
        Family::Gemini25 => {
            // 2.5 counts thinking against maxOutputTokens: leave the answer
            // 8k tokens, or half the cap when the cap is small (128 is the
            // smallest budget 2.5 Pro takes).
            let budget = match level { Low => 1024, Medium => 8192, _ => 24576 };
            let room = max_tokens.saturating_sub(8192).max(max_tokens / 2);
            ThinkingParam::GeminiBudget(budget.min(room).max(128))
        }
        Family::OpenAiOther | Family::GeminiOther | Family::Plain => ThinkingParam::None,
    }
}

/// The Anthropic web-search tool `type` for this model: the dynamic-filtering
/// version on Opus/Sonnet 4.6+, the basic one everywhere else. The tool entry
/// is `{"type": <this>, "name": "web_search", "max_uses": 5}`.
pub fn anthropic_web_search_tool(model: &str) -> &'static str {
    match claude_generation(&model.trim().to_ascii_lowercase()) {
        Some(("opus" | "sonnet", major, minor)) if (major, minor) >= (4, 6) => {
            "web_search_20260209"
        }
        _ => "web_search_20250305",
    }
}

/// Best guess at the provider behind a stored base URL (v1 had no provider).
pub fn infer_provider(base_url: &str) -> Provider {
    let url = base_url.trim().to_ascii_lowercase();
    let after_scheme = url.split_once("://").map_or(url.as_str(), |(_, rest)| rest);
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit_once(':').map_or(authority, |(host, _)| host);
    match host {
        "api.anthropic.com" => Provider::Anthropic,
        "api.openai.com" => Provider::Openai,
        "generativelanguage.googleapis.com" => Provider::Google,
        _ if authority.ends_with(":11434") => Provider::Ollama,
        _ if authority.ends_with(":1234") => Provider::Lmstudio,
        _ => Provider::Custom,
    }
}

/// The curated label for a model ("Claude Sonnet 5.5"), or the raw id.
pub fn model_label(p: Provider, model: &str) -> String {
    let id = model.trim();
    provider_info(p)
        .models
        .into_iter()
        .find(|m| m.id.eq_ignore_ascii_case(id))
        .map_or_else(|| id.to_string(), |m| m.label)
}

/// A connection the way the engine expects it: whitespace trimmed off the
/// typed fields (a pasted key with a trailing newline would otherwise be
/// rejected), and cloud providers pinned to their own driver and base URL —
/// the UI doesn't let users edit those, and the capability table assumes them
/// (OpenAI web search, for one, only exists on the Responses API).
pub fn canonicalize(mut conn: Connection) -> Connection {
    conn.base_url = conn.base_url.trim().to_string();
    conn.api_key = conn.api_key.trim().to_string();
    conn.model = conn.model.trim().to_string();
    if conn.provider.is_cloud() {
        let info = provider_info(conn.provider);
        if let Some(driver) = info.driver {
            conn.driver = driver;
        }
        conn.base_url = info.base_url;
    }
    conn
}

/// The config of a fresh install: no connections yet, so the Connections page
/// opens on the provider picker.
pub fn default_config() -> AppConfig {
    AppConfig {
        version: CONFIG_VERSION,
        default_source: "zh".into(),
        default_target: "en".into(),
        default_workdir: None,
        translation: None,
        web_search: WebSearch::Off,
        provider_keys: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::sample;
    use ThinkingLevel::{High, Low, Medium, Off};

    #[test]
    fn providers_come_in_tile_order_with_their_drivers() {
        let all = providers();
        let ids: Vec<Provider> = all.iter().map(|p| p.provider).collect();
        assert_eq!(ids, ALL_PROVIDERS);
        let driver = |p: Provider| provider_info(p).driver;
        assert_eq!(driver(Provider::Anthropic), Some(Driver::Anthropic));
        assert_eq!(driver(Provider::Openai), Some(Driver::OpenaiResponses));
        assert_eq!(driver(Provider::Google), Some(Driver::Gemini));
        assert_eq!(driver(Provider::Ollama), Some(Driver::Openai));
        assert_eq!(driver(Provider::Lmstudio), Some(Driver::Openai));
        assert_eq!(driver(Provider::Custom), None); // detected on Connect
    }

    #[test]
    fn provider_table_flags_and_defaults() {
        for info in providers() {
            let cloud = info.provider.is_cloud();
            assert_eq!(info.needs_key, cloud, "{:?}", info.provider);
            assert_eq!(info.can_search, cloud, "{:?}", info.provider);
            assert_eq!(info.can_think, cloud, "{:?}", info.provider);
            assert_eq!(info.local, info.provider.is_local(), "{:?}", info.provider);
            assert_eq!(info.key_url.is_some(), cloud);
            assert_eq!(info.key_hint.is_some(), cloud);
            if cloud {
                // The preselected model is the first ("Recommended") suggestion.
                assert_eq!(info.default_model, info.models[0].id);
                assert_eq!(info.models[0].note.as_deref(), Some("Recommended"));
                assert_eq!(info.tuning, CLOUD_TUNING);
            } else {
                assert!(info.models.is_empty() && info.default_model.is_empty());
            }
        }
        assert_eq!(provider_info(Provider::Anthropic).default_model, "claude-sonnet-5-5");
        assert_eq!(provider_info(Provider::Openai).default_model, "gpt-5.5");
        assert_eq!(provider_info(Provider::Google).default_model, "gemini-3.1-pro-preview");
        assert_eq!(provider_info(Provider::Google).base_url, GOOGLE_BASE);
        assert_eq!(provider_info(Provider::Ollama).tuning, LOCAL_TUNING);
        assert_eq!(provider_info(Provider::Custom).tuning.concurrency, 3);
        assert_eq!(provider_info(Provider::Custom).base_url, "");
    }

    fn caps(levels: &[ThinkingLevel], default: Option<ThinkingLevel>, web: bool) -> ModelCapabilities {
        ModelCapabilities { thinking_levels: levels.to_vec(), default_thinking: default, web_search: web }
    }

    #[test]
    fn anthropic_capabilities_split_adaptive_from_legacy_claude() {
        let adaptive = caps(&[Low, Medium, High], Some(Medium), true);
        for m in ["claude-sonnet-5-5", "claude-opus-5-5", "claude-haiku-5-5", "claude-opus-4-6",
                  "claude-sonnet-4-6", "claude-opus-4-7", "claude-opus-4-8", "claude-sonnet-5",
                  "claude-opus-5", "claude-fable-5-1", "claude-mythos-5", "Claude-Sonnet-5-5",
                  "claude-opus-4-6@20260101", "claude-sonnet-4-6-20260115"] {
            assert_eq!(capabilities(Provider::Anthropic, m), adaptive, "{m}");
        }
        let legacy = caps(&[Off, Low, Medium, High], Some(Off), true);
        for m in ["claude-haiku-4-5", "claude-sonnet-4-5-20250929", "claude-opus-4-1",
                  "claude-opus-4-20250514", "claude-3-7-sonnet-20250219", "claude-3-5-haiku-latest",
                  "claude-opus-latest"] {
            assert_eq!(capabilities(Provider::Anthropic, m), legacy, "{m}");
        }
        assert_eq!(capabilities(Provider::Anthropic, "glm-4.6"), caps(&[], None, false));
    }

    #[test]
    fn openai_and_google_capabilities() {
        let reasoning = caps(&[Low, Medium, High], None, true);
        for m in ["gpt-5.5", "gpt-5.4-mini", "gpt-5", "o1", "o3-mini", "o4-mini"] {
            assert_eq!(capabilities(Provider::Openai, m), reasoning, "{m}");
        }
        assert_eq!(capabilities(Provider::Openai, "gpt-4.1"), caps(&[], None, true));

        assert_eq!(capabilities(Provider::Google, "gemini-3.1-pro-preview"), caps(&[Low, High], None, true));
        assert_eq!(capabilities(Provider::Google, "gemini-3-flash-preview"), caps(&[Low, High], None, true));
        assert_eq!(capabilities(Provider::Google, "models/gemini-2.5-pro"),
                   caps(&[Low, Medium, High], None, true));
        assert_eq!(capabilities(Provider::Google, "gemini-2.5-flash"), caps(&[Low, Medium, High], None, true));
        assert_eq!(capabilities(Provider::Google, "gemini-2.0-flash"), caps(&[], None, true));
        assert_eq!(capabilities(Provider::Google, "gemma-3-27b"), caps(&[], None, false));
    }

    #[test]
    fn local_and_custom_models_have_no_capabilities() {
        // Even a Claude id behind a custom gateway: we can't know what it supports.
        for p in [Provider::Ollama, Provider::Lmstudio, Provider::Custom] {
            assert_eq!(capabilities(p, "claude-sonnet-5-5"), caps(&[], None, false));
            assert_eq!(capabilities(p, "qwen3:32b"), caps(&[], None, false));
        }
    }

    #[test]
    fn normalize_thinking_keeps_supported_levels_and_falls_back_to_the_default() {
        let n = |p, m, l| normalize_thinking(p, m, l);
        assert_eq!(n(Provider::Anthropic, "claude-sonnet-5-5", Some(High)), Some(High));
        // Adaptive Claude has no Off: fall back to its default, as for None.
        assert_eq!(n(Provider::Anthropic, "claude-sonnet-5-5", Some(Off)), Some(Medium));
        assert_eq!(n(Provider::Anthropic, "claude-sonnet-5-5", None), Some(Medium));
        assert_eq!(n(Provider::Anthropic, "claude-haiku-4-5", None), Some(Off));
        // Gemini 3 has no Medium; its default is the model's own (nothing sent).
        assert_eq!(n(Provider::Google, "gemini-3-flash-preview", Some(Medium)), None);
        assert_eq!(n(Provider::Google, "gemini-3-flash-preview", Some(Low)), Some(Low));
        assert_eq!(n(Provider::Openai, "gpt-5.5", None), None);
        assert_eq!(n(Provider::Openai, "gpt-4.1", Some(High)), None);
        assert_eq!(n(Provider::Ollama, "llama3.1", Some(High)), None);
    }

    #[test]
    fn thinking_param_for_adaptive_claude_sends_effort() {
        let t = |l| thinking_param(Provider::Anthropic, "claude-sonnet-5-5", Some(l), 32000);
        assert_eq!(t(Low), ThinkingParam::AnthropicAdaptive("low"));
        assert_eq!(t(Medium), ThinkingParam::AnthropicAdaptive("medium"));
        assert_eq!(t(High), ThinkingParam::AnthropicAdaptive("high"));
        // Off isn't offered on adaptive models: nothing sent.
        assert_eq!(t(Off), ThinkingParam::None);
    }

    #[test]
    fn thinking_param_for_legacy_claude_sends_a_share_of_max_tokens() {
        let t = |l, max| thinking_param(Provider::Anthropic, "claude-haiku-4-5", Some(l), max);
        assert_eq!(t(Low, 32000), ThinkingParam::AnthropicBudget(4000));
        assert_eq!(t(Medium, 32000), ThinkingParam::AnthropicBudget(8000));
        assert_eq!(t(High, 32000), ThinkingParam::AnthropicBudget(16000));
        // Floored at the API minimum...
        assert_eq!(t(Low, 4096), ThinkingParam::AnthropicBudget(1024));
        // ...and dropped when the floor doesn't fit under max_tokens.
        assert_eq!(t(High, 1024), ThinkingParam::None);
        assert_eq!(t(Low, 16), ThinkingParam::None);
        assert_eq!(t(Off, 32000), ThinkingParam::None);
    }

    #[test]
    fn thinking_param_for_openai_and_gemini() {
        assert_eq!(thinking_param(Provider::Openai, "gpt-5.5", Some(High), 32000),
                   ThinkingParam::OpenAiEffort("high"));
        assert_eq!(thinking_param(Provider::Openai, "gpt-4.1", Some(High), 32000), ThinkingParam::None);

        let g3 = |l| thinking_param(Provider::Google, "gemini-3.1-pro-preview", Some(l), 32000);
        assert_eq!(g3(Low), ThinkingParam::GeminiLevel("low"));
        assert_eq!(g3(High), ThinkingParam::GeminiLevel("high"));
        assert_eq!(g3(Medium), ThinkingParam::None); // not a Gemini 3 level

        let g25 = |l| thinking_param(Provider::Google, "gemini-2.5-flash", Some(l), 32000);
        assert_eq!(g25(Low), ThinkingParam::GeminiBudget(1024));
        assert_eq!(g25(Medium), ThinkingParam::GeminiBudget(8192));
        assert_eq!(g25(High), ThinkingParam::GeminiBudget(23808)); // 32000 − 8k for the answer
        let small = |l| thinking_param(Provider::Google, "gemini-2.5-flash", Some(l), 16000);
        assert_eq!(small(Medium), ThinkingParam::GeminiBudget(8000)); // half of a small cap
        assert_eq!(small(High), ThinkingParam::GeminiBudget(8000));
    }

    #[test]
    fn thinking_param_sends_nothing_for_none_or_unsupported_providers() {
        assert_eq!(thinking_param(Provider::Anthropic, "claude-sonnet-5-5", None, 32000), ThinkingParam::None);
        assert_eq!(thinking_param(Provider::Custom, "claude-sonnet-5-5", Some(High), 32000), ThinkingParam::None);
        assert_eq!(thinking_param(Provider::Ollama, "qwen3", Some(High), 32000), ThinkingParam::None);
    }

    #[test]
    fn anthropic_web_search_tool_version_by_model() {
        for m in ["claude-opus-4-6", "claude-sonnet-4-6", "claude-sonnet-5", "claude-sonnet-5-5",
                  "claude-opus-5-5"] {
            assert_eq!(anthropic_web_search_tool(m), "web_search_20260209", "{m}");
        }
        for m in ["claude-haiku-5-5", "claude-haiku-4-5", "claude-sonnet-4-5", "claude-opus-4-1",
                  "claude-3-7-sonnet-20250219"] {
            assert_eq!(anthropic_web_search_tool(m), "web_search_20250305", "{m}");
        }
    }

    #[test]
    fn infer_provider_from_base_url() {
        assert_eq!(infer_provider("https://api.anthropic.com"), Provider::Anthropic);
        assert_eq!(infer_provider("https://API.anthropic.com/"), Provider::Anthropic);
        assert_eq!(infer_provider("https://api.openai.com/v1"), Provider::Openai);
        assert_eq!(
            infer_provider("https://generativelanguage.googleapis.com/v1beta/openai/"),
            Provider::Google
        );
        assert_eq!(infer_provider("http://localhost:11434/v1"), Provider::Ollama);
        assert_eq!(infer_provider("http://192.168.1.5:11434"), Provider::Ollama);
        assert_eq!(infer_provider("http://localhost:1234/v1"), Provider::Lmstudio);
        // Port matching is exact, not a prefix.
        assert_eq!(infer_provider("http://localhost:12345/v1"), Provider::Custom);
        assert_eq!(infer_provider("https://openrouter.ai/api/v1"), Provider::Custom);
        assert_eq!(infer_provider("https://api.z.ai/api/anthropic"), Provider::Custom);
        assert_eq!(infer_provider(""), Provider::Custom);
    }

    #[test]
    fn model_label_uses_the_curated_name_or_the_raw_id() {
        assert_eq!(model_label(Provider::Anthropic, "claude-sonnet-5-5"), "Claude Sonnet 5.5");
        assert_eq!(model_label(Provider::Google, "gemini-3-flash-preview"), "Gemini 3 Flash");
        assert_eq!(model_label(Provider::Anthropic, "claude-opus-4-6"), "claude-opus-4-6");
        assert_eq!(model_label(Provider::Ollama, "qwen3:32b"), "qwen3:32b");
    }

    #[test]
    fn canonicalize_pins_cloud_providers_and_trims() {
        let mut c = sample(Provider::Openai, " gpt-5.5 ");
        c.driver = Driver::Openai;
        c.base_url = "https://api.openai.com/v1/".into();
        c.api_key = "sk-abc\n".into();
        let c = canonicalize(c);
        assert_eq!(c.driver, Driver::OpenaiResponses);
        assert_eq!(c.base_url, OPENAI_BASE);
        assert_eq!(c.api_key, "sk-abc");
        assert_eq!(c.model, "gpt-5.5");

        // Local and custom keep what the user chose.
        let mut local = sample(Provider::Ollama, "qwen3");
        local.driver = Driver::Anthropic;
        local.base_url = " http://10.0.0.2:11434 ".into();
        let local = canonicalize(local);
        assert_eq!(local.driver, Driver::Anthropic);
        assert_eq!(local.base_url, "http://10.0.0.2:11434");
    }

    #[test]
    fn default_config_is_v2_with_no_connections() {
        let cfg = default_config();
        assert_eq!(cfg.version, CONFIG_VERSION);
        assert_eq!((cfg.default_source.as_str(), cfg.default_target.as_str()), ("zh", "en"));
        assert_eq!(cfg.translation, None);
        assert_eq!(cfg.web_search, WebSearch::Off);
        assert!(cfg.provider_keys.is_empty());
        assert_eq!(cfg.default_workdir, None);
    }
}
