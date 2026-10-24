//! O16/O17 commands. Thin wrappers — the engine lives in `translation::run`;
//! the live episode board in `translation::live`.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::LazyLock;

use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

use crate::config::projects::Tone;
use crate::error::{AppError, AppResult};
use crate::events::EpisodeView;
use crate::translation::live::LiveBoard;
use crate::translation::results::{FolderResults, TermLines};
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
    resume: Option<bool>,
) -> AppResult<()> {
    let resume = resume.unwrap_or(false);
    run::start(app, StartArgs { folder, files, tone, source_lang, target_lang, resume }).await
}

#[tauri::command]
pub async fn cancel_translation(app: AppHandle) -> AppResult<()> {
    run::cancel(app).await
}

/// One episode of the current or last run, for its live window. A waiting
/// episode's source is read from disk, so that happens on a blocking thread.
/// With `folder`, an episode that isn't in the run comes back `stored`, read
/// from the stored results.
// CONTRACT STUB: `folder` is ignored until stored results land.
#[tauri::command]
pub async fn episode_snapshot(
    app: AppHandle,
    file: String,
    folder: Option<String>,
) -> AppResult<EpisodeView> {
    let _ = folder;
    tauri::async_runtime::spawn_blocking(move || app.state::<LiveBoard>().snapshot(&file))
        .await
        .map_err(|e| AppError::Other(e.to_string()))
}

/// Held from "is it open?" to "it's built": the label only registers once the
/// window exists, so two quick opens (a double-clicked button) would otherwise
/// both build one.
static OPENING_EPISODE_WINDOW: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

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
/// `folder` lets it open a stored episode outside a run; `line` scrolls to
/// (and selects) that line.
// CONTRACT STUB: `line` on an already-open window is ignored for now.
#[tauri::command]
pub async fn open_episode_window(
    app: AppHandle,
    file: String,
    folder: Option<String>,
    line: Option<u32>,
) -> AppResult<()> {
    let window_err =
        |e: tauri::Error| AppError::Other(format!("couldn't open the episode window: {e}"));
    let label = episode_window_label(&file);
    let _opening = OPENING_EPISODE_WINDOW.lock().await;
    if let Some(window) = app.get_webview_window(&label) {
        window.unminimize().map_err(window_err)?;
        window.show().map_err(window_err)?;
        window.set_focus().map_err(window_err)?;
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(&app, label, tauri::WebviewUrl::App("episode.html".into()))
        .initialization_script(format!(
            "window.__EPISODE_FILE__ = {}; window.__EPISODE_FOLDER__ = {}; window.__EPISODE_LINE__ = {};",
            serde_json::to_string(&file)?,
            serde_json::to_string(&folder)?,
            serde_json::to_string(&line)?
        ))
        .title(format!("{file} — Polygluttony"))
        .inner_size(1100.0, 760.0)
        .min_inner_size(720.0, 480.0)
        .build()
        .map_err(window_err)?;
    Ok(())
}

/// Every stored episode result for a folder.
#[tauri::command]
pub fn get_folder_results(app: AppHandle, folder: String) -> AppResult<FolderResults> {
    let _ = (app, folder);
    Ok(FolderResults::default())
}

/// Retranslate just these lines of a finished episode (with their
/// neighbours as context) and write the output. Runs like a small run:
/// events on `TRANSLATION_EVENT`, exclusive with other runs.
#[tauri::command]
pub async fn retranslate_lines(
    app: AppHandle,
    folder: String,
    file: String,
    line_ids: Vec<u32>,
) -> AppResult<()> {
    let _ = (app, folder, file, line_ids);
    Err(AppError::Other("not implemented yet".into()))
}

/// Save the user's own translation of one line into the output file.
#[tauri::command]
pub fn save_line(
    app: AppHandle,
    folder: String,
    file: String,
    line_id: u32,
    text: String,
) -> AppResult<()> {
    let _ = (app, folder, file, line_id, text);
    Err(AppError::Other("not implemented yet".into()))
}

/// "Looks fine": drop one issue from the stored results.
#[tauri::command]
pub fn dismiss_issue(
    app: AppHandle,
    folder: String,
    file: String,
    line_id: u32,
    issue_type: String,
) -> AppResult<()> {
    let _ = (app, folder, file, line_id, issue_type);
    Err(AppError::Other("not implemented yet".into()))
}

/// "Mark episode done": rename the `.warning` output to the clean name and
/// record the episode as done.
#[tauri::command]
pub fn mark_episode_done(app: AppHandle, folder: String, file: String) -> AppResult<()> {
    let _ = (app, folder, file);
    Err(AppError::Other("not implemented yet".into()))
}

/// Lines of finished episodes whose source contains `source` (a glossary term).
#[tauri::command]
pub fn find_term_lines(app: AppHandle, folder: String, source: String) -> AppResult<Vec<TermLines>> {
    let _ = (app, folder, source);
    Ok(Vec::new())
}

/// Retranslate every line of finished episodes that uses a glossary term.
/// Returns how many lines were queued.
#[tauri::command]
pub async fn retranslate_term(app: AppHandle, folder: String, source: String) -> AppResult<u32> {
    let _ = (app, folder, source);
    Err(AppError::Other("not implemented yet".into()))
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
