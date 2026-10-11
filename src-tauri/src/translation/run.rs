//! Run manager: one active run; spawns ≤concurrency file pipelines; forwards
//! engine events to the webview.
//!
//! # tx lifecycle — why the forwarder terminates
//!
//! There are three `mpsc::Sender<RunEvent>` clones at peak:
//!
//! 1. **`tx`** — held by the spawner task until it drops at the end of its
//!    body (after sending `RunFinished`).
//! 2. **`job_tx`** — a per-file clone passed into each `translate_file` call;
//!    dropped when the file task returns.
//! 3. **`svc`'s internal clone** — `LlmService::new(driver, cap, cancel, tx)` stores
//!    one clone to emit `Log` events. `svc` is wrapped in `Arc` and cloned once per
//!    file task (`job_svc`). The spawner's own `svc` binding and all `job_svc` arcs
//!    are dropped when the spawner task and each file task finish. No other code path
//!    holds an `Arc<LlmService>`, so the service and its inner tx clone are freed once
//!    the last file task exits and the spawner drops its `svc`.
//!
//! Once (1), (2) for every file, and (3) have all dropped, `rx.recv()` returns `None`
//! and the forwarder loop exits — triggering state clear.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::config::projects::Tone;
use crate::config::store as config_store;
use crate::config::Step;
use crate::error::{AppError, AppResult};
use crate::events::{self, FileResult, RunEvent};
use crate::glossary::io::load_folder_glossary;
use crate::glossary::model::Glossary;
use crate::llm::service::LlmService;
use crate::models::language_pair::LanguagePair;
use crate::translation::pipeline::{translate_file, FileJob};

/// Managed Tauri state.
#[derive(Default)]
pub struct RunState(pub Mutex<Option<RunHandle>>);

pub struct RunHandle {
    pub cancel: CancellationToken,
}

pub struct StartArgs {
    pub folder: String,
    pub files: Vec<String>,
    pub tone: Tone,
    pub source_lang: String,
    pub target_lang: String,
}

