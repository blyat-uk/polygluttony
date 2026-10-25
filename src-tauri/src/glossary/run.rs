//! Glossary run manager: one glossary op at a time (build / Tidy up / Look
//! up names online / import), mutually exclusive with translation runs. Mirrors
//! `translation/run.rs` with a simpler forwarder (the slot is released by a
//! `SlotGuard` held by the op, not by channel close).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::config::store as config_store;
use crate::config::{Connection, Step};
use crate::error::{AppError, AppResult};
use crate::events::{self, GlossaryEvent, RunEvent};
use crate::glossary::build::{build_glossary, BuildJob, BuildMode};
use crate::glossary::world_detector::WorldType;
use crate::llm::service::LlmService;
use crate::llm::LlmDriver;
use crate::models::language_pair::LanguagePair;

/// Managed Tauri state.
#[derive(Default)]
pub struct GlossaryRunState(pub Mutex<Option<GlossaryOpHandle>>);

pub struct GlossaryOpHandle {
    pub kind: GlossaryOpKind,
    /// The subtitle folder whose glossary the op works on.
    pub folder: PathBuf,
    pub cancel: CancellationToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlossaryOpKind {
    Build,
    /// Tidy up on the glossary as it is.
    Normalize,
    /// Look up names online on the glossary as it is.
    LookUp,
    /// Install a picked glossary.json in place of the folder's.
    Import,
    /// Import from translated episodes: reads them into the reference cache,
    /// never writes glossary.json.
    Reference,
}

impl GlossaryOpKind {
    /// The op writes glossary.json itself, so nothing else may meanwhile.
    pub fn writes_glossary(self) -> bool {
        matches!(self, GlossaryOpKind::Build | GlossaryOpKind::Import)
    }
}

/// Why a direct write to the glossary (an edit, a restore) is refused.
pub const GLOSSARY_BUSY: &str = "The glossary is being rebuilt right now. Try again when it's done.";

/// Pure: refuse a direct write to `folder`'s glossary while `op` (the op
/// holding the slot, if any) is rewriting that same glossary. A build's
/// final save would drop what the write added.
pub fn refuse_while_rewritten(op: Option<&GlossaryOpHandle>, folder: &Path) -> AppResult<()> {
    match op {
        Some(op)
            if op.kind.writes_glossary()
                && crate::translation::live::same_folder(&op.folder, folder) =>
        {
            Err(AppError::Other(GLOSSARY_BUSY.into()))
        }
        _ => Ok(()),
    }
}

pub struct StartArgs {
    pub folder: String,
    pub mode: BuildMode,
    pub files: Vec<String>,
    pub world_type: WorldType,
    pub source_lang: String,
    pub target_lang: String,
    pub normalize: bool,
    pub personalize: bool,
    pub personalize_context: String,
}

/// Claim the single glossary-op slot. LOCK ORDERING: translation `RunState`
/// FIRST, then `GlossaryRunState` — `translation::run::start` uses the same
/// order; reversing it deadlocks.
pub async fn claim_slot(
    app: &AppHandle,
    kind: GlossaryOpKind,
    folder: &Path,
) -> AppResult<CancellationToken> {
    let t_state = app.state::<crate::translation::run::RunState>();
    let t_guard = t_state.0.lock().await;
    if t_guard.is_some() {
        return Err(AppError::RunAlreadyActive);
    }
    let g_state = app.state::<GlossaryRunState>();
    let mut g_guard = g_state.0.lock().await;
    if g_guard.is_some() {
        return Err(AppError::RunAlreadyActive);
    }
    let cancel = CancellationToken::new();
    *g_guard = Some(GlossaryOpHandle { kind, folder: folder.to_path_buf(), cancel: cancel.clone() });
    Ok(cancel)
}

pub async fn release_slot(app: &AppHandle) {
    if let Some(state) = app.try_state::<GlossaryRunState>() {
        *state.0.lock().await = None;
    }
}

/// RAII slot release: dropping the guard releases the glossary-op slot even if
/// the op panics (mirrors translation's Drop-driven release via channel close).
pub struct SlotGuard {
    app: AppHandle,
}

impl SlotGuard {
    pub fn new(app: AppHandle) -> Self {
        SlotGuard { app }
    }
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        let app = self.app.clone();
        tauri::async_runtime::spawn(async move {
            release_slot(&app).await;
        });
    }
}

/// Forward engine events to the webview until the channel closes. Unlike the
/// translation forwarder this does NOT clear the run slot — the op that
/// claimed it releases it.
pub fn spawn_forwarder(app: AppHandle, mut rx: mpsc::Receiver<GlossaryEvent>) {
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = rx.recv().await {
            let _ = app.emit(events::GLOSSARY_EVENT, &ev);
        }
    });
}

/// `LlmService` emits its metering logs as `RunEvent::Log` (step-3 type). This
/// adapter bridges them onto the glossary channel without touching LlmService.
pub fn llm_log_channel(g_tx: mpsc::Sender<GlossaryEvent>) -> mpsc::Sender<RunEvent> {
    let (tx, mut rx) = mpsc::channel::<RunEvent>(64);
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = rx.recv().await {
            if let RunEvent::Log { level, message, .. } = ev {
                let _ = g_tx.send(GlossaryEvent::Log { level, message }).await;
            }
        }
    });
    tx
}

