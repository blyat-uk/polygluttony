//! Application configuration model. Persisted via the Tauri store plugin
//! (`store.rs`); these types define the schema and generate matching TS bindings.
//!
//! v2 is organised around two jobs instead of a named list: one **translation**
//! connection (required) and an optional **web search** role that either reuses
//! it or points at a separate connection. v1 configs are migrated on load.

pub mod languages;
pub mod projects;
pub mod providers;
pub mod store;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Schema version written to disk. A stored value without `version` is v1.
pub const CONFIG_VERSION: u32 = 2;

/// Which service a connection talks to. Drives the curated models, the
/// capability rules (`providers::capabilities`) and the driver/base URL defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum Provider {
    Anthropic,
    Openai,
    Google,
    Ollama,
    Lmstudio,
    /// Any OpenAI- or Anthropic-compatible URL; the API format is detected.
    Custom,
}

impl Provider {
    /// The provider's wire id — also the key into `AppConfig::provider_keys`.
    pub fn id(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::Openai => "openai",
            Provider::Google => "google",
            Provider::Ollama => "ollama",
            Provider::Lmstudio => "lmstudio",
            Provider::Custom => "custom",
        }
    }

    /// Hosted APIs with a fixed endpoint, driver and an account key. Their keys
    /// are remembered across connections; their base URL isn't user-editable.
    pub fn is_cloud(self) -> bool {
        matches!(self, Provider::Anthropic | Provider::Openai | Provider::Google)
    }

    /// Servers running on the user's machine, which need no key.
    pub fn is_local(self) -> bool {
        matches!(self, Provider::Ollama | Provider::Lmstudio)
    }
}

/// LLM API driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum Driver {
    /// Anthropic Messages API / Anthropic-compatible.
    Anthropic,
    /// OpenAI Chat Completions / OpenRouter / local (Ollama, LM Studio).
    Openai,
    /// OpenAI Responses API (reasoning effort, web search).
    OpenaiResponses,
    /// Native Gemini API (thinking config, Google Search grounding).
    Gemini,
}

/// How hard the model thinks. Which levels exist depends on the model
/// (`providers::capabilities`); the request shape on the provider
/// (`providers::thinking_param`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum ThinkingLevel {
    Off,
    Low,
    Medium,
    High,
}

/// A single LLM connection (the translation one, or a separate web-search one).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct Connection {
    pub provider: Provider,
    pub driver: Driver,
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    pub model: String,
    /// `None` = the model's own default (nothing sent).
    #[serde(default)]
    pub thinking: Option<ThinkingLevel>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub batch_dialogue_limit: Option<u32>,
    #[serde(default)]
    pub timeout: Option<u32>,
    #[serde(default)]
    pub connect_timeout: Option<u32>,
    #[serde(default)]
    pub concurrency: Option<u32>,
    /// Runtime only — attach the provider's web-search tool. Set by
    /// `store::resolve_web_search`; never persisted, never sent to the UI.
    #[serde(skip)]
    #[ts(skip)]
    pub web_search: bool,
}

/// Which connection, if any, answers the glossary's "look up established
/// names online" call.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "mode", rename_all = "kebab-case")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum WebSearch {
    #[default]
    Off,
    /// The translation connection also searches.
    Translation,
    /// A dedicated search-capable connection.
    Separate { connection: Connection },
}

/// Top-level persisted configuration (v2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct AppConfig {
    pub version: u32,
    pub default_source: String,
    pub default_target: String,
    #[serde(default)]
    pub default_workdir: Option<String>,
    #[serde(default)]
    pub translation: Option<Connection>,
    #[serde(default)]
    pub web_search: WebSearch,
    /// Remembered API keys by provider id ("anthropic" | "openai" | "google").
    #[serde(default)]
    pub provider_keys: BTreeMap<String, String>,
}

/// Test fixture: a keyed connection on `provider`'s default driver and base URL,
/// tuning unset. Tests tweak the fields they care about.
#[cfg(test)]
pub(crate) fn sample(provider: Provider, model: &str) -> Connection {
    let info = providers::provider_info(provider);
    Connection {
        provider,
        driver: info.driver.unwrap_or(Driver::Openai),
        base_url: info.base_url,
        api_key: "k".into(),
        model: model.into(),
        thinking: None,
        max_tokens: None,
        batch_dialogue_limit: None,
        timeout: None,
        connect_timeout: None,
        concurrency: None,
        web_search: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_use_kebab_case_wire_names() {
        assert_eq!(serde_json::to_value(Provider::Lmstudio).unwrap(), "lmstudio");
        assert_eq!(serde_json::to_value(Driver::OpenaiResponses).unwrap(), "openai-responses");
        assert_eq!(serde_json::to_value(Driver::Gemini).unwrap(), "gemini");
        assert_eq!(serde_json::to_value(ThinkingLevel::Medium).unwrap(), "medium");
        for p in [Provider::Anthropic, Provider::Openai, Provider::Google, Provider::Ollama,
                  Provider::Lmstudio, Provider::Custom] {
            assert_eq!(serde_json::to_value(p).unwrap(), p.id());
        }
    }

    #[test]
    fn runtime_web_search_flag_is_never_serialized_or_read() {
        let mut c = sample(Provider::Openai, "gpt-5.5");
        c.web_search = true;
        let v = serde_json::to_value(&c).unwrap();
        assert!(v.get("web_search").is_none());
        // A stray persisted flag can't switch the tool on either.
        let mut tampered = v.clone();
        tampered["web_search"] = serde_json::json!(true);
        let back: Connection = serde_json::from_value(tampered).unwrap();
        assert!(!back.web_search);
    }

    #[test]
    fn web_search_is_tagged_by_mode() {
        assert_eq!(serde_json::to_value(WebSearch::Off).unwrap(), serde_json::json!({"mode": "off"}));
        assert_eq!(
            serde_json::to_value(WebSearch::Translation).unwrap(),
            serde_json::json!({"mode": "translation"})
        );
        let sep = WebSearch::Separate { connection: sample(Provider::Google, "gemini-2.5-pro") };
        let v = serde_json::to_value(&sep).unwrap();
        assert_eq!(v["mode"], "separate");
        assert_eq!(v["connection"]["provider"], "google");
        assert_eq!(serde_json::from_value::<WebSearch>(v).unwrap(), sep);
    }

    #[test]
    fn connection_optional_fields_default_when_absent() {
        let c: Connection = serde_json::from_str(
            r#"{"provider":"ollama","driver":"openai","base_url":"u","model":"m"}"#,
        )
        .unwrap();
        assert_eq!(c.api_key, "");
        assert_eq!(c.thinking, None);
        assert_eq!(c.max_tokens, None);
    }
}
