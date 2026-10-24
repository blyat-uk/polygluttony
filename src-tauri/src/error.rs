//! Error type returned from Tauri commands. Serializes to a plain string so it
//! surfaces cleanly as a rejected promise in the webview.

use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Http(#[from] reqwest::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Llm(#[from] crate::llm::error::LlmError),

    #[error("{0}")]
    Other(String),

    #[error("another operation is already running")]
    RunAlreadyActive,

    #[error("Connect a model first, on the Models page.")]
    NoActiveConnection,

    /// A step's model can't make requests: it has no API key. Names the step
    /// as the Models page does, so the user knows which row to fix.
    #[error("{step} uses {model}, which needs an API key. Add it on the Models page.")]
    StepNeedsKey { step: &'static str, model: String },

    #[error("no active translation run")]
    NoActiveRun,
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

/// Convenience alias for command results.
pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_point_at_the_models_page_in_plain_words() {
        assert_eq!(AppError::NoActiveConnection.to_string(), "Connect a model first, on the Models page.");
        let needs_key = AppError::StepNeedsKey { step: "Tidy up", model: "Claude Opus 5.5".into() };
        assert_eq!(
            needs_key.to_string(),
            "Tidy up uses Claude Opus 5.5, which needs an API key. Add it on the Models page."
        );
        // Commands reject with the message itself, as a plain string.
        assert_eq!(
            serde_json::to_value(AppError::NoActiveConnection).unwrap(),
            "Connect a model first, on the Models page."
        );
    }
}