/// Build an `LlmService` for a glossary op on `conn`.
pub fn service_for(
    conn: &Connection,
    cancel: CancellationToken,
    g_tx: mpsc::Sender<GlossaryEvent>,
) -> LlmService {
    let concurrency = conn.concurrency.unwrap_or(2).max(1);
    let driver: Arc<dyn LlmDriver> = Arc::from(crate::llm::create_driver(conn.clone()));
    LlmService::new(driver, concurrency, cancel, llm_log_channel(g_tx))
}

pub async fn start(app: AppHandle, args: StartArgs) -> AppResult<()> {
    if args.files.is_empty() {
        return Err(AppError::Other("Pick at least one episode first.".into()));
    }
    let pair = LanguagePair::from_codes(&args.source_lang, &args.target_lang)?;
    if !pair.supports_glossary {
        return Err(AppError::Other(format!(
            "Glossaries aren't available for {} subtitles.",
            pair.source_name
        )));
    }
    let cfg = config_store::load(&app)?;
    // Extract always runs (it also reads the reference files and sets the
    // batch size); Normalize only when asked for, so a Normalize model without
    // a key doesn't block a build that skips it.
    let extract = config_store::step_connection(&cfg, Step::Extract)?;
    let normalize = if args.normalize {
        Some(config_store::step_connection(&cfg, Step::Normalize)?)
    } else {
        None
    };

    let prompt_pack =
        crate::prompts::GlossaryPrompts::resolve(&crate::prompts::overrides_dir(&app)?)?;

    let cancel = claim_slot(&app, GlossaryOpKind::Build, Path::new(&args.folder)).await?;

    // No backup here: the build snapshots the current glossary to the history
    // right before its FIRST write (build.rs `BuildWriter`), so a run that
    // writes nothing leaves no stray history entry.
    let (tx, rx) = mpsc::channel::<GlossaryEvent>(512);
    spawn_forwarder(app.clone(), rx);

    // Each step gets its own service: its own model, and its own concurrency.
    let svc = service_for(&extract, cancel.clone(), tx.clone());
    let norm_svc = normalize.map(|conn| service_for(&conn, cancel.clone(), tx.clone()));
    // Personalization service only when requested AND a web-search model
    // resolves (cap 1 — single call). MUST share the build's cancel token so
    // cancel can interrupt a long web-search call.
    let personalize_svc = if args.personalize {
        config_store::resolve_web_search(&cfg).map(|p_conn| {
            let d: Arc<dyn LlmDriver> = Arc::from(crate::llm::create_driver(p_conn));
            LlmService::new(d, 1, cancel.clone(), llm_log_channel(tx.clone()))
        })
    } else {
        None
    };

    let job = BuildJob {
        folder: PathBuf::from(&args.folder),
        mode: args.mode,
        files: args.files,
        world_type: args.world_type.as_str().to_string(),
        pair,
        normalize: args.normalize,
        personalize: args.personalize,
        personalize_context: args.personalize_context,
        prompts: prompt_pack,
        batch_limit: extract.batch_dialogue_limit,
        cancel: cancel.clone(),
    };
    let app_for_release = app.clone();
    tauri::async_runtime::spawn(async move {
        // RAII slot release — survives a panic in build_glossary. Declared
        // FIRST so it drops LAST (locals drop in reverse declaration order):
        // the LlmService handles below must drop before the slot is released
        // so their internal log-adapter senders close before a new op can
        // claim the channel.
        let _guard = SlotGuard::new(app_for_release);
        // Without Normalize the build never calls `norm_svc`; extract's stands in.
        build_glossary(job, &svc, norm_svc.as_ref().unwrap_or(&svc), personalize_svc.as_ref(), tx).await;
        drop(svc);
        drop(norm_svc);
        drop(personalize_svc);
        // tx consumed by build_glossary → forwarder drains & exits; then
        // _guard drops here, releasing the slot.
    });
    Ok(())
}

/// Cancels whichever glossary op holds the slot (build, Tidy up, Look up
/// names online, import).
pub async fn cancel(app: AppHandle) -> AppResult<()> {
    let state = app.state::<GlossaryRunState>();
    let guard = state.0.lock().await;
    match guard.as_ref() {
        Some(h) => {
            h.cancel.cancel();
            Ok(())
        }
        None => Err(AppError::NoActiveRun),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(kind: GlossaryOpKind, folder: &Path) -> GlossaryOpHandle {
        GlossaryOpHandle { kind, folder: folder.to_path_buf(), cancel: CancellationToken::new() }
    }

    #[test]
    fn direct_writes_wait_for_a_build_or_import_of_the_same_glossary() {
        let here = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        assert!(refuse_while_rewritten(None, here.path()).is_ok());
        for kind in [GlossaryOpKind::Build, GlossaryOpKind::Import] {
            let err = refuse_while_rewritten(Some(&op(kind, here.path())), here.path()).unwrap_err();
            assert_eq!(err.to_string(), GLOSSARY_BUSY);
            // However the folder is spelled.
            let slashed = PathBuf::from(format!("{}/", here.path().display()));
            assert!(refuse_while_rewritten(Some(&op(kind, here.path())), &slashed).is_err());
            // Another folder's glossary is free.
            assert!(refuse_while_rewritten(Some(&op(kind, elsewhere.path())), here.path()).is_ok());
        }
        // Ops that leave glossary.json alone (or save through the UI) don't block.
        for kind in [GlossaryOpKind::Normalize, GlossaryOpKind::LookUp, GlossaryOpKind::Reference] {
            assert!(refuse_while_rewritten(Some(&op(kind, here.path())), here.path()).is_ok());
        }
    }
}
