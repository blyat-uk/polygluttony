//! Glossary commands (O9–O15). Thin wrappers — the engine lives in `glossary::*`.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use crate::config::store as config_store;
use crate::config::Step;
use crate::error::{AppError, AppResult};
use crate::glossary::build::BuildMode;
use crate::glossary::diff::GlossaryDiff;
use crate::glossary::io::{
    import_glossary_file, load_folder_glossary, save_folder_glossary, GlossaryBackupInfo,
};
use crate::glossary::meta::{
    GlossaryChangeKind, GlossaryLoad, GlossaryMeta, GlossaryScope, GlossarySnapshot,
};
use crate::glossary::model::{Glossary, GlossaryDoc};
use crate::glossary::normalize::{normalize_pass, NormalizeReview};
use crate::glossary::reference::{self, ReferenceStatus, ReferenceSummary, ReferenceTerminology};
use crate::glossary::run::{self, GlossaryOpKind, StartArgs};
use crate::glossary::watch::{self, GlossaryWatchState};
use crate::glossary::world_detector::WorldType;

/// O9 — the folder's glossary and its metadata; `missing` when there's no
/// glossary.json, `unreadable` (with the line, when known) when it isn't valid.
// CONTRACT STUB: meta is always the default; loading the sidecar is the
// glossary engine's job.
#[tauri::command]
pub fn load_glossary(folder: String) -> GlossaryLoad {
    let path = PathBuf::from(&folder).join("glossary.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return GlossaryLoad::Missing;
    };
    if let Err(e) = serde_json::from_str::<serde_json::Value>(&text) {
        return GlossaryLoad::Unreadable { message: e.to_string(), line: Some(e.line() as u32) };
    }
    match Glossary::from_json(&text) {
        Some(g) => GlossaryLoad::Ok { doc: GlossaryDoc::from(&g), meta: GlossaryMeta::default() },
        None => GlossaryLoad::Unreadable {
            message: "glossary.json isn't in the expected format".into(),
            line: None,
        },
    }
}

/// O14 — atomic write; the UI auto-saves every term edit. `meta: None` leaves
/// the sidecar as it is. `snapshot: Some(kind)` first saves the current
/// glossary to the history, labelled with the change about to happen.
// CONTRACT STUB: meta and snapshot are ignored until the glossary engine lands.
#[tauri::command]
pub fn save_glossary(
    folder: String,
    doc: GlossaryDoc,
    meta: Option<GlossaryMeta>,
    snapshot: Option<GlossaryChangeKind>,
) -> AppResult<()> {
    let _ = (meta, snapshot);
    save_folder_glossary(&PathBuf::from(folder), &doc.into_glossary())
}

// JS sends camelCase keys (worldType, sourceLang, …) — the macro converts; see
// commands/translate.rs:9-15.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn start_glossary_build(
    app: AppHandle,
    folder: String,
    mode: BuildMode,
    files: Vec<String>,
    world_type: WorldType,
    source_lang: String,
    target_lang: String,
    normalize: bool,
    personalize: bool,
    personalize_context: String,
) -> AppResult<()> {
    run::start(
        app,
        StartArgs {
            folder,
            mode,
            files,
            world_type,
            source_lang,
            target_lang,
            normalize,
            personalize,
            personalize_context,
        },
    )
    .await
}

/// Cancels the active glossary op (build / normalize / import).
#[tauri::command]
pub async fn cancel_glossary_build(app: AppHandle) -> AppResult<()> {
    run::cancel(app).await
}

