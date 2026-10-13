//! Application configuration model. Persisted via the Tauri store plugin
//! (`store.rs`); these types define the schema and generate matching TS bindings.
//!
//! v2 is a small **model library** plus the five LLM **steps** (extract,
//! normalize, web search, translate, verify), each pointing at a library model
//! with its own Thinking and Tuning. By default one model does every step.
//! v1 configs (and pre-release v2 ones) are migrated on load.

pub mod languages;
pub mod projects;
pub mod providers;
pub mod store;
pub mod transfer;

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
    /// are remembered across models; their base URL isn't user-editable.
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

/// A model in the library: where to reach it. How each step uses it lives
/// on `StepSettings`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct Model {
    /// Stable id that steps refer to. Empty on a new model: the backend assigns one.
    pub id: String,
    pub provider: Provider,
    pub driver: Driver,
    pub base_url: String,
    /// Cloud providers: always the provider's remembered key (one key per provider).
    #[serde(default)]
    pub api_key: String,
    pub model: String,
    /// Index into the UI's model palette, fixed when the model joins the library.
    #[serde(default)]
    pub color: u8,
    #[serde(default)]
    pub connect_timeout: Option<u32>,
}

impl Model {
    /// The name people know the model by ("Claude Sonnet 5.5"), or its raw id.
    pub fn label(&self) -> String {
        providers::model_label(self.provider, &self.model)
    }

    /// Whether the provider's web-search tool works with this model.
    pub fn can_search(&self) -> bool {
        providers::capabilities(self.provider, &self.model).web_search
    }
}

/// A job an LLM does, in workflow order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum Step {
    Extract,
    Normalize,
    WebSearch,
    Translate,
    Verify,
}

impl Step {
    /// Every step, in workflow order.
    pub const ALL: [Step; 5] =
        [Step::Extract, Step::Normalize, Step::WebSearch, Step::Translate, Step::Verify];

    /// The steps that are always on — and that share one model and one set of
    /// settings while the page shows a single model (`AppConfig::per_step`).
    pub const TEXT: [Step; 4] = [Step::Extract, Step::Normalize, Step::Translate, Step::Verify];

    /// The step's name on the Connections page, so a message can say where to
    /// fix it.
    pub fn label(self) -> &'static str {
        match self {
            Step::Extract => "Extract names",
            Step::Normalize => "Normalize",
            Step::WebSearch => "Web search",
            Step::Translate => "Translate",
            Step::Verify => "Verify",
        }
    }
}

/// How one step uses a library model. Unset tuning follows the provider's defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct StepSettings {
    /// `Model::id`.
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
    pub concurrency: Option<u32>,
}

impl StepSettings {
    /// Fresh settings on `model`: its own Thinking default (normalisation fills
    /// it in) and the provider's tuning.
    pub fn new(model: impl Into<String>) -> Self {
        StepSettings {
            model: model.into(),
            thinking: None,
            max_tokens: None,
            batch_dialogue_limit: None,
            timeout: None,
            concurrency: None,
        }
    }
}

/// Which model does each step, and how.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct Steps {
    pub extract: StepSettings,
    pub normalize: StepSettings,
    /// `None` = web search off.
    #[serde(default)]
    pub web_search: Option<StepSettings>,
    pub translate: StepSettings,
    pub verify: StepSettings,
}

impl Steps {
    /// Every text step on `text`; web search on `web_search` (`None` = off).
    pub fn all(text: StepSettings, web_search: Option<StepSettings>) -> Steps {
        Steps {
            extract: text.clone(),
            normalize: text.clone(),
            web_search,
            translate: text.clone(),
            verify: text,
        }
    }

    /// The step's settings; `None` only for web search while it's off.
    pub fn get(&self, step: Step) -> Option<&StepSettings> {
        match step {
            Step::Extract => Some(&self.extract),
            Step::Normalize => Some(&self.normalize),
            Step::WebSearch => self.web_search.as_ref(),
            Step::Translate => Some(&self.translate),
            Step::Verify => Some(&self.verify),
        }
    }

