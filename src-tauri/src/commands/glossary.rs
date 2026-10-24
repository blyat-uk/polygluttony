//! Glossary commands (O9–O15). Thin wrappers — the engine lives in `glossary::*`.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use crate::config::store as config_store;
use crate::config::Step;
use crate::error::{AppError, AppResult};
use crate::glossary::build::BuildMode;
use crate::glossary::history;
use crate::glossary::io::{
    import_glossary_file, load_folder_glossary, save_folder_glossary, GlossaryBackupInfo,
};
use crate::glossary::meta::{
    load_meta, save_meta, GlossaryChangeKind, GlossaryLoad, GlossaryMeta, GlossaryScope,
    GlossarySnapshot,
};
use crate::glossary::model::{Glossary, GlossaryDoc};
use crate::glossary::normalize::NormalizeReview;
use crate::glossary::reference::{self, ReferenceStatus, ReferenceSummary, ReferenceTerminology};
use crate::glossary::run::{self, GlossaryOpKind, StartArgs};
use crate::glossary::watch::{self, GlossaryWatchState};
use crate::glossary::world_detector::WorldType;
use crate::glossary::{improve, scope};

/// O9 — the folder's glossary and its metadata; `missing` when there's no
/// glossary.json, `unreadable` (with the line, when known) when it isn't valid.
/// A missing or corrupt sidecar gives the default meta, never a failed load.
/// Meta entries for terms that are no longer in the glossary (removed by hand
/// in a text editor, say) are left out.
#[tauri::command]
pub fn load_glossary(folder: String) -> GlossaryLoad {
    load_folder(&PathBuf::from(&folder))
}

fn load_folder(dir: &Path) -> GlossaryLoad {
    let Ok(text) = std::fs::read_to_string(dir.join("glossary.json")) else {
        return GlossaryLoad::Missing;
    };
    if let Err(e) = serde_json::from_str::<serde_json::Value>(&text) {
        return GlossaryLoad::Unreadable { message: e.to_string(), line: Some(e.line() as u32) };
    }
    match Glossary::from_json(&text) {
        Some(g) => {
            let mut meta = load_meta(dir);
            meta.retain_terms(&g);
            GlossaryLoad::Ok { doc: GlossaryDoc::from(&g), meta }
        }
        None => GlossaryLoad::Unreadable {
            message: "glossary.json isn't in the expected format".into(),
            line: None,
        },
    }
}

/// O14 — atomic write; the UI auto-saves every term edit. `meta: None` leaves
/// the sidecar as it is. `snapshot: Some(kind)` first saves the current
/// glossary (and meta) to the history, labelled with the change about to
/// happen; when that fails nothing is written.
#[tauri::command]
pub fn save_glossary(
    folder: String,
    doc: GlossaryDoc,
    meta: Option<GlossaryMeta>,
    snapshot: Option<GlossaryChangeKind>,
) -> AppResult<()> {
    save_folder(&PathBuf::from(folder), doc, meta, snapshot)
}

