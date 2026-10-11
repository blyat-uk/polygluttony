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

    #[error("no usable connection — configure one in Connections")]
    NoActiveConnection,

    /// A step's model can't make requests: it has no API key. Names the step
    /// as the Connections page does, so the user knows which row to fix.
    #[error("{step} uses {model}, which needs an API key. Add it on the Connections page.")]
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
