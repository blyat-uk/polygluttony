//! Run manager: one active run; spawns ≤concurrency file pipelines; forwards
//! engine events to the webview and folds them into the live board
//! (`translation::live`) that episode windows read. Each file's ending is
//! stored as it lands (`translation::results`), so the episode list, Review
//! and Resume survive a restart.
//!
//! Two kinds of run share the one slot: a translation run ([`start`]) and a
//! review run ([`start_review`]) that retranslates chosen lines of finished
//! episodes. Both speak the same events.
//!
//! # How a run ends
//!
//! Every started file ends done, needs-a-look, failed (on its own) or
//! stopped (`State(Stopped)`: the run stopped first). Files that never
//! started send nothing and stay pending. When the run stopped early —
//! the user's Stop, or an error no retry fixes — `Stopped { info }` goes out
//! once, then `RunFinished` with the started files' results.
//!
//! # tx lifecycle — why the forwarder terminates
//!
//! There are four `mpsc::Sender<RunEvent>` clones at peak:
//!
//! 1. **`tx`** — held by the spawner task until it drops at the end of its
//!    body (after sending `RunFinished`).
//! 2. **`job_tx`** — a per-file clone passed into each file task; dropped when
//!    the task returns.
//! 3. **`svc`'s internal clone** — `LlmService::new(driver, cap, cancel, tx)` stores
//!    one clone to emit `Log` events. `svc` is wrapped in `Arc` and cloned once per
//!    file task (`job_svc`). The spawner's own `svc` binding and all `job_svc` arcs
//!    are dropped when the spawner task and each file task finish. No other code path
//!    holds an `Arc<LlmService>`, so the service and its inner tx clone are freed once
//!    the last file task exits and the spawner drops its `svc`.
//! 4. **`verify_svc`'s internal clone** — the Verify step's own service (or
//!    another `Arc` of `svc` when Verify runs on Translate's model and settings),
//!    shared and dropped exactly like `svc` (`job_verify_svc` per file task).
//!
//! Once (1), (2) for every file, (3) and (4) have all dropped, `rx.recv()` returns
//! `None` and the forwarder loop exits — triggering state clear.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, Mutex, MutexGuard, Semaphore};

use crate::ass::tags::strip_for_text;
use crate::config::projects::Tone;
use crate::config::store as config_store;
use crate::config::{Connection, Provider, Step};
use crate::error::{AppError, AppResult};
use crate::events::{
    self, EpisodeChanged, FileResult, FileStateKind, ResultsChanged, RunEvent,
    EPISODE_CHANGED_EVENT, RESULTS_CHANGED_EVENT,
};
use crate::glossary::io::load_folder_glossary;
use crate::glossary::model::Glossary;
use crate::llm::service::LlmService;
use crate::models::language_pair::LanguagePair;
use crate::prompts::TranslationPrompts;
use crate::translation::live::LiveBoard;
use crate::translation::pipeline::{
    retranslate_episode_lines, translate_file, FileEnd, FileJob, FileOutcome, ReviewJob, Seed,
};
use crate::translation::results::{
    now_ms, EpisodeOutcome, EpisodeRecord, ResultsStore, StoredLines,
};
use crate::translation::stop::RunStop;

/// Managed Tauri state.
#[derive(Default)]
pub struct RunState(pub Mutex<Option<RunHandle>>);

pub struct RunHandle {
    pub stop: RunStop,
}

pub struct StartArgs {
    pub folder: String,
    pub files: Vec<String>,
    pub tone: Tone,
    pub source_lang: String,
    pub target_lang: String,
    /// Carry on from the stored partial translations of stopped files.
    pub resume: bool,
}

/// Verify on Translate's model with Translate's settings shares Translate's
/// service: one pool of requests in flight (the user's "Requests at once"
/// covers both) and one rate-limit back-off, as before steps had their own
/// models. Batch size and concurrency are ignored: Verify uses neither.
fn same_service(translate: &Connection, verify: &Connection) -> bool {
    let verify = Connection {
        batch_dialogue_limit: translate.batch_dialogue_limit,
        concurrency: translate.concurrency,
        ..verify.clone()
    };
    *translate == verify
}

/// Who answers a connection, for messages: the provider's name, or "The
/// server" for a custom URL (as the Models page says it).
fn who(conn: &Connection) -> String {
    if conn.provider == Provider::Custom {
        "The server".to_string()
    } else {
        crate::config::providers::provider_info(conn.provider).label
    }
}

/// Claim the one run slot. Lock ordering: RunState FIRST, then
/// GlossaryRunState — glossary::run::claim_slot uses the same order.
async fn claim(app: &AppHandle) -> AppResult<MutexGuard<'_, Option<RunHandle>>> {
    let guard = app.state::<RunState>().inner().0.lock().await;
    if guard.is_some() {
        return Err(AppError::RunAlreadyActive);
    }
    let g_state = app.state::<crate::glossary::run::GlossaryRunState>();
    if g_state.0.lock().await.is_some() {
        return Err(AppError::RunAlreadyActive);
    }
    Ok(guard)
}