    pub fn get_mut(&mut self, step: Step) -> Option<&mut StepSettings> {
        match step {
            Step::Extract => Some(&mut self.extract),
            Step::Normalize => Some(&mut self.normalize),
            Step::WebSearch => self.web_search.as_mut(),
            Step::Translate => Some(&mut self.translate),
            Step::Verify => Some(&mut self.verify),
        }
    }

    /// Replace one step. Only web search can be switched off; `None` for a
    /// text step leaves it as it is (callers reject that first).
    pub fn set(&mut self, step: Step, settings: Option<StepSettings>) {
        match (step, settings) {
            (Step::WebSearch, settings) => self.web_search = settings,
            (step, Some(settings)) => {
                if let Some(slot) = self.get_mut(step) {
                    *slot = settings;
                }
            }
            (_, None) => {}
        }
    }

    /// The settings of every step that is on, in workflow order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut StepSettings> {
        [
            Some(&mut self.extract),
            Some(&mut self.normalize),
            self.web_search.as_mut(),
            Some(&mut self.translate),
            Some(&mut self.verify),
        ]
        .into_iter()
        .flatten()
    }

    /// Whether `model_id` does any step.
    pub fn uses(&self, model_id: &str) -> bool {
        Step::ALL.into_iter().filter_map(|s| self.get(s)).any(|s| s.model == model_id)
    }

    /// The four text steps share one model and identical settings — what the
    /// one-model view assumes.
    pub fn text_steps_shared(&self) -> bool {
        [&self.normalize, &self.translate, &self.verify].into_iter().all(|s| *s == self.extract)
    }
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
    /// The model library. Empty = nothing set up yet.
    #[serde(default)]
    pub models: Vec<Model>,
    /// `None` exactly when `models` is empty.
    #[serde(default)]
    pub steps: Option<Steps>,
    /// The Connections page shows every step (true) or one shared model (false).
    /// False implies extract, normalize, translate and verify share identical settings.
    #[serde(default)]
    pub per_step: bool,
    /// Remembered API keys by provider id ("anthropic" | "openai" | "google").
    #[serde(default)]
    pub provider_keys: BTreeMap<String, String>,
}

/// What a driver talks to: a library model merged with one step's settings
/// (`store::step_connection`). Runtime only — never persisted and never sent
/// to the UI, so the drivers keep one flat struct while the config stores
/// `Model` + `StepSettings`.
#[derive(Debug, Clone, PartialEq)]
pub struct Connection {
    pub provider: Provider,
    pub driver: Driver,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// `None` = the model's own default (nothing sent).
    pub thinking: Option<ThinkingLevel>,
    pub max_tokens: Option<u32>,
    pub batch_dialogue_limit: Option<u32>,
    pub timeout: Option<u32>,
    pub connect_timeout: Option<u32>,
    pub concurrency: Option<u32>,
    /// Attach the provider's web-search tool. Set by `store::resolve_web_search`.
    pub web_search: bool,
}

impl Connection {
    /// `model` run with `settings`, as stored: unset tuning stays unset
    /// (`store::step_connection` fills it in) and no web-search tool.
    pub fn from_step(model: &Model, settings: &StepSettings) -> Connection {
        Connection {
            provider: model.provider,
            driver: model.driver,
            base_url: model.base_url.clone(),
            api_key: model.api_key.clone(),
            model: model.model.clone(),
            thinking: settings.thinking,
            max_tokens: settings.max_tokens,
            batch_dialogue_limit: settings.batch_dialogue_limit,
            timeout: settings.timeout,
            connect_timeout: model.connect_timeout,
            concurrency: settings.concurrency,
            web_search: false,
        }
    }

