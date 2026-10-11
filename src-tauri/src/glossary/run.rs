//! Glossary run manager: one glossary op at a time (build / standalone
//! normalize / import), mutually exclusive with translation runs. Mirrors
//! `translation/run.rs` with a simpler forwarder (the slot is released by a
//! `SlotGuard` held by the op, not by channel close).

use std::path::PathBuf;
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
    #[allow(dead_code)] // surfaced to the UI in a later step; cancel is the point
    pub kind: GlossaryOpKind,
    pub cancel: CancellationToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlossaryOpKind {
    Build,
    Normalize,
    Import,
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
pub async fn claim_slot(app: &AppHandle, kind: GlossaryOpKind) -> AppResult<CancellationToken> {
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
    *g_guard = Some(GlossaryOpHandle { kind, cancel: cancel.clone() });
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
        return Err(AppError::Other("no files selected".into()));
    }
    let pair = LanguagePair::from_codes(&args.source_lang, &args.target_lang)?;
    if !pair.supports_glossary {
        return Err(AppError::Other(format!(
            "glossary isn't available for {}",
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

    let cancel = claim_slot(&app, GlossaryOpKind::Build).await?;

    // Regenerate throws the old glossary away — and not only at the end: the
    // first completed batch's incremental save (build.rs) already overwrites
    // glossary.json with new-terms-only.
    //
    // The backup runs AFTER the slot claim because taking it is itself
    // destructive: `glossary.prev.json` is a single slot, so this copy
    // DESTROYS the previous undo copy. A run that cannot start (another op
    // holds the slot) must not replace the user's only undo copy while
    // reporting that nothing happened. `claim_slot` has already taken the slot
    // and the `SlotGuard` is only created inside the spawned task, so a failing
    // backup has to hand the slot back itself. A backup we cannot write means
    // we do not start.
    if args.mode == BuildMode::Regenerate {
        if let Err(e) = crate::glossary::io::backup_folder_glossary(&PathBuf::from(&args.folder)) {
            release_slot(&app).await;
            return Err(e);
        }
    }
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

/// Cancels whichever glossary op holds the slot (build, normalize, import).
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