/// O12 — returns the review WITHOUT saving; the UI saves on accept via
/// `save_glossary`. Claims the op slot (exclusive with build/translation).
///
/// Cancel semantics (for the UI task): a user cancel mid-normalize makes the
/// in-flight category requests fail, so the originals are kept and this
/// command still resolves Ok with a no-changes review — the UI must discard
/// the resolved review after a user-cancel instead of presenting it.
#[tauri::command]
pub async fn normalize_glossary(
    app: AppHandle,
    folder: String,
    scope: Option<GlossaryScope>,
) -> AppResult<NormalizeReview> {
    // CONTRACT STUB: scope (None = all) is ignored until the glossary engine lands.
    let _ = scope;
    let dir = PathBuf::from(&folder);
    let original = load_folder_glossary(&dir)
        .ok_or_else(|| AppError::Other("no glossary to normalize".into()))?;
    if original.is_empty() {
        return Err(AppError::Other("glossary is empty".into()));
    }
    let cfg = config_store::load(&app)?;
    let conn = config_store::step_connection(&cfg, Step::Normalize)?;
    let templates =
        crate::prompts::GlossaryPrompts::resolve_normalize(&crate::prompts::overrides_dir(&app)?)?;
    let cancel = run::claim_slot(&app, GlossaryOpKind::Normalize).await?;
    // RAII: the guard releases the slot on every exit path, including panics.
    // Declared before svc/tx so it drops last (after their senders close).
    let _guard = run::SlotGuard::new(app.clone());
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    run::spawn_forwarder(app.clone(), rx);
    let svc = run::service_for(&conn, cancel.clone(), tx.clone());
    let normalized = normalize_pass(&svc, &original, &tx, &templates).await;
    Ok(NormalizeReview {
        diff: GlossaryDiff::compute(Some(&original), &normalized),
        normalized: GlossaryDoc::from(&normalized),
    })
}

/// Look up names online on the glossary as it is — no subtitle files are
/// read. Returns a review; nothing is saved. `scope: None` = all terms.
#[tauri::command]
pub async fn lookup_glossary_names(
    app: AppHandle,
    folder: String,
    show_name: String,
    scope: Option<GlossaryScope>,
) -> AppResult<NormalizeReview> {
    let _ = (app, folder, show_name, scope);
    Err(AppError::Other("not implemented yet".into()))
}

/// The glossary's version history, newest first.
#[tauri::command]
pub fn list_glossary_snapshots(folder: String) -> AppResult<Vec<GlossarySnapshot>> {
    let _ = folder;
    Ok(Vec::new())
}

/// Bring back a snapshot. The current glossary is snapshotted first
/// (reason `restore`), so a restore can itself be undone.
#[tauri::command]
pub fn restore_glossary_snapshot(folder: String, id: String) -> AppResult<()> {
    let _ = (folder, id);
    Err(AppError::Other("not implemented yet".into()))
}

/// O11 — extract reference terms from user-picked translated `.ass` files and
/// cache them; subsequent builds pick the cache up automatically.
#[tauri::command]
pub async fn import_reference_files(
    app: AppHandle,
    folder: String,
    paths: Vec<String>,
) -> AppResult<ReferenceSummary> {
    if paths.is_empty() {
        return Err(AppError::Other("no files selected".into()));
    }
    let dir = PathBuf::from(&folder);
    let cfg = config_store::load(&app)?;
    // Reading reference files is the Extract step's job.
    let conn = config_store::step_connection(&cfg, Step::Extract)?;
    let reference_template = crate::prompts::resolve(
        crate::prompts::PromptId::ReferenceExtract,
        &crate::prompts::overrides_dir(&app)?,
    )?;
    let cancel = run::claim_slot(&app, GlossaryOpKind::Import).await?;
    // RAII: the guard releases the slot on every exit path, including panics.
    // Declared before svc/tx so it drops last (after their senders close).
    let _guard = run::SlotGuard::new(app.clone());
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    run::spawn_forwarder(app.clone(), rx);
    let svc = run::service_for(&conn, cancel.clone(), tx.clone());
    let files: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let (terms, files_processed, errors) =
        reference::extract_from_files(&svc, &files, conn.batch_dialogue_limit, &tx, &reference_template).await;
    let count = terms.count() as u32;
    if count > 0 {
        reference::save_cache(&dir, &terms)?;
    }
    Ok(ReferenceSummary { count, files_processed, cancelled: cancel.is_cancelled(), errors })
}

