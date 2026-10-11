//! O16/O17 commands. Thin wrappers — the engine lives in `translation::run`;
//! the live episode board in `translation::live`.

use std::hash::{DefaultHasher, Hash, Hasher};

use tauri::{AppHandle, Manager};

use crate::config::projects::Tone;
use crate::error::{AppError, AppResult};
use crate::events::EpisodeView;
use crate::translation::live::LiveBoard;
use crate::translation::run::{self, StartArgs};

// JS-side key casing: Tauri v2's #[tauri::command] macro converts every Rust
// snake_case parameter name to lowerCamelCase before matching against the
// JSON payload — see tauri-macros/src/command/wrapper.rs lines 505-508
// (`ArgumentCase::Camel => { key = key.to_lower_camel_case(); }`), with
// `ArgumentCase::Camel` as the hard-coded default (line 51).
// Therefore JS MUST send camelCase keys: `sourceLang`, `targetLang`, etc.
// (Override with `#[tauri::command(rename_all = "snake_case")]` if desired.)
#[tauri::command]
pub async fn start_translation(
    app: AppHandle,
    folder: String,
    files: Vec<String>,
    tone: Tone,
    source_lang: String,
    target_lang: String,
) -> AppResult<()> {
    run::start(app, StartArgs { folder, files, tone, source_lang, target_lang }).await
}

#[tauri::command]
pub async fn cancel_translation(app: AppHandle) -> AppResult<()> {
    run::cancel(app).await
}

/// One episode of the current or last run, for its live window. `async` so a
/// waiting episode's disk read stays off the main thread.
#[tauri::command]
pub async fn episode_snapshot(app: AppHandle, file: String) -> AppResult<EpisodeView> {
    Ok(app.state::<LiveBoard>().snapshot(&file))
}

/// Window label for an episode. Labels allow only a few characters and a file
/// name may hold anything, so hash it; stable within a process, which is all
/// "focus the open window" needs.
fn episode_window_label(file: &str) -> String {
    let mut hasher = DefaultHasher::new();
    file.hash(&mut hasher);
    format!("episode-{:016x}", hasher.finish())
}

/// Open an episode's live window, or bring its open one to the front. `async`
/// because building a window from a sync command deadlocks on Windows.
#[tauri::command]
pub async fn open_episode_window(app: AppHandle, file: String) -> AppResult<()> {
    let window_err =
        |e: tauri::Error| AppError::Other(format!("couldn't open the episode window: {e}"));
    let label = episode_window_label(&file);
    if let Some(window) = app.get_webview_window(&label) {
        window.unminimize().map_err(window_err)?;
        window.show().map_err(window_err)?;
        window.set_focus().map_err(window_err)?;
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(&app, label, tauri::WebviewUrl::App("episode.html".into()))
        .initialization_script(format!(
            "window.__EPISODE_FILE__ = {};",
            serde_json::to_string(&file)?
        ))
        .title(format!("{file} — Polygluttony"))
        .inner_size(1100.0, 760.0)
        .min_inner_size(720.0, 480.0)
        .build()
        .map_err(window_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::episode_window_label;

    #[test]
    fn episode_window_label_is_stable_and_label_safe() {
        let label = episode_window_label("[Group] Show — 01 (1080p).ass");
        assert_eq!(label, episode_window_label("[Group] Show — 01 (1080p).ass"));
        assert_ne!(label, episode_window_label("[Group] Show — 02 (1080p).ass"));
        assert!(label.starts_with("episode-"));
        assert!(label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }
}