/// What every file task of a run shares.
struct Kit {
    stop: RunStop,
    tx: mpsc::Sender<RunEvent>,
    svc: Arc<LlmService>,
    verify_svc: Arc<LlmService>,
    glossary: Arc<Glossary>,
    batch_limit: Option<u32>,
    concurrency: u32,
}

/// The Translate and Verify connections, the folder's glossary and the
/// run's services and channel. Nothing is claimed or started yet.
fn kit(app: &AppHandle, folder: &Path) -> AppResult<(Kit, mpsc::Receiver<RunEvent>)> {
    let cfg = config_store::load(app)?;
    let conn = config_store::step_connection(&cfg, Step::Translate)?;
    let verify_conn = config_store::step_connection(&cfg, Step::Verify)?;
    let glossary: Arc<Glossary> = Arc::new(load_folder_glossary(folder).unwrap_or_default());

    let stop = RunStop::default();
    let (tx, rx) = mpsc::channel::<RunEvent>(512);
    // Translate's concurrency is the run's: how many files at once, and so how
    // many requests each service may have in flight. Verify checks files as
    // they finish translating, so it runs as many at once as Translate does;
    // its own model and settings otherwise.
    let concurrency = conn.concurrency.unwrap_or(2).max(1);
    let service = |conn: Connection| {
        let label = who(&conn);
        let driver: Arc<dyn crate::llm::LlmDriver> = Arc::from(crate::llm::create_driver(conn));
        Arc::new(
            LlmService::new(driver, concurrency, stop.token().clone(), tx.clone())
                .with_provider(label),
        )
    };
    let svc = service(conn.clone());
    let verify_svc = if same_service(&conn, &verify_conn) {
        svc.clone()
    } else {
        service(verify_conn)
    };
    let batch_limit = conn.batch_dialogue_limit;
    Ok((
        Kit {
            stop,
            tx,
            svc,
            verify_svc,
            glossary,
            batch_limit,
            concurrency,
        },
        rx,
    ))
}

/// Resolve prompt templates ONCE — a running job is immune to mid-job edits;
/// changes apply to the next run. An unreadable override fails loudly here.
fn prompts(app: &AppHandle, pair: &LanguagePair, tone: Tone) -> AppResult<Arc<TranslationPrompts>> {
    Ok(Arc::new(TranslationPrompts::resolve(
        &crate::prompts::overrides_dir(app)?,
        pair,
        tone,
    )?))
}

/// Forwarder: engine events → live board + webview; clears the run slot
/// once every sender is gone. Internal (board-only) events never reach the
/// webview; every board change nudges the episode windows.
fn spawn_forwarder(app: AppHandle, mut rx: mpsc::Receiver<RunEvent>) {
    tauri::async_runtime::spawn(async move {
        let board = app.state::<LiveBoard>();
        while let Some(ev) = rx.recv().await {
            let change = board.apply(&ev);
            if !ev.is_internal() {
                let _ = app.emit(events::TRANSLATION_EVENT, &ev);
            }
            if let Some(payload) = change.into_event() {
                let _ = app.emit(EPISODE_CHANGED_EVENT, payload);
            }
        }
        if let Some(state) = app.try_state::<RunState>() {
            *state.0.lock().await = None;
        }
    });
}

/// A fresh board before any file task can send to it, with the translations
/// episodes start from; open episode windows refetch and see the new run (or
/// that they are no longer in it).
fn reset_board<'a>(
    app: &AppHandle,
    folder: &Path,
    files: &[String],
    primed: impl IntoIterator<Item = (&'a String, &'a BTreeMap<u32, String>)>,
) {
    let board = app.state::<LiveBoard>();
    board.reset(folder, files);
    for (file, translations) in primed {
        board.prime(
            file,
            translations.iter().map(|(id, t)| (*id, strip_for_text(t))),
        );
    }
    let _ = app.emit(EPISODE_CHANGED_EVENT, EpisodeChanged { file: None });
}

fn results_changed(app: &AppHandle, folder: &str, file: &str) {
    let _ = app.emit(
        RESULTS_CHANGED_EVENT,
        ResultsChanged {
            folder: folder.to_string(),
            file: file.to_string(),
        },
    );
}

/// What the stored lines of a run's episodes are made with.
#[derive(Debug, Clone)]
pub struct RunMeta {
    pub source_lang: String,
    pub target_lang: String,
    pub tone: Tone,
}