fn save_folder(
    dir: &Path,
    doc: GlossaryDoc,
    meta: Option<GlossaryMeta>,
    snapshot: Option<GlossaryChangeKind>,
) -> AppResult<()> {
    if let Some(reason) = snapshot {
        history::snapshot(dir, reason)?;
    }
    save_folder_glossary(dir, &doc.into_glossary())?;
    if let Some(meta) = meta {
        save_meta(dir, &meta)?;
    }
    Ok(())
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

/// Cancels the active glossary op (build / Tidy up / Look up names online /
/// import).
#[tauri::command]
pub async fn cancel_glossary_build(app: AppHandle) -> AppResult<()> {
    run::cancel(app).await
}

/// The glossary a Tidy up or Look up works on, with plain errors for "there
/// isn't one" and "it's empty". `what` finishes "…so there's nothing to …".
fn glossary_to_improve(dir: &Path, what: &str) -> AppResult<Glossary> {
    let g = load_folder_glossary(dir).ok_or_else(|| AppError::Other(NO_GLOSSARY.into()))?;
    if g.is_empty() {
        return Err(AppError::Other(format!("The glossary is empty, so there's nothing to {what}.")));
    }
    Ok(g)
}

const NO_GLOSSARY: &str = "There's no glossary in this folder yet.";

/// Shown when Look up names online has no model to run on.
const NO_WEB_SEARCH: &str =
    "No model that can search the web is set up. Turn on Look up names online on the Models page.";

/// O12 — Tidy up, scoped (`scope: None` = all terms). Returns the review
/// WITHOUT saving; the UI saves on accept via `save_glossary`. Claims the op
/// slot (exclusive with builds, Look up and translation).
///
/// Cancel semantics: a user cancel resolves Ok with a no-changes review (the
/// glossary as it is) — the UI must discard a review that resolves after the
/// user stopped the op instead of presenting it.
#[tauri::command]
pub async fn normalize_glossary(
    app: AppHandle,
    folder: String,
    scope: Option<GlossaryScope>,
) -> AppResult<NormalizeReview> {
    let dir = PathBuf::from(&folder);
    let original = glossary_to_improve(&dir, "tidy up")?;
    let sub = scope::resolve(&original, scope.as_ref())?;
    if sub.is_empty() {
        return Err(AppError::Other("None of the chosen terms are in the glossary.".into()));
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
    Ok(improve::tidy_up(&svc, &original, &sub, &templates, &tx, &cancel).await)
}

/// Look up names online on the glossary as it is — no subtitle files are
/// read. Scoped like Tidy up (`scope: None` = all terms); `show_name` is what
/// the model searches for. Returns a review; nothing is saved (the UI saves
/// the show name in the meta). Runs on the same web-search model a build
/// would use, claims the op slot (cancel with `cancel_glossary_build`), and
/// sends a `personalizing` phase event when the request goes out.
///
/// Cancel semantics: as `normalize_glossary` — a user cancel resolves Ok with
/// a no-changes review, which the UI discards.
#[tauri::command]
pub async fn lookup_glossary_names(
    app: AppHandle,
    folder: String,
    show_name: String,
    scope: Option<GlossaryScope>,
) -> AppResult<NormalizeReview> {
    let dir = PathBuf::from(&folder);
    let show_name = show_name.trim();
    if show_name.is_empty() {
        return Err(AppError::Other(
            "Type the show's name first, so the lookup knows what to search for.".into(),
        ));
    }
    let original = glossary_to_improve(&dir, "look up")?;
    let sub = scope::resolve(&original, scope.as_ref())?;
    if sub.is_empty() {
        return Err(AppError::Other("None of the chosen terms are in the glossary.".into()));
    }
    let cfg = config_store::load(&app)?;
    let conn =
        config_store::resolve_web_search(&cfg).ok_or_else(|| AppError::Other(NO_WEB_SEARCH.into()))?;
    let template = crate::prompts::resolve(
        crate::prompts::PromptId::GlossaryPersonalize,
        &crate::prompts::overrides_dir(&app)?,
    )?;
    let cancel = run::claim_slot(&app, GlossaryOpKind::LookUp).await?;
    // Declared before svc/tx so it drops last (after their senders close).
    let _guard = run::SlotGuard::new(app.clone());
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    run::spawn_forwarder(app.clone(), rx);
    let svc = run::service_for(&conn, cancel.clone(), tx.clone());
    improve::look_up(&svc, &original, &sub, show_name, &template, &tx, &cancel).await
}

/// The glossary's version history, newest first: up to ten snapshots, plus
/// the legacy `glossary.prev.json` (id `prev`) when the folder has one.
#[tauri::command]
pub fn list_glossary_snapshots(folder: String) -> AppResult<Vec<GlossarySnapshot>> {
    Ok(history::list(&PathBuf::from(folder)))
}

/// Bring back a snapshot. The current glossary is snapshotted first
/// (reason `restore`), so a restore can itself be undone.
#[tauri::command]
pub fn restore_glossary_snapshot(folder: String, id: String) -> AppResult<()> {
    history::restore(&PathBuf::from(folder), &id)
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
        return Err(AppError::Other("Pick at least one file first.".into()));
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

/// The newest earlier version with terms (a history snapshot, or the legacy
/// `glossary.prev.json`); None = nothing to go back to yet. Kept for the old
/// confirm dialogs; the history list replaces it. A read — no slot claim.
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

/// Install a picked `glossary.json` as this folder's glossary. The current one
/// goes to the history first (reason `import`); every imported term is marked
/// as yours. Returns the imported term count.
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
        return Err(AppError::Other(NO_GLOSSARY.into()));
    }
    // fs::copy truncates dest before reading, so dest == src would zero the
    // glossary. canonicalize fails for a non-existent dest — fine: a
    // non-existent dest can't be src.
    if let (Ok(d), Ok(s)) = (Path::new(&dest).canonicalize(), src.canonicalize()) {
        if d == s {
            return Err(AppError::Other(
                "That's the glossary itself. Pick another place to export to.".into(),
            ));
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
        return Err(AppError::Other(NO_GLOSSARY.into()));
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
/// Powers the Look up names online gating.
#[tauri::command]
pub fn web_search_status(app: AppHandle) -> AppResult<Option<String>> {
    Ok(config_store::web_search_label(&config_store::load(&app)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glossary::meta::TermOrigin;

    fn doc(terms: &[(&str, &str)]) -> GlossaryDoc {
        let mut g = Glossary::new("xianxia");
        for (s, t) in terms {
            g.characters.insert(s.to_string(), t.to_string());
        }
        GlossaryDoc::from(&g)
    }

    fn meta_with_origin(source: &str) -> GlossaryMeta {
        let mut m = GlossaryMeta { show_name: Some("Martial Universe".into()), ..Default::default() };
        m.origins.entry("characters".into()).or_default().insert(source.into(), TermOrigin::Yours);
        m
    }

    #[test]
    fn load_missing_and_unreadable_with_line() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(load_folder(dir.path()), GlossaryLoad::Missing));
        std::fs::write(dir.path().join("glossary.json"), "{\n  \"terms\": {\n    oops\n}").unwrap();
        match load_folder(dir.path()) {
            GlossaryLoad::Unreadable { line, message } => {
                assert_eq!(line, Some(3));
                assert!(!message.is_empty());
            }
            other => panic!("expected unreadable, got {other:?}"),
        }
    }

    #[test]
    fn load_returns_the_sidecar_meta() {
        let dir = tempfile::tempdir().unwrap();
        save_folder(dir.path(), doc(&[("林动", "Lin Dong")]), Some(meta_with_origin("林动")), None)
            .unwrap();
        match load_folder(dir.path()) {
            GlossaryLoad::Ok { doc, meta } => {
                assert_eq!(doc.count, 1);
                assert_eq!(meta.origins["characters"]["林动"], TermOrigin::Yours);
                assert_eq!(meta.show_name.as_deref(), Some("Martial Universe"));
            }
            other => panic!("expected ok, got {other:?}"),
        }
    }

    #[test]
    fn load_with_corrupt_meta_still_loads_with_default_meta() {
        let dir = tempfile::tempdir().unwrap();
        save_folder(dir.path(), doc(&[("林动", "Lin Dong")]), None, None).unwrap();
        let meta_dir = dir.path().join(".polygluttony");
        std::fs::create_dir_all(&meta_dir).unwrap();
        std::fs::write(meta_dir.join("glossary-meta.json"), "{{{ not json").unwrap();
        match load_folder(dir.path()) {
            GlossaryLoad::Ok { doc, meta } => {
                assert_eq!(doc.count, 1);
                assert!(meta.origins.is_empty() && meta.show_name.is_none());
            }
            other => panic!("expected ok, got {other:?}"),
        }
    }

    #[test]
    fn load_leaves_out_meta_for_terms_no_longer_there() {
        let dir = tempfile::tempdir().unwrap();
        save_folder(dir.path(), doc(&[("林动", "Lin Dong")]), Some(meta_with_origin("gone")), None)
            .unwrap();
        let GlossaryLoad::Ok { meta, .. } = load_folder(dir.path()) else { panic!() };
        assert!(meta.origins.is_empty());
        assert_eq!(meta.show_name.as_deref(), Some("Martial Universe"));
    }

    #[test]
    fn save_without_meta_leaves_the_sidecar_alone() {
        let dir = tempfile::tempdir().unwrap();
        save_folder(dir.path(), doc(&[("林动", "Lin Dong")]), Some(meta_with_origin("林动")), None)
            .unwrap();
        save_folder(dir.path(), doc(&[("林动", "Lin Dong (edited)")]), None, None).unwrap();
        let m = load_meta(dir.path());
        assert_eq!(m.origins["characters"]["林动"], TermOrigin::Yours);
        assert!(history::list(dir.path()).is_empty(), "no snapshot unless asked");
    }

    #[test]
    fn save_with_snapshot_keeps_the_previous_version() {
        let dir = tempfile::tempdir().unwrap();
        save_folder(dir.path(), doc(&[("林动", "Lin Dong")]), Some(meta_with_origin("林动")), None)
            .unwrap();
        save_folder(
            dir.path(),
            doc(&[("林动", "Lin Dong"), ("应欢欢", "Ying Huanhuan")]),
            Some(GlossaryMeta::default()),
            Some(GlossaryChangeKind::TidyUp),
        )
        .unwrap();
        let snaps = history::list(dir.path());
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].reason, GlossaryChangeKind::TidyUp);
        assert_eq!(snaps[0].count, 1);
        assert!(load_meta(dir.path()).origins.is_empty(), "the new meta was written");

        restore_glossary_snapshot(dir.path().to_string_lossy().into(), snaps[0].id.clone())
            .unwrap();
        assert_eq!(load_meta(dir.path()).origins["characters"]["林动"], TermOrigin::Yours);
        assert_eq!(load_folder_glossary(dir.path()).unwrap().count(), 1);
    }

    /// The first save in a folder has nothing to snapshot — that's not an error.
    #[test]
    fn save_with_snapshot_and_no_glossary_yet() {
        let dir = tempfile::tempdir().unwrap();
        save_folder(dir.path(), doc(&[("林动", "Lin Dong")]), None, Some(GlossaryChangeKind::BulkEdit))
            .unwrap();
        assert!(history::list(dir.path()).is_empty());
        assert_eq!(load_folder_glossary(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn improve_errors_are_plain() {
        let dir = tempfile::tempdir().unwrap();
        let err = glossary_to_improve(dir.path(), "tidy up").unwrap_err().to_string();
        assert_eq!(err, "There's no glossary in this folder yet.");
        save_folder(dir.path(), doc(&[]), None, None).unwrap();
        let err = glossary_to_improve(dir.path(), "look up").unwrap_err().to_string();
        assert_eq!(err, "The glossary is empty, so there's nothing to look up.");
        save_folder(dir.path(), doc(&[("林动", "Lin Dong")]), None, None).unwrap();
        assert_eq!(glossary_to_improve(dir.path(), "tidy up").unwrap().count(), 1);
    }
}
