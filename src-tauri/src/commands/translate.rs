//! O16/O17 commands. Thin wrappers — the engine lives in `translation::run`;
//! the live episode board in `translation::live`.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Mutex;

use crate::config::projects::Tone;
use crate::error::{AppError, AppResult};
use crate::events::{
    EpisodeChanged, EpisodeGoToLine, EpisodeView, ResultsChanged, EPISODE_CHANGED_EVENT,
    EPISODE_GO_TO_LINE_EVENT, RESULTS_CHANGED_EVENT,
};
use crate::models::language_pair::LanguagePair;
use crate::translation::live::{LiveBoard, Standing};
use crate::translation::results::{FolderResults, ResultsStore, TermLines};
use crate::translation::review;
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

/// One episode for its window. Without `folder`, the current or last run's
/// live board, as before. With `folder`: `live` while the episode is in the
/// run that is going now; otherwise `stored`, read from the stored results
/// (so Review edits show at once), falling back to the last run's board for
/// an episode with nothing stored; `not_in_run` when there's neither. Disk
/// reads happen on a blocking thread.
#[tauri::command]
pub async fn episode_snapshot(
    app: AppHandle,
    file: String,
    folder: Option<String>,
) -> AppResult<EpisodeView> {
    tauri::async_runtime::spawn_blocking(move || {
        let board = app.state::<LiveBoard>();
        let Some(folder) = folder else { return board.snapshot(&file) };
        let folder = PathBuf::from(folder);
        let standing = board.standing(&folder, &file);
        if standing == Standing::Running {
            return board.snapshot(&file);
        }
        let stored = ResultsStore::for_app(&app)
            .ok()
            .and_then(|store| review::stored_snapshot(&store, &folder, &file));
        match (stored, standing) {
            (Some(view), _) => view,
            (None, Standing::Finished) => board.snapshot(&file),
            (None, _) => EpisodeView::NotInRun,
        }
    })
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

/// Open an episode's window, or bring its open one to the front. `async`
/// because building a window from a sync command deadlocks on Windows.
/// `folder` lets it open a stored episode outside a run; `line` scrolls to
/// (and selects) that line — an open window gets it as
/// `EPISODE_GO_TO_LINE_EVENT`.
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
        if let Some(line) = line {
            app.emit_to(label.as_str(), EPISODE_GO_TO_LINE_EVENT, EpisodeGoToLine { file, line })
                .map_err(window_err)?;
        }
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

/// Every stored episode result for a folder. A missing or damaged store is
/// empty, never an error.
#[tauri::command]
pub fn get_folder_results(app: AppHandle, folder: String) -> AppResult<FolderResults> {
    Ok(ResultsStore::for_app(&app)
        .map(|store| store.results(Path::new(&folder)))
        .unwrap_or_default())
}

/// Review actions write the episode's output and stored lines; a run that is
/// translating the same episode would write over them.
fn not_running(app: &AppHandle, folder: &Path, file: &str) -> AppResult<()> {
    if app.state::<LiveBoard>().standing(folder, file) == Standing::Running {
        return Err(AppError::Other(
            "This episode is being translated right now. Wait for it to finish.".into(),
        ));
    }
    Ok(())
}

/// After a review action: windows refetch, and the episode list rereads the results.
fn changed(app: &AppHandle, folder: &str, file: &str) {
    let _ = app.emit(
        RESULTS_CHANGED_EVENT,
        ResultsChanged { folder: folder.to_string(), file: file.to_string() },
    );
    let _ = app.emit(EPISODE_CHANGED_EVENT, EpisodeChanged { file: Some(file.to_string()) });
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
    run::start_review(app, folder, vec![(file, line_ids)]).await
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
    let path = PathBuf::from(&folder);
    not_running(&app, &path, &file)?;
    review::save_line(&ResultsStore::for_app(&app)?, &path, &file, line_id, &text)?;
    changed(&app, &folder, &file);
    Ok(())
}

/// "Looks fine": drop one issue from the stored results. `term` picks the
/// name issue about that name; without it, every issue of that type on the
/// line goes.
#[tauri::command]
pub fn dismiss_issue(
    app: AppHandle,
    folder: String,
    file: String,
    line_id: u32,
    issue_type: String,
    term: Option<String>,
) -> AppResult<()> {
    let path = PathBuf::from(&folder);
    not_running(&app, &path, &file)?;
    let store = ResultsStore::for_app(&app)?;
    review::dismiss_issue(&store, &path, &file, line_id, &issue_type, term.as_deref())?;
    changed(&app, &folder, &file);
    Ok(())
}

/// "Mark episode done": rename the `.warning` output to the clean name and
/// record the episode as done.
#[tauri::command]
pub fn mark_episode_done(app: AppHandle, folder: String, file: String) -> AppResult<()> {
    let path = PathBuf::from(&folder);
    not_running(&app, &path, &file)?;
    // For an episode translated before results were stored: the folder's languages.
    let fallback = crate::config::projects::load(&app)
        .ok()
        .and_then(|cfg| crate::config::projects::get_prefs(&cfg, &folder))
        .and_then(|p| LanguagePair::from_codes(&p.source_lang, &p.target_lang).ok());
    review::mark_episode_done(&ResultsStore::for_app(&app)?, &path, &file, fallback)?;
    changed(&app, &folder, &file);
    Ok(())
}

/// Lines of finished episodes whose source contains `source` (a glossary term).
#[tauri::command]
pub fn find_term_lines(app: AppHandle, folder: String, source: String) -> AppResult<Vec<TermLines>> {
    let store = ResultsStore::for_app(&app)?;
    Ok(review::find_term_lines(&store, Path::new(&folder), &source))
}

/// Retranslate every line of finished episodes that uses a glossary term,
/// as one review run. Returns how many lines were queued (0: no run).
#[tauri::command]
pub async fn retranslate_term(app: AppHandle, folder: String, source: String) -> AppResult<u32> {
    let store = ResultsStore::for_app(&app)?;
    let found = review::find_term_lines(&store, Path::new(&folder), &source);
    let count = found.iter().map(|t| t.line_ids.len() as u32).sum();
    if count == 0 {
        return Ok(0);
    }
    let jobs = found.into_iter().map(|t| (t.file, t.line_ids)).collect();
    run::start_review(app, folder, jobs).await?;
    Ok(count)
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