#[tauri::command]
pub fn reference_status(folder: String) -> ReferenceStatus {
    reference::reference_status(&PathBuf::from(folder))
}

/// What `glossary.prev.json` holds right now (None = no backup yet). Powers
/// the "you are about to replace this backup" disclosure on the destructive
/// paths. A read — no slot claim.
#[tauri::command]
pub fn glossary_backup_status(folder: String) -> Option<GlossaryBackupInfo> {
    crate::glossary::io::backup_status(&PathBuf::from(folder))
}

#[tauri::command]
pub fn clear_reference(folder: String) -> AppResult<()> {
    reference::clear_cache(&PathBuf::from(folder))
}

/// Load the cached reference terminology for the review screen (None = no cache).
#[tauri::command]
pub fn load_reference(folder: String) -> Option<ReferenceTerminology> {
    reference::load_cache(&PathBuf::from(folder))
}

/// Persist review-screen edits (term pruning). Saving an empty terminology is
/// allowed and keeps the file — explicit Clear deletes it via `clear_reference`.
#[tauri::command]
pub fn save_reference(folder: String, terms: ReferenceTerminology) -> AppResult<()> {
    reference::save_cache(&PathBuf::from(folder), &terms)
}

/// Install a picked `glossary.json` as this folder's glossary, backing up any
/// existing one to `glossary.prev.json`. Returns the imported term count.
/// Claims the glossary-op slot — mutually exclusive with build, normalize,
/// and reference import.
#[tauri::command]
pub async fn import_glossary(app: AppHandle, folder: String, src: String) -> AppResult<u32> {
    run::claim_slot(&app, GlossaryOpKind::Import).await?;
    // RAII: the guard releases the slot on every exit path, including panics.
    let _guard = run::SlotGuard::new(app.clone());
    import_glossary_file(&PathBuf::from(folder), &PathBuf::from(src))
}

/// Plain file copy; the UI supplies `dest` from a save dialog.
#[tauri::command]
pub fn export_glossary(folder: String, dest: String) -> AppResult<()> {
    let src = PathBuf::from(folder).join("glossary.json");
    if !src.is_file() {
        return Err(AppError::Other("no glossary.json to export".into()));
    }
    // fs::copy truncates dest before reading, so dest == src would zero the
    // glossary. canonicalize fails for a non-existent dest — fine: a
    // non-existent dest can't be src.
    if let (Ok(d), Ok(s)) = (Path::new(&dest).canonicalize(), src.canonicalize()) {
        if d == s {
            return Err(AppError::Other("export destination is the glossary itself".into()));
        }
    }
    std::fs::copy(src, dest)?;
    Ok(())
}

/// O15 — open glossary.json with the OS default editor.
#[tauri::command]
pub fn open_glossary_editor(app: AppHandle, folder: String) -> AppResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let path = PathBuf::from(folder).join("glossary.json");
    if !path.is_file() {
        return Err(AppError::Other("no glossary.json yet".into()));
    }
    app.opener()
        .open_path(path.to_string_lossy().into_owned(), None::<String>)
        .map_err(|e| AppError::Other(e.to_string()))
}

#[tauri::command]
pub fn watch_glossary(app: AppHandle, folder: String) -> AppResult<()> {
    let state = app.state::<GlossaryWatchState>();
    watch::watch(app.clone(), &state, &PathBuf::from(folder))
}

#[tauri::command]
pub fn unwatch_glossary(app: AppHandle) -> AppResult<()> {
    watch::unwatch(&app.state::<GlossaryWatchState>());
    Ok(())
}

/// Label of the model that answers web searches ("Claude Sonnet 5.5") — the
/// Web search step's, when it is on, usable and can search — else None.
/// Powers the "Look up established names online" checkbox gating.
#[tauri::command]
pub fn web_search_status(app: AppHandle) -> AppResult<Option<String>> {
    Ok(config_store::web_search_label(&config_store::load(&app)?))
}