    /// The inverse of `from_step`: the library entry (with `id` and `color`)
    /// and the step settings this connection stood for. Migration turns the
    /// old role connections into library models with it.
    pub fn split(self, id: &str, color: u8) -> (Model, StepSettings) {
        let settings = StepSettings {
            model: id.to_string(),
            thinking: self.thinking,
            max_tokens: self.max_tokens,
            batch_dialogue_limit: self.batch_dialogue_limit,
            timeout: self.timeout,
            concurrency: self.concurrency,
        };
        let model = Model {
            id: id.to_string(),
            provider: self.provider,
            driver: self.driver,
            base_url: self.base_url,
            api_key: self.api_key,
            model: self.model,
            color,
            connect_timeout: self.connect_timeout,
        };
        (model, settings)
    }
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

/// Test fixture: a new (id-less) keyed library model on `provider`'s default
/// driver and base URL.
#[cfg(test)]
pub(crate) fn sample_model(provider: Provider, model: &str) -> Model {
    sample(provider, model).split("", 0).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn enums_use_kebab_case_wire_names() {
        assert_eq!(serde_json::to_value(Provider::Lmstudio).unwrap(), "lmstudio");
        assert_eq!(serde_json::to_value(Driver::OpenaiResponses).unwrap(), "openai-responses");
        assert_eq!(serde_json::to_value(Driver::Gemini).unwrap(), "gemini");
        assert_eq!(serde_json::to_value(ThinkingLevel::Medium).unwrap(), "medium");
        assert_eq!(serde_json::to_value(Step::WebSearch).unwrap(), "web-search");
        for p in [Provider::Anthropic, Provider::Openai, Provider::Google, Provider::Ollama,
                  Provider::Lmstudio, Provider::Custom] {
            assert_eq!(serde_json::to_value(p).unwrap(), p.id());
        }
    }

    #[test]
    fn steps_come_in_workflow_order_with_their_page_names() {
        let names: Vec<&str> = Step::ALL.iter().map(|s| s.label()).collect();
        assert_eq!(names, ["Extract names", "Normalize", "Web search", "Translate", "Verify"]);
        assert!(Step::ALL.windows(2).all(|w| w[0] < w[1]));
        assert!(!Step::TEXT.contains(&Step::WebSearch));
    }

    #[test]
    fn model_optional_fields_default_when_absent() {
        let m: Model = serde_json::from_value(json!({
            "id": "m1", "provider": "ollama", "driver": "openai", "base_url": "u", "model": "qwen3"
        }))
        .unwrap();
        assert_eq!((m.api_key.as_str(), m.color, m.connect_timeout), ("", 0, None));
        let s: StepSettings = serde_json::from_value(json!({ "model": "m1" })).unwrap();
        assert_eq!(s, StepSettings::new("m1"));
        let steps: Steps = serde_json::from_value(json!({
            "extract": s, "normalize": s, "translate": s, "verify": s
        }))
        .unwrap();
        assert_eq!(steps.web_search, None, "absent web search is off");
    }

    #[test]
    fn steps_accessors_cover_every_step() {
        let mut steps = Steps::all(StepSettings::new("a"), None);
        assert!(steps.get(Step::WebSearch).is_none());
        assert_eq!(steps.iter_mut().count(), 4);
        steps.set(Step::WebSearch, Some(StepSettings::new("b")));
        steps.set(Step::Verify, Some(StepSettings::new("c")));
        steps.set(Step::Translate, None); // text steps can't be switched off
        let ids: Vec<&str> = Step::ALL.iter().map(|s| steps.get(*s).unwrap().model.as_str()).collect();
        assert_eq!(ids, ["a", "a", "b", "a", "c"]);
        assert!(steps.uses("b") && steps.uses("c") && !steps.uses("d"));
        assert!(!steps.text_steps_shared());
        steps.set(Step::Verify, Some(StepSettings::new("a")));
        assert!(steps.text_steps_shared(), "web search doesn't count");
    }

    #[test]
    fn a_connection_splits_into_the_model_and_step_it_came_from() {
        let mut c = sample(Provider::Anthropic, "claude-opus-5-5");
        c.thinking = Some(ThinkingLevel::High);
        c.timeout = Some(90);
        c.connect_timeout = Some(5);
        let (model, settings) = c.clone().split("m2", 3);
        assert_eq!((model.id.as_str(), model.color, model.connect_timeout), ("m2", 3, Some(5)));
        assert_eq!((settings.model.as_str(), settings.thinking, settings.timeout), ("m2", Some(ThinkingLevel::High), Some(90)));
        assert_eq!(Connection::from_step(&model, &settings), c);
    }
}
