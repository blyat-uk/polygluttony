//! Tauri command handlers — the entire surface the webview can call. Each
//! command is a thin wrapper that delegates to the engine modules.

pub mod connections;
pub use connections::*;

pub mod glossary;
pub use glossary::*;

pub mod project;
pub use project::*;

pub mod translate;
pub use translate::*;

pub mod prompts;
pub use prompts::*;

pub mod setup;
pub use setup::*;

use crate::error::{AppError, AppResult};
use crate::models::AppInfo;

/// Run disk work on a blocking thread: a sync command runs on the main
/// thread, and an async one must not stall the runtime's.
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| AppError::Other(format!("Something went wrong inside Polygluttony: {e}")))?
}

/// Returns app/core metadata. Used by the webview as a startup health check for
/// the IPC bridge.
#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}