/// Pure: store how a file ended. Done and needs-a-look keep their issues and
/// lines; a stopped file keeps its partial lines and says why it stopped; a
/// failed one keeps only its reason.
pub fn store_outcome(
    store: &ResultsStore,
    folder: &Path,
    outcome: &FileOutcome,
    meta: &RunMeta,
    stop: &RunStop,
) -> AppResult<()> {
    let result = &outcome.result;
    let (kind, reason, issues, keep_lines) = match &outcome.end {
        FileEnd::Done => (EpisodeOutcome::Done, None, result.issues.clone(), true),
        FileEnd::NeedsLook => (EpisodeOutcome::NeedsLook, None, result.issues.clone(), true),
        FileEnd::Stopped => (
            EpisodeOutcome::Stopped,
            Some(stop.reason_sentence()),
            Vec::new(),
            true,
        ),
        FileEnd::Failed { reason, .. } => (
            EpisodeOutcome::Failed,
            Some(reason.clone()),
            Vec::new(),
            false,
        ),
    };
    let lines = match (&outcome.source_hash, keep_lines) {
        (Some(hash), true) => Some(StoredLines {
            file: result.file.clone(),
            source_lang: meta.source_lang.clone(),
            target_lang: meta.target_lang.clone(),
            tone: meta.tone,
            source_hash: hash.clone(),
            total_lines: result.total_lines,
            translations: outcome.translations.clone(),
            output_hash: outcome.output_hash.clone(),
        }),
        _ => None,
    };
    let record = EpisodeRecord {
        file: result.file.clone(),
        outcome: kind,
        issues,
        output_name: outcome.output_name.clone(),
        finished_ms: now_ms(),
        total_lines: result.total_lines,
        translated_lines: result.translated_lines,
        reason,
    };
    store.put_episode(folder, record, lines.as_ref())
}

/// Pure: the stored partial translations of the stopped files among `files`,
/// when they were made for this run's languages and tone from the source as
/// it is now. Any other stopped file is translated from the start. Decodes
/// the sources: call it off the async threads.
pub fn resume_seeds(
    store: &ResultsStore,
    folder: &Path,
    files: &[String],
    meta: &RunMeta,
) -> HashMap<String, Seed> {
    let results = store.results(folder);
    files
        .iter()
        .filter(|f| {
            results
                .episodes
                .get(*f)
                .is_some_and(|r| r.outcome == EpisodeOutcome::Stopped)
        })
        .filter_map(|f| {
            let lines = store.lines(folder, f)?;
            // Only lines made for this language pair and tone…
            let same_run = lines.source_lang == meta.source_lang
                && lines.target_lang == meta.target_lang
                && lines.tone == meta.tone;
            if !same_run {
                return None;
            }
            // …from the source as it is now.
            let source = crate::ass::decode::decode_file(&folder.join(f)).ok()?;
            if crate::translation::results::fingerprint(&source) != lines.source_hash {
                return None;
            }
            Some((
                f.clone(),
                Seed {
                    source_hash: lines.source_hash,
                    translations: lines.translations,
                },
            ))
        })
        .collect()
}

/// A file task panicked: its message, as plainly as the payload allows.
fn panic_message(err: tauri::Error) -> String {
    // tauri::async_runtime wraps tokio::task::JoinError inside
    // tauri::Error::JoinError; unwrap it to reach is_panic / into_panic.
    let detail = match err {
        tauri::Error::JoinError(je) if je.is_panic() => {
            let payload = je.into_panic();
            if let Some(s) = payload.downcast_ref::<&str>() {
                Some(s.to_string())
            } else {
                payload.downcast_ref::<String>().cloned()
            }
        }
        _ => None,
    };
    match detail {
        Some(d) => format!("Something went wrong inside Polygluttony: {d}"),
        None => "Something went wrong inside Polygluttony.".into(),
    }
}