pub async fn start(app: AppHandle, args: StartArgs) -> AppResult<()> {
    let state = app.state::<RunState>();
    let mut guard = state.0.lock().await;
    if guard.is_some() {
        return Err(AppError::RunAlreadyActive);
    }
    // Exclusivity with glossary operations. Lock ordering: RunState (already
    // held here) FIRST, then GlossaryRunState — glossary::run::claim_slot uses
    // the same order.
    {
        let g_state = app.state::<crate::glossary::run::GlossaryRunState>();
        if g_state.0.lock().await.is_some() {
            return Err(AppError::RunAlreadyActive);
        }
    }

    let cfg = config_store::load(&app)?;
    let conn = config_store::step_connection(&cfg, Step::Translate)?;

    if args.files.is_empty() {
        return Err(AppError::Other("no files selected".into()));
    }

    let pair = LanguagePair::from_codes(&args.source_lang, &args.target_lang)?;
    // Resolve prompt templates ONCE — a running job is immune to mid-job edits;
    // changes apply to the next run. An unreadable override fails loudly here.
    let prompts = Arc::new(crate::prompts::TranslationPrompts::resolve(
        &crate::prompts::overrides_dir(&app)?,
        &pair,
        args.tone,
    )?);
    let folder = PathBuf::from(&args.folder);
    let glossary: Arc<Glossary> = Arc::new(load_folder_glossary(&folder).unwrap_or_default());

    let cancel = CancellationToken::new();
    let (tx, mut rx) = mpsc::channel::<RunEvent>(512);
    let concurrency = conn.concurrency.unwrap_or(2).max(1);
    let driver: Arc<dyn crate::llm::LlmDriver> = Arc::from(crate::llm::create_driver(conn.clone()));
    let svc = Arc::new(LlmService::new(driver, concurrency, cancel.clone(), tx.clone()));

    *guard = Some(RunHandle { cancel: cancel.clone() });
    drop(guard);

    // Forwarder: engine events → webview; clears state.
    //
    // `rx.recv()` returns None once all Sender clones are dropped. The spawner
    // holds `tx` (dropped at end of spawner body) and `svc` (which holds one
    // inner tx clone, dropped when the Arc refcount hits zero after every file
    // task and the spawner itself drop their `Arc<LlmService>`). Each file task
    // also holds `job_tx` (dropped when the task returns). So the forwarder
    // exits naturally after RunFinished is sent and all tasks have finished.
    let app_fwd = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = rx.recv().await {
            let _ = app_fwd.emit(events::TRANSLATION_EVENT, &ev);
        }
        if let Some(state) = app_fwd.try_state::<RunState>() {
            *state.0.lock().await = None;
        }
    });

    // Worker spawner.
    let sem = Arc::new(Semaphore::new(concurrency as usize));
    let batch_limit = conn.batch_dialogue_limit;
    tauri::async_runtime::spawn(async move {
        // Zip file names with task handles so a panicking task can emit a named error.
        let mut handles: Vec<(String, tauri::async_runtime::JoinHandle<FileResult>)> = Vec::new();
        for name in args.files {
            // Race semaphore acquisition against cancellation for crisp shutdown.
            let permit = tokio::select! {
                res = sem.clone().acquire_owned() => match res {
                    Ok(p) => p,
                    Err(_) => break, // semaphore closed (shouldn't happen)
                },
                _ = cancel.cancelled() => break,
            };
            if cancel.is_cancelled() {
                drop(permit);
                break;
            }
            let job_tx = tx.clone();
            let job_cancel = cancel.clone();
            let job_svc = svc.clone();
            let job_glossary = glossary.clone();
            let job_pair = pair.clone();
            let job_prompts = prompts.clone();
            let input = folder.join(&name);
            let task_name = name.clone();
            let handle = tauri::async_runtime::spawn(async move {
                let _permit = permit;
                translate_file(FileJob {
                    input,
                    file_name: task_name,
                    svc: &job_svc,
                    glossary: &job_glossary,
                    pair: job_pair,
                    prompts: &job_prompts,
                    batch_limit,
                    cancel: job_cancel,
                    tx: job_tx,
                })
                .await
            });
            handles.push((name, handle));
        }

        // Drop svc here so the service's inner tx clone is released before we
        // wait on handles. File tasks each have their own Arc<LlmService> clone;
        // those drop when their task finishes. The spawner's `svc` binding is
        // the only one left at this point, and must be dropped before the join
        // loop or the forwarder could otherwise outlive the RunFinished send.
        drop(svc);

        let mut results = Vec::new();
        for (name, handle) in handles {
            match handle.await {
                Ok(r) => results.push(r),
                Err(tauri_err) => {
                    // Task panicked: extract the panic payload when possible.
                    // tauri::async_runtime wraps tokio::task::JoinError inside
                    // tauri::Error::JoinError; unwrap it to reach is_panic / into_panic.
                    let message = if let tauri::Error::JoinError(je) = tauri_err {
                        if je.is_panic() {
                            let payload = je.into_panic();
                            if let Some(s) = payload.downcast_ref::<&str>() {
                                s.to_string()
                            } else if let Some(s) = payload.downcast_ref::<String>() {
                                s.clone()
                            } else {
                                "file task panicked".into()
                            }
                        } else {
                            "file task panicked".into()
                        }
                    } else {
                        "file task panicked".into()
                    };
                    // Emit both the error message and a State(Failed) so the UI
                    // can mark the file as failed without a separate state event.
                    let _ = tx
                        .send(RunEvent::State {
                            file: name.clone(),
                            state: crate::events::FileStateKind::Failed,
                            detail: None,
                        })
                        .await;
                    let _ = tx
                        .send(RunEvent::Error {
                            file: name.clone(),
                            message,
                        })
                        .await;
                    results.push(FileResult {
                        file: name,
                        success: false,
                        total_lines: 0,
                        translated_lines: 0,
                        has_warnings: false,
                        issues: Vec::new(),
                        output_path: None,
                    });
                }
            }
        }
        // Send RunFinished then drop tx — forwarder loop ends once this and all
        // job_tx clones (already dropped by finished file tasks) are released.
        let _ = tx.send(RunEvent::RunFinished { results }).await;
        // tx dropped here
    });

    Ok(())
}

pub async fn cancel(app: AppHandle) -> AppResult<()> {
    let state = app.state::<RunState>();
    let guard = state.0.lock().await;
    match guard.as_ref() {
        Some(h) => {
            h.cancel.cancel();
            Ok(())
        }
        None => Err(AppError::NoActiveRun),
    }
}