/// Run `work` for each file, at most `concurrency` at once, stopping the
/// queue when the run stops (files never started stay pending: no events).
/// Then `Stopped` (when the run ended early) and `RunFinished` with every
/// started file's result. `work` must send the file's own events.
fn spawn_files<F, Fut>(
    kit: Kit,
    folder: String,
    resume: bool,
    files: Vec<String>,
    on_panic: impl Fn(&str, &str) + Send + 'static,
    work: F,
) where
    F: Fn(String, Arc<LlmService>, Arc<LlmService>, mpsc::Sender<RunEvent>) -> Fut + Send + 'static,
    Fut: Future<Output = FileResult> + Send + 'static,
{
    let Kit {
        stop,
        tx,
        svc,
        verify_svc,
        concurrency,
        ..
    } = kit;
    let sem = Arc::new(Semaphore::new(concurrency as usize));
    tauri::async_runtime::spawn(async move {
        // Always the run's first event: before any file task exists.
        let _ = tx
            .send(RunEvent::RunStarted {
                folder,
                files: files.clone(),
                resume,
            })
            .await;
        // Zip file names with task handles so a panicking task can emit a named error.
        let mut handles: Vec<(String, tauri::async_runtime::JoinHandle<FileResult>)> = Vec::new();
        for name in files {
            // Race semaphore acquisition against cancellation for crisp shutdown.
            let permit = tokio::select! {
                res = sem.clone().acquire_owned() => match res {
                    Ok(p) => p,
                    Err(_) => break, // semaphore closed (shouldn't happen)
                },
                _ = stop.token().cancelled() => break,
            };
            if stop.is_stopped() {
                drop(permit);
                break;
            }
            let fut = work(name.clone(), svc.clone(), verify_svc.clone(), tx.clone());
            let handle = tauri::async_runtime::spawn(async move {
                let _permit = permit;
                fut.await
            });
            handles.push((name, handle));
        }

        // Drop both services here so their inner tx clones are released before
        // we wait on handles. File tasks each have their own Arc<LlmService>
        // clones; those drop when their task finishes. The spawner's bindings
        // are the only ones left at this point, and must be dropped before the
        // join loop or the forwarder could otherwise outlive the RunFinished send.
        drop(svc);
        drop(verify_svc);

        let mut results = Vec::new();
        for (name, handle) in handles {
            match handle.await {
                Ok(r) => results.push(r),
                Err(err) => {
                    let message = panic_message(err);
                    on_panic(&name, &message);
                    // Emit both the error message and a State(Failed) so the UI
                    // can mark the file as failed without a separate state event.
                    let _ = tx
                        .send(RunEvent::State {
                            file: name.clone(),
                            state: FileStateKind::Failed,
                            detail: None,
                        })
                        .await;
                    let _ = tx
                        .send(RunEvent::Error {
                            file: name.clone(),
                            message,
                            error_kind: events::ErrorKind::Other,
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
        // Say why the run ended early, once, before it finishes.
        if let Some(info) = stop.info() {
            let _ = tx.send(RunEvent::Stopped { info }).await;
        }
        // Send RunFinished then drop tx — forwarder loop ends once this and all
        // job_tx clones (already dropped by finished file tasks) are released.
        let _ = tx.send(RunEvent::RunFinished { results }).await;
        // tx dropped here
    });
}

pub async fn start(app: AppHandle, args: StartArgs) -> AppResult<()> {
    let mut guard = claim(&app).await?;
    if args.files.is_empty() {
        return Err(AppError::Other("no files selected".into()));
    }
    let folder = PathBuf::from(&args.folder);
    let (kit, rx) = kit(&app, &folder)?;
    let pair = LanguagePair::from_codes(&args.source_lang, &args.target_lang)?;
    let prompts = prompts(&app, &pair, args.tone)?;
    let store = ResultsStore::for_app(&app)?;
    let meta = RunMeta {
        source_lang: pair.source.clone(),
        target_lang: pair.target.clone(),
        tone: args.tone,
    };

    *guard = Some(RunHandle {
        stop: kit.stop.clone(),
    });
    drop(guard);
    // Resume reads every stopped file: on a blocking thread, with the run
    // slot already taken and its lock let go.
    let seeds = if args.resume {
        let (store, folder, files, meta) =
            (store.clone(), folder.clone(), args.files.clone(), meta.clone());
        tauri::async_runtime::spawn_blocking(move || resume_seeds(&store, &folder, &files, &meta))
            .await
            .unwrap_or_default()
    } else {
        HashMap::new()
    };
    let seeds = Arc::new(std::sync::Mutex::new(seeds));
    {
        let seeds = seeds.lock().unwrap_or_else(|e| e.into_inner());
        reset_board(
            &app,
            &folder,
            &args.files,
            seeds.iter().map(|(f, s)| (f, &s.translations)),
        );
    }
    spawn_forwarder(app.clone(), rx);

    let stop = kit.stop.clone();
    let glossary = kit.glossary.clone();
    let batch_limit = kit.batch_limit;
    let panic_store = (
        store.clone(),
        folder.clone(),
        app.clone(),
        args.folder.clone(),
    );
    let on_panic = move |file: &str, message: &str| {
        let (store, folder, app, folder_arg) = &panic_store;
        let record = EpisodeRecord {
            file: file.to_string(),
            outcome: EpisodeOutcome::Failed,
            issues: Vec::new(),
            output_name: None,
            finished_ms: now_ms(),
            total_lines: 0,
            translated_lines: 0,
            reason: Some(message.to_string()),
        };
        if let Err(e) = store.put_episode(folder, record, None) {
            tracing::warn!("couldn't store {file}'s result: {e}");
        }
        results_changed(app, folder_arg, file);
    };
    let files = args.files;
    let folder_arg = args.folder;
    let started_folder = folder_arg.clone();
    spawn_files(
        kit,
        started_folder,
        args.resume,
        files,
        on_panic,
        move |name, svc, verify_svc, tx| {
            let seed = seeds
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&name);
            let (stop, glossary, prompts, pair) = (
                stop.clone(),
                glossary.clone(),
                prompts.clone(),
                pair.clone(),
            );
            let (store, folder, meta, app, folder_arg) = (
                store.clone(),
                folder.clone(),
                meta.clone(),
                app.clone(),
                folder_arg.clone(),
            );
            async move {
                let outcome = translate_file(FileJob {
                    input: folder.join(&name),
                    file_name: name.clone(),
                    svc: &svc,
                    verify_svc: &verify_svc,
                    glossary: &glossary,
                    pair,
                    prompts: &prompts,
                    batch_limit,
                    stop: stop.clone(),
                    tx,
                    seed,
                })
                .await;
                if let Err(e) = store_outcome(&store, &folder, &outcome, &meta, &stop) {
                    tracing::warn!("couldn't store {name}'s result: {e}");
                }
                results_changed(&app, &folder_arg, &name);
                outcome.result
            }
        },
    );
    Ok(())
}

/// One episode of a review run, checked before the run starts.
struct ReviewPlan {
    line_ids: Vec<u32>,
    output_name: String,
    issues: Vec<crate::events::VerifyIssue>,
    needs_look: bool,
    lines: StoredLines,
}

/// Pure: check a reviewer's request against the store. Every problem is a
/// plain sentence for the command's error.
fn plan_review(
    store: &ResultsStore,
    folder: &Path,
    file: &str,
    line_ids: Vec<u32>,
) -> AppResult<ReviewPlan> {
    let record = store.record(folder, file).ok_or_else(|| {
        AppError::Other(format!(
            "There's no stored result for {file}. Translate it first."
        ))
    })?;
    let output_name = match (record.outcome, record.output_name) {
        (EpisodeOutcome::Done | EpisodeOutcome::NeedsLook, Some(name)) => name,
        _ => {
            return Err(AppError::Other(format!(
                "{file} has no finished translation to fix yet. Translate it first."
            )))
        }
    };
    let lines = store.lines(folder, file).ok_or_else(|| {
        AppError::Other(format!(
            "The lines of {file} aren't stored any more. Translate it again to review it."
        ))
    })?;
    if line_ids.is_empty() {
        return Err(AppError::Other(
            "Pick at least one line to retranslate.".into(),
        ));
    }
    let source = crate::ass::decode::decode_file(&folder.join(file))
        .map_err(|e| AppError::Other(format!("Couldn't read {file}: {e}")))?;
    if crate::translation::results::fingerprint(&source) != lines.source_hash {
        return Err(AppError::Other(format!(
            "{file} changed since it was translated. Translate it again to review it."
        )));
    }
    crate::translation::output::check_unchanged(folder, &output_name, lines.output_hash.as_deref())?;
    Ok(ReviewPlan {
        line_ids,
        output_name,
        issues: record.issues,
        needs_look: record.outcome == EpisodeOutcome::NeedsLook,
        lines,
    })
}

/// Every reviewed episode's plan, with its language pair and prompts.
type Plans = HashMap<String, (ReviewPlan, LanguagePair, Arc<TranslationPrompts>)>;

/// Check every job against the store (on a blocking thread: it decodes the
/// sources) and resolve each episode's prompts.
async fn plan_all(
    app: &AppHandle,
    store: &ResultsStore,
    folder: &Path,
    jobs: Vec<(String, Vec<u32>)>,
) -> AppResult<Plans> {
    let (store, folder) = (store.clone(), folder.to_path_buf());
    let planned = tauri::async_runtime::spawn_blocking(move || {
        jobs.into_iter()
            .map(|(file, ids)| Ok((file.clone(), plan_review(&store, &folder, &file, ids)?)))
            .collect::<AppResult<Vec<_>>>()
    })
    .await
    .map_err(|e| AppError::Other(format!("Something went wrong inside Polygluttony: {e}")))??;
    let mut plans = HashMap::new();
    for (file, plan) in planned {
        let pair = LanguagePair::from_codes(&plan.lines.source_lang, &plan.lines.target_lang)?;
        let prompts = prompts(app, &pair, plan.lines.tone)?;
        plans.insert(file, (plan, pair, prompts));
    }
    Ok(plans)
}

/// A review run: retranslate chosen lines of finished episodes (with their
/// neighbours as context), rewrite each output and update its stored lines
/// and issues. `jobs` is file → line ids.
///
/// The episodes are marked running on the board BEFORE their stored lines
/// are read, so a line saved meanwhile is either read here or refused; it
/// can't land in between and be overwritten by the run.
pub async fn start_review(
    app: AppHandle,
    folder: String,
    jobs: Vec<(String, Vec<u32>)>,
) -> AppResult<()> {
    if jobs.is_empty() {
        return Err(AppError::Other("Nothing to retranslate.".into()));
    }
    let mut guard = claim(&app).await?;
    let folder_path = PathBuf::from(&folder);
    let store = ResultsStore::for_app(&app)?;
    let (kit, rx) = kit(&app, &folder_path)?;
    *guard = Some(RunHandle {
        stop: kit.stop.clone(),
    });
    drop(guard);

    let files: Vec<String> = jobs.iter().map(|(file, _)| file.clone()).collect();
    let board = app.state::<LiveBoard>();
    let previous = board.replace(&folder_path, &files);
    let plans = match plan_all(&app, &store, &folder_path, jobs).await {
        Ok(plans) => plans,
        Err(e) => {
            // The run never starts: the last board and a free slot again.
            board.put_back(previous);
            *app.state::<RunState>().0.lock().await = None;
            return Err(e);
        }
    };
    for (file, (plan, _, _)) in &plans {
        board.prime(
            file,
            plan.lines.translations.iter().map(|(id, t)| (*id, strip_for_text(t))),
        );
    }
    let _ = app.emit(EPISODE_CHANGED_EVENT, EpisodeChanged { file: None });
    spawn_forwarder(app.clone(), rx);

    let plans = Arc::new(std::sync::Mutex::new(plans));
    let stop = kit.stop.clone();
    let glossary = kit.glossary.clone();
    let batch_limit = kit.batch_limit;
    spawn_files(
        kit,
        folder.clone(),
        false,
        files,
        |_, _| {},
        move |name, svc, verify_svc, tx| {
            let plan = plans
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&name);
            let (stop, glossary, store, folder_path, app, folder) = (
                stop.clone(),
                glossary.clone(),
                store.clone(),
                folder_path.clone(),
                app.clone(),
                folder.clone(),
            );
            async move {
                let (plan, pair, prompts) = plan.expect("every queued file has a plan");
                let outcome = retranslate_episode_lines(ReviewJob {
                    job: FileJob {
                        input: folder_path.join(&name),
                        file_name: name.clone(),
                        svc: &svc,
                        verify_svc: &verify_svc,
                        glossary: &glossary,
                        pair,
                        prompts: &prompts,
                        batch_limit,
                        stop,
                        tx,
                        seed: Some(Seed {
                            source_hash: plan.lines.source_hash.clone(),
                            translations: plan.lines.translations.clone(),
                        }),
                    },
                    line_ids: plan.line_ids,
                    output_name: plan.output_name,
                    output_hash: plan.lines.output_hash.clone(),
                    issues: plan.issues,
                    needs_look: plan.needs_look,
                })
                .await;
                if outcome.failed.is_none() && !outcome.retranslated.is_empty() {
                    let fixed: std::collections::BTreeSet<u32> =
                        outcome.retranslated.iter().copied().collect();
                    let stored = store.edit_episode(&folder_path, &name, |record, lines| {
                        if let Some(lines) = lines {
                            lines.translations = outcome.translations.clone();
                            lines.output_hash = outcome.output_hash.clone();
                        }
                        record.issues.retain(|i| !fixed.contains(&i.line_id));
                        record.translated_lines = outcome.translations.len() as u32;
                        record.finished_ms = now_ms();
                        Ok(())
                    });
                    if let Err(e) = stored {
                        tracing::warn!("couldn't store {name}'s review: {e}");
                    }
                    results_changed(&app, &folder, &name);
                }
                outcome.result
            }
        },
    );
    Ok(())
}

pub async fn cancel(app: AppHandle) -> AppResult<()> {
    let state = app.state::<RunState>();
    let guard = state.0.lock().await;
    match guard.as_ref() {
        Some(h) => {
            h.stop.by_user();
            Ok(())
        }
        None => Err(AppError::NoActiveRun),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{sample, ThinkingLevel};
    use crate::events::{ErrorKind, VerifyIssue};
    use crate::translation::stop::StopCause;

    #[test]
    fn verify_shares_translates_service_only_on_the_same_model_and_settings() {
        let mut translate = sample(Provider::Anthropic, "claude-sonnet-5-5");
        translate.concurrency = Some(5);
        translate.batch_dialogue_limit = Some(200);
        // One-model view, or a per-step Verify left on Translate's model: Verify's
        // unused batch size / concurrency don't split the pool.
        let mut verify = translate.clone();
        verify.concurrency = Some(2);
        verify.batch_dialogue_limit = None;
        assert!(same_service(&translate, &verify));

        let mut other_thinking = verify.clone();
        other_thinking.thinking = Some(ThinkingLevel::Low);
        assert!(!same_service(&translate, &other_thinking));
        assert!(!same_service(
            &translate,
            &sample(Provider::Anthropic, "claude-haiku-5-5")
        ));
    }

    #[test]
    fn who_names_the_provider_or_the_server() {
        assert_eq!(
            who(&sample(Provider::Anthropic, "claude-sonnet-5-5")),
            "Anthropic"
        );
        assert_eq!(who(&sample(Provider::Custom, "x")), "The server");
    }

    /// The source every test episode is made from.
    const SRC: &str = "[Events]\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,你好\n";

    fn meta() -> RunMeta {
        RunMeta {
            source_lang: "zh".into(),
            target_lang: "en".into(),
            tone: Tone::Xianxia,
        }
    }

    fn outcome(file: &str, end: FileEnd, translations: &[(u32, &str)]) -> FileOutcome {
        let finished = matches!(end, FileEnd::Done | FileEnd::NeedsLook);
        FileOutcome {
            result: FileResult {
                file: file.into(),
                success: finished,
                total_lines: 3,
                translated_lines: if matches!(end, FileEnd::Failed { .. }) {
                    0
                } else {
                    translations.len() as u32
                },
                has_warnings: end == FileEnd::NeedsLook,
                issues: if end == FileEnd::NeedsLook {
                    vec![VerifyIssue {
                        line_id: 3,
                        source: "s".into(),
                        translation: String::new(),
                        issue_type: "untranslated".into(),
                        description: "d".into(),
                        severity: "warning".into(),
                        term: None,
                        expected: None,
                    }]
                } else {
                    Vec::new()
                },
                output_path: None,
            },
            output_name: finished.then(|| format!("{file}.out")),
            end,
            translations: translations
                .iter()
                .map(|(id, t)| (*id, t.to_string()))
                .collect(),
            source_hash: Some(crate::translation::results::fingerprint(SRC)),
            output_hash: None,
        }
    }

    #[test]
    fn stored_outcomes_are_done_needs_look_failed_or_stopped() {
        let data = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let store = ResultsStore::at(data.path());
        let stop = RunStop::default();
        stop.by_error(StopCause {
            kind: ErrorKind::Auth,
            step: Step::Translate,
            who: "Anthropic".into(),
            model: "m".into(),
        });

        let cases = [
            outcome("done.ass", FileEnd::Done, &[(1, "a"), (2, "b"), (3, "c")]),
            outcome("look.ass", FileEnd::NeedsLook, &[(1, "a"), (2, "b")]),
            outcome("stopped.ass", FileEnd::Stopped, &[(1, "a")]),
            outcome(
                "failed.ass",
                FileEnd::Failed {
                    reason: "This file has no dialogue lines.".into(),
                    kind: ErrorKind::Other,
                },
                &[],
            ),
        ];
        for o in &cases {
            store_outcome(&store, folder.path(), o, &meta(), &stop).unwrap();
        }
        let results = store.results(folder.path());
        let get = |f: &str| results.episodes[f].clone();

        assert_eq!(get("done.ass").outcome, EpisodeOutcome::Done);
        assert_eq!(get("done.ass").output_name.as_deref(), Some("done.ass.out"));
        assert_eq!(get("look.ass").outcome, EpisodeOutcome::NeedsLook);
        assert_eq!(get("look.ass").issues.len(), 1);

        let stopped = get("stopped.ass");
        assert_eq!(stopped.outcome, EpisodeOutcome::Stopped);
        assert_eq!(
            stopped.reason.as_deref(),
            Some("Anthropic rejected the API key used for Translate.")
        );
        assert_eq!((stopped.translated_lines, stopped.total_lines), (1, 3));
        assert_eq!(stopped.output_name, None);
        let partial = store.lines(folder.path(), "stopped.ass").unwrap();
        assert_eq!(partial.translations.len(), 1);
        assert_eq!(
            (partial.source_lang.as_str(), partial.tone),
            ("zh", Tone::Xianxia)
        );

        let failed = get("failed.ass");
        assert_eq!(failed.outcome, EpisodeOutcome::Failed);
        assert_eq!(
            failed.reason.as_deref(),
            Some("This file has no dialogue lines.")
        );
        assert!(store.lines(folder.path(), "failed.ass").is_none());
    }

    #[test]
    fn a_user_stop_is_stored_as_such() {
        let data = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let store = ResultsStore::at(data.path());
        let stop = RunStop::default();
        stop.by_user();
        store_outcome(
            &store,
            folder.path(),
            &outcome("ep.ass", FileEnd::Stopped, &[]),
            &meta(),
            &stop,
        )
        .unwrap();
        assert_eq!(
            store
                .record(folder.path(), "ep.ass")
                .unwrap()
                .reason
                .as_deref(),
            Some("You stopped the run.")
        );
    }

    #[test]
    fn resume_seeds_only_stopped_files() {
        let data = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let store = ResultsStore::at(data.path());
        let stop = RunStop::default();
        stop.by_user();
        for o in [
            outcome("stopped.ass", FileEnd::Stopped, &[(1, "a"), (2, "b")]),
            outcome("done.ass", FileEnd::Done, &[(1, "a"), (2, "b"), (3, "c")]),
        ] {
            store_outcome(&store, folder.path(), &o, &meta(), &stop).unwrap();
        }
        std::fs::write(folder.path().join("stopped.ass"), SRC).unwrap();
        let files: Vec<String> = ["stopped.ass", "done.ass", "new.ass"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let seeds = resume_seeds(&store, folder.path(), &files, &meta());
        assert_eq!(seeds.keys().collect::<Vec<_>>(), ["stopped.ass"]);
        assert_eq!(
            seeds["stopped.ass"].source_hash,
            crate::translation::results::fingerprint(SRC)
        );
        assert_eq!(seeds["stopped.ass"].translations.len(), 2);
        // A source that changed since keeps nothing: it starts over.
        std::fs::write(folder.path().join("stopped.ass"), "[Events]\n").unwrap();
        assert!(resume_seeds(&store, folder.path(), &files, &meta()).is_empty());
    }

    #[test]
    fn resume_starts_over_in_another_language_or_tone() {
        let data = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let store = ResultsStore::at(data.path());
        let stop = RunStop::default();
        stop.by_user();
        let stopped = outcome("stopped.ass", FileEnd::Stopped, &[(1, "a")]);
        store_outcome(&store, folder.path(), &stopped, &meta(), &stop).unwrap();
        std::fs::write(folder.path().join("stopped.ass"), SRC).unwrap();
        let files = vec!["stopped.ass".to_string()];
        assert_eq!(resume_seeds(&store, folder.path(), &files, &meta()).len(), 1);

        let other_target = RunMeta { target_lang: "fr".into(), ..meta() };
        let other_source = RunMeta { source_lang: "ja".into(), ..meta() };
        let other_tone = RunMeta { tone: Tone::Standard, ..meta() };
        for run in [other_target, other_source, other_tone] {
            assert!(resume_seeds(&store, folder.path(), &files, &run).is_empty(), "{run:?}");
        }
    }

    #[test]
    fn review_needs_a_finished_stored_episode() {
        let data = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let store = ResultsStore::at(data.path());
        let stop = RunStop::default();
        let err = |r: AppResult<ReviewPlan>| r.err().map(|e| e.to_string()).unwrap_or_default();

        assert!(err(plan_review(&store, folder.path(), "ep.ass", vec![1]))
            .contains("Translate it first"));
        store_outcome(
            &store,
            folder.path(),
            &outcome("stopped.ass", FileEnd::Stopped, &[(1, "a")]),
            &meta(),
            &stop,
        )
        .unwrap();
        assert!(
            err(plan_review(&store, folder.path(), "stopped.ass", vec![1]))
                .contains("no finished translation")
        );

        store_outcome(
            &store,
            folder.path(),
            &outcome("look.ass", FileEnd::NeedsLook, &[(1, "a")]),
            &meta(),
            &stop,
        )
        .unwrap();
        assert!(err(plan_review(&store, folder.path(), "look.ass", vec![]))
            .contains("at least one line"));
        assert!(
            err(plan_review(&store, folder.path(), "look.ass", vec![3])).contains("Couldn't read")
        );
        std::fs::write(folder.path().join("look.ass"), "[Events]\n").unwrap();
        assert!(
            err(plan_review(&store, folder.path(), "look.ass", vec![3])).contains("changed since")
        );
        std::fs::write(folder.path().join("look.ass"), SRC).unwrap();
        let plan = plan_review(&store, folder.path(), "look.ass", vec![3]).unwrap();
        assert!(plan.needs_look);
        assert_eq!(plan.output_name, "look.ass.out");
        assert_eq!(plan.issues.len(), 1);

        // The output was edited by hand since it was written: not overwritten.
        std::fs::write(folder.path().join("look.ass.out"), "fixed by hand").unwrap();
        store
            .edit_episode(folder.path(), "look.ass", |_, lines| {
                lines.as_mut().unwrap().output_hash =
                    Some(crate::translation::results::fingerprint_bytes(b"as written"));
                Ok(())
            })
            .unwrap();
        assert!(err(plan_review(&store, folder.path(), "look.ass", vec![3]))
            .starts_with("look.ass.out was changed outside Polygluttony"));
    }

    #[tokio::test]
    async fn a_stopped_run_leaves_unstarted_files_pending_and_says_why_once() {
        let stop = RunStop::default();
        let (tx, mut rx) = mpsc::channel(64);
        let driver = crate::llm::test_support::ScriptedDriver::new(vec![]);
        let svc = Arc::new(LlmService::new(driver, 1, stop.token().clone(), tx.clone()));
        let kit = Kit {
            stop: stop.clone(),
            tx,
            svc: svc.clone(),
            verify_svc: svc,
            glossary: Arc::default(),
            batch_limit: None,
            concurrency: 1,
        };
        let files: Vec<String> = ["a.ass", "b.ass", "c.ass"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let doomed = stop.clone();
        spawn_files(
            kit,
            "/work".into(),
            true,
            files,
            |_, _| {},
            move |name, _svc, _verify, tx| {
                let stop = doomed.clone();
                async move {
                    // The first file meets a dead key: the run stops.
                    stop.by_error(StopCause {
                        kind: ErrorKind::Auth,
                        step: Step::Translate,
                        who: "Anthropic".into(),
                        model: "m".into(),
                    });
                    let _ = tx
                        .send(RunEvent::State {
                            file: name.clone(),
                            state: FileStateKind::Stopped,
                            detail: None,
                        })
                        .await;
                    FileResult {
                        file: name,
                        success: false,
                        total_lines: 3,
                        translated_lines: 1,
                        has_warnings: false,
                        issues: Vec::new(),
                        output_path: None,
                    }
                }
            },
        );
        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        let tags: Vec<String> = events
            .iter()
            .map(|e| match e {
                RunEvent::State { file, state, .. } => format!("{file} {state:?}"),
                RunEvent::Stopped { info } => format!("stopped: {}", info.message),
                RunEvent::RunStarted {
                    folder,
                    files,
                    resume,
                } => {
                    format!("started {folder} {files:?} resume={resume}")
                }
                RunEvent::RunFinished { results } => format!(
                    "finished {:?}",
                    results.iter().map(|r| r.file.as_str()).collect::<Vec<_>>()
                ),
                other => format!("{other:?}"),
            })
            .collect();
        // b and c never started: no events, no results; they stay pending.
        assert_eq!(
            tags,
            [
                r#"started /work ["a.ass", "b.ass", "c.ass"] resume=true"#,
                "a.ass Stopped",
                "stopped: Anthropic rejected the API key used for Translate.",
                "finished [\"a.ass\"]",
            ]
        );
    }
}
