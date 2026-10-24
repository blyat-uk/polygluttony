//! Glossary build pipeline (O10): the build orchestrator (`build_glossary`)
//! plus the `glossary_batches` slicer shared with the reference extractor.
//!
//! Port of `core/glossary_builder.py:build_from_files` (68-233) with the
//! UI-spec deviations: world type arrives pre-detected from the UI (the build
//! NEVER re-detects), reference terminology is loaded/extracted inline (O11),
//! and every incremental save merges with the existing glossary first — fixing
//! the Python data-loss window where `glossary_phase.py:104-105` wrote the
//! new-terms-only glossary over a prior on-disk one.
//!
//! ## Failure philosophy (hard requirement)
//! Batch failures NEVER abort the build. An auth error stops the *remaining*
//! batches (via the cancel token) but the build still finalizes: merge what
//! completed → save → `Done { aborted: true }`. User cancel takes the same
//! finalize path with `cancelled: true`. Partial glossary > no glossary. The
//! only `Error` event is a final-save IO failure.

use std::path::PathBuf;

use futures::stream::{FuturesOrdered, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use crate::ass::{decode::decode_file, parse::parse_dialogues, tags::strip_for_text};
use crate::events::{GlossaryBuildSummary, GlossaryEvent, GlossaryPhase, LogLevel};
use crate::glossary::diff::GlossaryDiff;
use crate::glossary::io::{load_folder_glossary, save_folder_glossary};
use crate::glossary::model::Glossary;
use crate::glossary::{normalize, personalize, prompts, reference};
use crate::llm::service::LlmService;
use crate::llm::LlmRequest;
use crate::models::language_pair::LanguagePair;
use crate::translation::parse_response;

/// Slice a cross-file line stream into batches of `limit × 0.7` lines
/// (`glossary_builder.py:136-138,235-241`; `reference_extractor.py:65-67,124-137`).
/// The 30% headroom leaves room for prompt overhead.
pub fn glossary_batches(lines: &[String], batch_limit: Option<u32>) -> Vec<String> {
    let limit = batch_limit.unwrap_or(crate::translation::batching::BATCH_LINE_LIMIT);
    let per = (((limit as f64) * 0.7) as usize).max(1);
    lines.chunks(per).map(|c| c.join("\n")).collect()
}

/// Whether a build merges into the glossary already on disk or replaces it.
///
/// `Append` is the everyday case: existing terms win, new terms fill gaps.
/// `Regenerate` starts from nothing, which makes it byte-for-byte the
/// first-time build path — and destructive, so `run::start` takes a backup
/// first (see `io::backup_folder_glossary`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum BuildMode {
    Append,
    Regenerate,
}

/// Everything O10 needs from the UI. The command layer assembles this from the
/// project state + Glossary view options.
pub struct BuildJob {
    pub folder: PathBuf,
    /// Merge base selector — see `BuildMode`.
    pub mode: BuildMode,
    /// File NAMES relative to `folder` (the Project view's `selected_files`).
    pub files: Vec<String>,
    /// EFFECTIVE world (override ?? detected) — the build never re-detects.
    pub world_type: String,
    pub pair: LanguagePair,
    pub normalize: bool,
    pub personalize: bool,
    pub personalize_context: String,
    pub prompts: crate::prompts::GlossaryPrompts,
    pub batch_limit: Option<u32>,
    pub cancel: CancellationToken,
}

/// Existing terms always win; `new_terms` only fills gaps. Every save (the
/// incremental ones included) goes through this so a pre-existing glossary can
/// never be clobbered by a partial build.
pub(crate) fn merged_with_existing(existing: Option<&Glossary>, new_terms: &Glossary) -> Glossary {
    match existing {
        Some(e) => {
            let mut out = e.clone();
            out.merge_first_wins(new_terms);
            out
        }
        None => new_terms.clone(),
    }
}

async fn phase(tx: &mpsc::Sender<GlossaryEvent>, p: GlossaryPhase, detail: Option<String>) {
    let _ = tx.send(GlossaryEvent::Phase { phase: p, detail }).await;
}

async fn log(tx: &mpsc::Sender<GlossaryEvent>, level: LogLevel, message: String) {
    let _ = tx.send(GlossaryEvent::Log { level, message }).await;
}

/// O10 build orchestrator. Always emits `Done {summary}` when a result exists
/// (including aborted/cancelled/no-text); the only `Error` event is a
/// final-save IO failure (the last incremental save remains on disk then).
pub async fn build_glossary(
    job: BuildJob,
    svc: &LlmService,
    norm_svc: &LlmService,
    personalize_svc: Option<&LlmService>,
    tx: mpsc::Sender<GlossaryEvent>,
) {
    // The diff is always against what was on disk before the run — including a
    // regenerate, where the merge base is deliberately empty but the user still
    // needs to see what they lost. `existing` stays the MERGE base only (every
    // incremental save + the final merge); `run::start` has already copied the
    // old file to glossary.prev.json.
    let diff_base = load_folder_glossary(&job.folder);
    let existing = match job.mode {
        BuildMode::Append => diff_base.clone(),
        BuildMode::Regenerate => None,
    };

    // ── Loading: decode + parse + strip each selected file ─────────────────
    phase(&tx, GlossaryPhase::Loading, None).await;
    let mut all_lines: Vec<String> = Vec::new();
    let mut files_processed = 0u32;
    for name in &job.files {
        match decode_file(&job.folder.join(name)) {
            Ok(text) => {
                let dialogues = parse_dialogues(&text);
                if dialogues.is_empty() {
                    log(&tx, LogLevel::Warning, format!("no dialogue text in {name} — skipped"))
                        .await;
                    continue;
                }
                let n = dialogues.len();
                all_lines.extend(dialogues.iter().map(|d| strip_for_text(&d.text)));
                files_processed += 1;
                log(&tx, LogLevel::Info, format!("loaded {name} ({n} lines)")).await;
            }
            Err(e) => {
                log(&tx, LogLevel::Warning, format!("error loading {name}: {e} — skipped")).await;
            }
        }
    }

    if all_lines.is_empty() {
        // Nothing to extract from: report (never silently no-op) and write
        // nothing — `glossary_builder.py:121-126`.
        //
        // This path writes NOTHING, so the on-disk glossary is still
        // `diff_base` and the diff must say "no changes". Diffing `result`
        // (empty for a regenerate) against `diff_base` would report a phantom
        // wipe of a glossary that is sitting intact on disk.
        let result = merged_with_existing(existing.as_ref(), &Glossary::new(&job.world_type));
        let persisted = diff_base.clone().unwrap_or_else(|| Glossary::new(&job.world_type));
        let summary = GlossaryBuildSummary {
            world_type: job.world_type.clone(),
            files_processed,
            batches_processed: 0,
            batches_total: 0,
            terms_extracted: 0,
            terms_final: result.count() as u32,
            normalized: false,
            personalized: false,
            aborted: false,
            cancelled: false,
            errors: vec!["No text found in files".into()],
            diff: GlossaryDiff::compute(diff_base.as_ref(), &persisted),
        };
        let _ = tx.send(GlossaryEvent::Done { summary }).await;
        return;
    }

    // ── Reference: advisory English terminology (O11, logs for itself) ─────
    phase(&tx, GlossaryPhase::Reference, None).await;
    let (reference_terms, reference_errors) =
        reference::load_or_extract(&job.folder, svc, job.batch_limit, &tx, &job.prompts.reference)
            .await;

    // ── Extracting: all batches through the LLM, one shared system prompt ──
    let batches = glossary_batches(&all_lines, job.batch_limit);
    let total = batches.len() as u32;
    phase(&tx, GlossaryPhase::Extracting, Some(format!("{total} batches"))).await;
    let _ = tx.send(GlossaryEvent::Progress { done: 0, total }).await;

    let system = prompts::extraction_prompt(
        &job.prompts.extract,
        &job.world_type,
        &job.pair,
        reference_terms.as_ref(),
    );

    // Progress is emitted by each future ON COMPLETION (shared atomic counter)
    // so the bar moves the moment ANY batch finishes — results are still
    // CONSUMED in batch order below for deterministic first-wins merging.
    // Near-simultaneous completions can deliver counts out of order (the
    // frontend store clamps these — see the UX-overhaul store changes).
    // FuturesOrdered: structured concurrency (drop-cancellation, no 'static
    // bound, no panic arm); strictly-in-order consumption means a slow batch 1
    // head-of-line-blocks incremental SAVES of later batches — deterministic
    // merges were chosen over completion-order saves.
    // The LlmService bounds the actual parallelism via its permits.
    let completed = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    // Shared seen-set for the LIVE console only: lets each batch stream its NEW
    // terms the moment it COMPLETES (any order), so a slow batch 1 never
    // head-of-line-blocks the stream the way in-order consumption does. The
    // authoritative first-wins merge still happens in batch order below; this is
    // purely for the live "terms streaming into lanes" feedback.
    let seen = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::BTreeSet::<String>::new()));
    let world = job.world_type.clone();
    let mut futs: FuturesOrdered<_> = batches
        .iter()
        .map(|batch| {
            let req = LlmRequest {
                system: system.clone(),
                user: prompts::extraction_user_prompt(batch),
            };
            let tx = tx.clone();
            let completed = completed.clone();
            let seen = seen.clone();
            let world = world.clone();
            async move {
                let result = svc.request(req).await;
                let done = completed.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                let _ = tx.send(GlossaryEvent::Progress { done, total }).await;
                if let Ok(resp) = &result {
                    if let Ok(v) = parse_response::extract_object(&resp.text) {
                        let bg = Glossary::from_terms_value(&v, &world);
                        let mut hits: Vec<crate::events::TermHit> = Vec::new();
                        {
                            let mut seen = seen.lock().await;
                            for c in crate::glossary::model::CATEGORIES {
                                for (src, tgt) in bg.category(c) {
                                    if seen.insert(src.clone()) {
                                        hits.push(crate::events::TermHit {
                                            category: c.to_string(),
                                            source: src.clone(),
                                            target: tgt.clone(),
                                        });
                                    }
                                }
                            }
                        }
                        if !hits.is_empty() {
                            let _ = tx.send(GlossaryEvent::Terms { batch: done, hits }).await;
                        }
                    }
                }
                result
            }
        })
        .collect();

    let glossary_path = job.folder.join("glossary.json");
    let mut new_terms = Glossary::new(&job.world_type);
    let mut errors: Vec<String> = reference_errors;
    let mut terms_extracted = 0u32;
    let mut batches_processed = 0u32;
    let mut aborted = false;
    let mut consumed = 0u32;

    while let Some(result) = futs.next().await {
        consumed += 1;
        let n = consumed;
        match result {
            Ok(resp) => {
                // Unparseable 2xx = empty glossary but still "processed"
                // (Python parity: parse failure is logged, not an error).
                let batch_glossary = match parse_response::extract_object(&resp.text) {
                    Ok(v) => Glossary::from_terms_value(&v, &job.world_type),
                    Err(e) => {
                        log(
                            &tx,
                            LogLevel::Warning,
                            format!("batch {n}/{total}: unparseable response ({e})"),
                        )
                        .await;
                        Glossary::new(&job.world_type)
                    }
                };
                // Per-batch count BEFORE cross-batch dedupe (Python parity).
                let count = batch_glossary.count() as u32;
                terms_extracted += count;
                new_terms.merge_first_wins(&batch_glossary);
                batches_processed += 1;
                // Crash-safe incremental save: always merged-with-existing
                // (the Python bug wrote new-terms-only), never an empty file.
                let snapshot = merged_with_existing(existing.as_ref(), &new_terms);
                if !snapshot.is_empty() {
                    if let Err(e) = save_folder_glossary(&job.folder, &snapshot) {
                        log(
                            &tx,
                            LogLevel::Warning,
                            format!(
                                "incremental save of {} failed: {e}",
                                glossary_path.display()
                            ),
                        )
                        .await;
                    }
                }
                log(&tx, LogLevel::Info, format!("batch {n}/{total}: {count} terms")).await;
            }
            Err(e) if e.is_auth() && !aborted => {
                // Retrying won't fix credentials: stop the REMAINING batches,
                // but keep consuming results — completed work is still merged
                // and saved below (hard requirement: partial > none).
                aborted = true;
                job.cancel.cancel();
                log(
                    &tx,
                    LogLevel::Warning,
                    format!("batch {n}/{total}: auth error — stopping remaining batches"),
                )
                .await;
                errors.push(format!(
                    "batch {n}/{total}: auth error, remaining batches stopped ({e})"
                ));
            }
            Err(e) => {
                let noise = (aborted || job.cancel.is_cancelled()) && e.is_cancelled();
                if !noise {
                    log(&tx, LogLevel::Warning, format!("batch {n}/{total} failed: {e}")).await;
                    errors.push(format!("batch {n}/{total} failed: {e}"));
                }
            }
        }
    }

    new_terms.deduplicate();

    // A user cancel can land at ANY point after the extraction loop too, so
    // every gate below reads the token FRESH instead of a value latched at
    // loop exit (which would misreport a mid-normalize/personalize cancel).

    // ── Normalizing: NEW terms only, before the merge (build step 6) ────────
    let mut normalized = false;
    if job.normalize && !aborted && !job.cancel.is_cancelled() && !new_terms.is_empty() {
        phase(
            &tx,
            GlossaryPhase::Normalizing,
            Some(format!("{} new terms", new_terms.count())),
        )
        .await;
        new_terms = normalize::normalize_pass(norm_svc, &new_terms, &tx, &job.prompts.normalize).await;
        // A cancel mid-normalize reports normalized=false even though some
        // categories may already have been normalized (each keeps its original
        // terms on failure, so the data is valid either way) — conservative
        // reporting beats claiming a full pass.
        normalized = !job.cancel.is_cancelled();
    }

    // ── Merge: existing terms win (build step 7) ────────────────────────────
    let mut result = merged_with_existing(existing.as_ref(), &new_terms);

    // ── Personalizing: one call on the web-capable connection (step 8) ──────
    let mut personalized = false;
    if job.personalize && !aborted && !job.cancel.is_cancelled() && !result.is_empty() {
        if let Some(p_svc) = personalize_svc {
            phase(&tx, GlossaryPhase::Personalizing, None).await;
            match personalize::personalize_pass(
                p_svc,
                &result,
                &job.personalize_context,
                &job.prompts.personalize,
            )
            .await
            {
                Ok(g) => {
                    result = g;
                    personalized = true;
                }
                // A failure caused by a cancel landing mid-call is a
                // consequence of the stop, not a cause — suppress it
                // (`aborted` is always false here: the gate excludes it).
                Err(_) if job.cancel.is_cancelled() => {}
                Err(reason) => errors.push(reason),
            }
        }
    }

    // ── Saving: final write; empty result writes nothing, ever ──────────────
    phase(&tx, GlossaryPhase::Saving, None).await;
    if !result.is_empty() {
        if let Err(e) = save_folder_glossary(&job.folder, &result) {
            // The ONLY Error event in the build. The last incremental save
            // remains on disk.
            let _ = tx
                .send(GlossaryEvent::Error {
                    message: format!("could not save {}: {e}", glossary_path.display()),
                })
                .await;
            return;
        }
    }

    // Both saves are guarded by !is_empty(), so an empty result means nothing was
    // written and the on-disk glossary is still diff_base. Diffing result against
    // diff_base there would report a phantom wipe of a glossary that is intact.
    let persisted = if result.is_empty() {
        diff_base.clone().unwrap_or_else(|| Glossary::new(&job.world_type))
    } else {
        result.clone()
    };

    // Recompute NOW, not at extraction-loop exit: a cancel that arrived during
    // normalize/personalize must still be reported as a cancel.
    let cancelled = job.cancel.is_cancelled() && !aborted;
    let summary = GlossaryBuildSummary {
        world_type: job.world_type,
        files_processed,
        batches_processed,
        batches_total: total,
        terms_extracted,
        terms_final: result.count() as u32,
        normalized,
        personalized,
        aborted,
        cancelled,
        errors,
        diff: GlossaryDiff::compute(diff_base.as_ref(), &persisted),
    };
    let _ = tx.send(GlossaryEvent::Done { summary }).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("line {i}")).collect()
    }

    #[test]
    fn batches_slice_at_seventy_percent() {
        // limit 10 → 7 lines per batch → 15 lines = 7 + 7 + 1.
        let b = glossary_batches(&lines(15), Some(10));
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].lines().count(), 7);
        assert_eq!(b[2], "line 14");
    }

    #[test]
    fn batches_floor_at_one_line_and_default_limit() {
        let b = glossary_batches(&lines(3), Some(1)); // 0.7 → floor 1
        assert_eq!(b.len(), 3);
        // Default = BATCH_LINE_LIMIT (260) → 182 per batch.
        let b = glossary_batches(&lines(183), None);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].lines().count(), 182);
    }

    #[test]
    fn empty_lines_give_no_batches() {
        assert!(glossary_batches(&[], Some(10)).is_empty());
    }

    // ── O10 orchestrator tests ──────────────────────────────────────────────

    use crate::events::GlossaryEvent;
    use crate::glossary::io::load_folder_glossary;
    use crate::glossary::model::Glossary;
    use crate::llm::error::LlmError;
    use crate::llm::service::LlmService;
    use crate::llm::test_support::ScriptedDriver;
    use crate::models::language_pair::LanguagePair;
    use std::path::Path;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    fn svc1(driver: Arc<ScriptedDriver>, cancel: CancellationToken) -> LlmService {
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        // cap 1 → batches hit the driver in batch order (deterministic scripts).
        LlmService::new(driver, 1, cancel, tx)
    }

    fn write_ass(dir: &Path, name: &str, lines: &[&str]) {
        let mut content = String::from(
            "[Script Info]\nTitle: t\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
        );
        for (i, l) in lines.iter().enumerate() {
            content.push_str(&format!(
                "Dialogue: 0,0:00:0{i}.00,0:00:0{}.00,Default,,0,0,0,,{l}\n",
                i + 1
            ));
        }
        std::fs::write(dir.join(name), content).unwrap();
    }

    fn job(dir: &Path, files: Vec<String>, cancel: CancellationToken) -> BuildJob {
        BuildJob {
            folder: dir.to_path_buf(),
            mode: BuildMode::Append,
            files,
            world_type: "xianxia".into(),
            pair: LanguagePair::from_codes("zh", "en").unwrap(),
            normalize: false,
            personalize: false,
            personalize_context: String::new(),
            prompts: crate::prompts::GlossaryPrompts::defaults(),
            batch_limit: Some(2), // ×0.7 → 1 line per batch
            cancel,
        }
    }

    /// Drain the channel after build returns; the Done summary must be last.
    async fn run_and_collect(
        job: BuildJob,
        svc: &LlmService,
        p_svc: Option<&LlmService>,
    ) -> (Vec<GlossaryEvent>, crate::events::GlossaryBuildSummary) {
        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        build_glossary(job, svc, svc, p_svc, tx).await;
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        let summary = events
            .iter()
            .rev()
            .find_map(|e| match e {
                GlossaryEvent::Done { summary } => Some(summary.clone()),
                _ => None,
            })
            .expect("build must emit Done");
        (events, summary)
    }

    #[tokio::test(start_paused = true)]
    async fn happy_path_merges_first_wins_in_index_order_and_saves() {
        let dir = tempfile::tempdir().unwrap();
        write_ass(dir.path(), "e1.ass", &["修仙第一句", "修仙第二句"]); // 2 lines → 2 batches
        let cancel = CancellationToken::new();
        let d = ScriptedDriver::new(vec![
            Ok(r#"{"terms":{"characters":{"林动":"Lin Dong"}}}"#.into()),
            // Batch 2 re-extracts 林动 with a different value — batch 1 wins.
            Ok(r#"{"characters":{"林动":"DUPE"},"locations":{"青阳镇":"Qingyang Town"}}"#.into()),
        ]);
        let svc = svc1(d, cancel.clone());
        let (_events, s) =
            run_and_collect(job(dir.path(), vec!["e1.ass".into()], cancel), &svc, None).await;

        assert_eq!(s.batches_total, 2);
        assert_eq!(s.batches_processed, 2);
        assert_eq!(s.files_processed, 1);
        assert_eq!(s.terms_extracted, 3); // 1 + 2, pre cross-batch dedupe
        assert_eq!(s.terms_final, 2);
        assert!(!s.aborted && !s.cancelled && s.errors.is_empty());
        assert_eq!(s.world_type, "xianxia");
        assert_eq!(s.diff.total_added, 2);

        let saved = load_folder_glossary(dir.path()).unwrap();
        assert_eq!(saved.characters.get("林动").unwrap(), "Lin Dong"); // first wins
        assert_eq!(saved.locations.get("青阳镇").unwrap(), "Qingyang Town");
        assert_eq!(saved.world_type, "xianxia");
    }

    #[tokio::test(start_paused = true)]
    async fn batch_failure_is_partial_never_fatal() {
        let dir = tempfile::tempdir().unwrap();
        write_ass(dir.path(), "e1.ass", &["一", "二"]);
        let cancel = CancellationToken::new();
        let d = ScriptedDriver::new(vec![
            Err(LlmError::Http { status: 400, body: "bad request".into(), retry_after: None }), // batch 1: non-retryable, one call
            Ok(r#"{"characters":{"林动":"Lin Dong"}}"#.into()),              // batch 2 ok
        ]);
        let svc = svc1(d, cancel.clone());
        let (_events, s) =
            run_and_collect(job(dir.path(), vec!["e1.ass".into()], cancel), &svc, None).await;

        assert!(!s.aborted, "non-auth failures must not abort");
        assert_eq!(s.batches_processed, 1);
        assert_eq!(s.errors.len(), 1);
        assert!(s.errors[0].contains("batch 1"));
        assert_eq!(s.terms_final, 1); // half a glossary > no glossary
        assert!(load_folder_glossary(dir.path()).is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn auth_error_aborts_remaining_but_keeps_partial_and_existing() {
        let dir = tempfile::tempdir().unwrap();
        // Existing glossary on disk — must survive verbatim (crash-safety fix).
        let mut existing = Glossary::new("xianxia");
        existing.characters.insert("应欢欢".into(), "Ying Huanhuan".into());
        crate::glossary::io::save_folder_glossary(dir.path(), &existing).unwrap();

        write_ass(dir.path(), "e1.ass", &["一", "二"]);
        let cancel = CancellationToken::new();
        let d = ScriptedDriver::new(vec![
            Ok(r#"{"characters":{"林动":"Lin Dong"}}"#.into()), // batch 1 ok
            Err(LlmError::Http { status: 401, body: "bad key".into(), retry_after: None }), // batch 2: auth, no retry
        ]);
        let svc = svc1(d, cancel.clone());
        let (_events, s) =
            run_and_collect(job(dir.path(), vec!["e1.ass".into()], cancel), &svc, None).await;

        assert!(s.aborted);
        assert!(!s.cancelled);
        assert_eq!(s.errors.len(), 1, "only the auth error, no cancel noise: {:?}", s.errors);
        assert_eq!(s.batches_processed, 1);
        let saved = load_folder_glossary(dir.path()).unwrap();
        assert_eq!(saved.characters.get("应欢欢").unwrap(), "Ying Huanhuan"); // existing preserved
        assert_eq!(saved.characters.get("林动").unwrap(), "Lin Dong"); // partial kept
        assert_eq!(s.diff.total_added, 1); // diff vs pre-build state
    }

    #[tokio::test(start_paused = true)]
    async fn auth_abort_suppresses_queued_cancel_noise() {
        let dir = tempfile::tempdir().unwrap();
        write_ass(dir.path(), "e1.ass", &["一", "二", "三"]); // limit 2 → 3 batches
        let cancel = CancellationToken::new();
        // Only ONE batch ever reaches the driver (cap 1 + FuturesOrdered): its
        // auth error trips the cancel token, so the two queued batches fail
        // inside the service with CANCELLED_MSG without consuming script
        // entries — and that noise must NOT appear in summary.errors.
        let d = ScriptedDriver::new(vec![Err(LlmError::Http {
            status: 401,
            body: "bad key".into(),
            retry_after: None,
        })]);
        let svc = svc1(d, cancel.clone());
        let (_events, s) =
            run_and_collect(job(dir.path(), vec!["e1.ass".into()], cancel), &svc, None).await;

        assert!(s.aborted);
        assert_eq!(s.batches_processed, 0);
        assert_eq!(s.batches_total, 3);
        assert_eq!(s.errors.len(), 1, "queued-batch noise suppressed: {:?}", s.errors);
        assert!(s.errors[0].contains("auth error"), "{}", s.errors[0]);
    }

    /// The ONLY `Error` event in the build: the final save fails. Unix-only —
    /// a read+exec (0o555) folder makes every write fail: incremental saves
    /// degrade to warnings, the final save emits `Error`, and no `Done` follows.
    #[cfg(unix)]
    #[tokio::test(start_paused = true)]
    async fn final_save_failure_emits_error_and_no_done() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        write_ass(dir.path(), "e1.ass", &["一"]); // 1 line → 1 batch
        let cancel = CancellationToken::new();
        let d = ScriptedDriver::new(vec![Ok(r#"{"characters":{"林动":"Lin Dong"}}"#.into())]);
        let svc = svc1(d, cancel.clone());

        // Strip write permission AFTER writing the .ass so all saves fail.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        build_glossary(job(dir.path(), vec!["e1.ass".into()], cancel), &svc, &svc, None, tx).await;

        // Restore BEFORE asserting so the tempdir can clean itself up.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        let error_msgs: Vec<&String> = events
            .iter()
            .filter_map(|e| match e {
                GlossaryEvent::Error { message } => Some(message),
                _ => None,
            })
            .collect();
        assert_eq!(error_msgs.len(), 1, "exactly one Error event: {events:?}");
        assert!(error_msgs[0].contains("could not save"), "{}", error_msgs[0]);
        assert!(
            error_msgs[0].contains("glossary.json"),
            "Error names the path: {}",
            error_msgs[0]
        );
        assert!(
            !events.iter().any(|e| matches!(e, GlossaryEvent::Done { .. })),
            "no Done after a final-save Error"
        );
        // The incremental save degraded to a path-bearing warning on the way.
        assert!(
            events.iter().any(|e| matches!(
                e,
                GlossaryEvent::Log { level: crate::events::LogLevel::Warning, message }
                    if message.contains("incremental save") && message.contains("glossary.json")
            )),
            "incremental-save warning expected: {events:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_finalizes_with_partial_counts() {
        let dir = tempfile::tempdir().unwrap();
        write_ass(dir.path(), "e1.ass", &["一", "二"]);
        let cancel = CancellationToken::new();
        cancel.cancel(); // user cancelled immediately
        let d = ScriptedDriver::new(vec![]); // service errors before reaching the driver
        let svc = svc1(d, cancel.clone());
        let (_events, s) =
            run_and_collect(job(dir.path(), vec!["e1.ass".into()], cancel), &svc, None).await;
        assert!(s.cancelled && !s.aborted);
        assert_eq!(s.batches_processed, 0);
        assert!(s.errors.is_empty(), "cancel noise suppressed: {:?}", s.errors);
        assert!(load_folder_glossary(dir.path()).is_none(), "nothing written");
    }

    #[tokio::test(start_paused = true)]
    async fn no_text_emits_done_with_error_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bad.ass"), "not an ass file").unwrap();
        let cancel = CancellationToken::new();
        let d = ScriptedDriver::new(vec![]);
        let svc = svc1(d, cancel.clone());
        let (_events, s) = run_and_collect(
            job(dir.path(), vec!["bad.ass".into(), "missing.ass".into()], cancel),
            &svc,
            None,
        )
        .await;
        assert_eq!(s.files_processed, 0);
        assert_eq!(s.errors, vec!["No text found in files".to_string()]);
        assert!(load_folder_glossary(dir.path()).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn progress_is_emitted_per_completion_with_full_coverage() {
        let dir = tempfile::tempdir().unwrap();
        write_ass(dir.path(), "e1.ass", &["一", "二", "三"]); // 3 batches at limit 2
        let cancel = CancellationToken::new();
        let d = ScriptedDriver::new(vec![
            Ok(r#"{"characters":{"a":"A"}}"#.into()),
            Ok(r#"{"characters":{"b":"B"}}"#.into()),
            Ok(r#"{"characters":{"c":"C"}}"#.into()),
        ]);
        let svc = svc1(d, cancel.clone());
        let (events, s) =
            run_and_collect(job(dir.path(), vec!["e1.ass".into()], cancel), &svc, None).await;
        assert_eq!(s.batches_processed, 3);
        // Progress comes from completions: initial 0, then 1..=3 EXACTLY once
        // each (any arrival order — the frontend store clamps these; see the
        // UX-overhaul store changes).
        // Strict multiset equality also proves there is a single emission
        // source — a second one would duplicate counts and fail this.
        let mut dones: Vec<u32> = events
            .iter()
            .filter_map(|e| match e {
                GlossaryEvent::Progress { done, total } => {
                    assert_eq!(*total, 3);
                    Some(*done)
                }
                _ => None,
            })
            .collect();
        dones.sort_unstable();
        assert_eq!(dones, vec![0, 1, 2, 3]);
    }

    #[tokio::test(start_paused = true)]
    async fn reference_errors_land_in_done_summary() {
        // ref/ dir → reference extraction fires BEFORE the glossary batch.
        // Driver cap=1 → requests are serialized in submission order:
        //   call 1: reference batch → unparseable → records an error
        //   call 2: glossary batch  → ok → 1 term
        let dir = tempfile::tempdir().unwrap();
        let ref_dir = dir.path().join("ref");
        std::fs::create_dir(&ref_dir).unwrap();
        write_ass(&ref_dir, "r1.ass", &["Lin Dong strikes"]); // 1 reference batch
        write_ass(dir.path(), "e1.ass", &["一"]); // 1 extraction batch
        let cancel = CancellationToken::new();
        let d = ScriptedDriver::new(vec![
            Ok("not json at all".into()), // reference batch → unparseable → error
            Ok(r#"{"characters":{"林动":"Lin Dong"}}"#.into()), // glossary batch ok
        ]);
        let svc = svc1(d, cancel.clone());
        let (_events, s) =
            run_and_collect(job(dir.path(), vec!["e1.ass".into()], cancel), &svc, None).await;

        assert_eq!(s.errors.len(), 1, "expected 1 error in summary: {:?}", s.errors);
        assert!(
            s.errors[0].starts_with("reference batch"),
            "error must start with 'reference batch': {:?}",
            s.errors[0]
        );
        assert!(
            s.errors[0].contains("unparseable"),
            "error must mention 'unparseable': {:?}",
            s.errors[0]
        );
        assert_eq!(s.terms_final, 1, "the glossary build itself must be unaffected");
        assert!(!s.aborted && !s.cancelled);
    }

    #[tokio::test(start_paused = true)]
    async fn normalize_runs_on_new_terms_and_merge_keeps_existing_wins() {
        let dir = tempfile::tempdir().unwrap();
        let mut existing = Glossary::new("xianxia");
        existing.characters.insert("林动".into(), "EXISTING WINS".into());
        crate::glossary::io::save_folder_glossary(dir.path(), &existing).unwrap();

        write_ass(dir.path(), "e1.ass", &["一"]); // 1 batch
        let cancel = CancellationToken::new();
        let d = ScriptedDriver::new(vec![
            // extraction: two new chars (one colliding with existing)
            Ok(r#"{"characters":{"林动":"ignored","应欢欢":"ying huanhuan"}}"#.into()),
            // normalize characters (the only non-empty category of NEW terms)
            Ok(r#"{"林动":"still ignored","应欢欢":"Ying Huanhuan"}"#.into()),
        ]);
        let svc = svc1(d, cancel.clone());
        let mut j = job(dir.path(), vec!["e1.ass".into()], cancel);
        j.normalize = true;
        let (_events, s) = run_and_collect(j, &svc, None).await;
        assert!(s.normalized);
        let saved = load_folder_glossary(dir.path()).unwrap();
        assert_eq!(saved.characters.get("林动").unwrap(), "EXISTING WINS");
        assert_eq!(saved.characters.get("应欢欢").unwrap(), "Ying Huanhuan");
    }

    #[tokio::test(start_paused = true)]
    async fn custom_extract_template_reaches_the_request() {
        // No ref/ dir and no cache → build goes straight to extraction.
        // batch_limit 2 × 0.7 → 1 line per batch; 1 line file → 1 extraction call.
        let dir = tempfile::tempdir().unwrap();
        write_ass(dir.path(), "e1.ass", &["修仙第一句"]);
        let cancel = CancellationToken::new();
        let mut prompts = crate::prompts::GlossaryPrompts::defaults();
        prompts.extract = "XEXTRACTX {world_type}".into();
        let d = ScriptedDriver::new(vec![Ok(r#"{"terms":{}}"#.into())]);
        let svc = svc1(d.clone(), cancel.clone());
        let mut j = job(dir.path(), vec!["e1.ass".into()], cancel);
        j.prompts = prompts;
        let _ = run_and_collect(j, &svc, None).await;
        let req = d.last_request().expect("extraction must have sent a request");
        assert!(
            req.system.starts_with("XEXTRACTX"),
            "custom extract template must reach the wire: {:?}",
            req.system
        );
        assert!(
            !req.system.contains("{world_type}"),
            "world_type placeholder must be filled in custom template: {:?}",
            req.system
        );
        assert!(
            req.system.contains("xianxia"),
            "world value 'xianxia' must appear in custom template: {:?}",
            req.system
        );
    }

    #[tokio::test(start_paused = true)]
    async fn personalize_failure_keeps_glossary_and_records_error() {
        let dir = tempfile::tempdir().unwrap();
        write_ass(dir.path(), "e1.ass", &["一"]);
        let cancel = CancellationToken::new();
        let d = ScriptedDriver::new(vec![Ok(r#"{"characters":{"林动":"Lin Dong"}}"#.into())]);
        let pd = ScriptedDriver::new(vec![Ok("no json here".into())]);
        let svc = svc1(d, cancel.clone());
        let p_svc = svc1(pd, CancellationToken::new());
        let mut j = job(dir.path(), vec!["e1.ass".into()], cancel);
        j.personalize = true;
        let (_events, s) = run_and_collect(j, &svc, Some(&p_svc)).await;
        assert!(!s.personalized);
        assert_eq!(s.errors.len(), 1);
        assert!(s.errors[0].starts_with("Look up names online"), "{}", s.errors[0]);
        assert_eq!(s.terms_final, 1); // glossary intact
    }

    /// Normalization must run on `norm_svc`, NOT the extraction service —
    /// the two params are adjacent and identically typed, so a transposition
    /// would otherwise be invisible to every same-service test.
    #[tokio::test(start_paused = true)]
    async fn normalize_pass_runs_on_the_norm_service() {
        let dir = tempfile::tempdir().unwrap();
        write_ass(dir.path(), "e1.ass", &["一"]); // 1 line → 1 extraction batch

        let cancel = CancellationToken::new();

        // Driver A: scripted for extraction only — returns one term.
        let extract_driver = ScriptedDriver::new(vec![Ok(
            r#"{"characters":{"林动":"lin dong"}}"#.into(),
        )]);

        // Driver B: scripted for normalization only — returns the corrected casing.
        let norm_driver = ScriptedDriver::new(vec![Ok(r#"{"林动":"Lin Dong"}"#.into())]);

        let extract_svc = svc1(extract_driver.clone(), cancel.clone());
        let norm_svc = svc1(norm_driver.clone(), CancellationToken::new());

        let mut j = job(dir.path(), vec!["e1.ass".into()], cancel);
        j.normalize = true;

        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        build_glossary(j, &extract_svc, &norm_svc, None, tx).await;

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        let summary = events
            .iter()
            .rev()
            .find_map(|e| match e {
                GlossaryEvent::Done { summary } => Some(summary.clone()),
                _ => None,
            })
            .expect("build must emit Done");

        // Extraction driver A received exactly the extraction call(s).
        assert_eq!(extract_driver.call_count(), 1, "extraction driver must have been called once");
        let extract_req = extract_driver.last_request().expect("extraction driver must have a recorded request");
        // The extraction system prompt starts with "You are a terminology extractor"
        // (from glossary.txt), NOT the normalize prompt.
        assert!(
            extract_req.system.contains("terminology extractor"),
            "extraction driver must have received the extraction prompt, got: {:?}",
            &extract_req.system[..extract_req.system.len().min(120)]
        );

        // Normalization driver B received exactly the normalize call(s).
        assert!(norm_driver.call_count() >= 1, "norm driver must have been called at least once");
        let norm_req = norm_driver.last_request().expect("norm driver must have a recorded request");
        // The normalize system prompt starts with "You are normalizing"
        // (from glossary-normalize-characters.txt), NOT the extraction prompt.
        assert!(
            norm_req.system.contains("normalizing"),
            "norm driver must have received the normalize prompt, got: {:?}",
            &norm_req.system[..norm_req.system.len().min(120)]
        );

        // The final glossary reflects the normalized value from driver B.
        assert!(summary.normalized, "build must report normalized=true");
        let saved = load_folder_glossary(dir.path()).unwrap();
        assert_eq!(saved.characters.get("林动").unwrap(), "Lin Dong");
    }

    #[tokio::test(start_paused = true)]
    async fn regenerate_ignores_existing_and_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let mut existing = Glossary::new("xianxia");
        existing.characters.insert("应欢欢".into(), "Ying Huanhuan".into());
        crate::glossary::io::save_folder_glossary(dir.path(), &existing).unwrap();

        write_ass(dir.path(), "e1.ass", &["一"]); // 1 line → 1 batch
        let cancel = CancellationToken::new();
        let d = ScriptedDriver::new(vec![Ok(r#"{"characters":{"林动":"Lin Dong"}}"#.into())]);
        let svc = svc1(d, cancel.clone());
        let mut j = job(dir.path(), vec!["e1.ass".into()], cancel);
        j.mode = BuildMode::Regenerate;
        let (_events, s) = run_and_collect(j, &svc, None).await;

        let saved = load_folder_glossary(dir.path()).unwrap();
        assert_eq!(saved.count(), 1, "regenerate must not merge the old glossary");
        assert_eq!(saved.characters.get("林动").unwrap(), "Lin Dong");
        assert!(!saved.characters.contains_key("应欢欢"), "old term must be gone");
        assert_eq!(s.terms_final, 1);
        // The diff base is what was ON DISK before the run, not the (empty)
        // merge base — a regenerate must report the curated terms it discarded,
        // at the one moment the user can still notice the loss.
        assert_eq!(s.diff.total_added, 1);
        assert_eq!(s.diff.total_removed, 1, "the discarded term must be reported as removed");
    }

    #[tokio::test(start_paused = true)]
    async fn append_mode_keeps_existing_wins() {
        let dir = tempfile::tempdir().unwrap();
        let mut existing = Glossary::new("xianxia");
        existing.characters.insert("林动".into(), "EXISTING WINS".into());
        crate::glossary::io::save_folder_glossary(dir.path(), &existing).unwrap();

        write_ass(dir.path(), "e1.ass", &["一"]);
        let cancel = CancellationToken::new();
        let d = ScriptedDriver::new(vec![Ok(
            r#"{"characters":{"林动":"ignored","应欢欢":"Ying Huanhuan"}}"#.into(),
        )]);
        let svc = svc1(d, cancel.clone());
        let mut j = job(dir.path(), vec!["e1.ass".into()], cancel);
        j.mode = BuildMode::Append;
        let (_events, s) = run_and_collect(j, &svc, None).await;

        let saved = load_folder_glossary(dir.path()).unwrap();
        assert_eq!(saved.characters.get("林动").unwrap(), "EXISTING WINS");
        assert_eq!(saved.characters.get("应欢欢").unwrap(), "Ying Huanhuan");
        assert_eq!(s.terms_final, 2);
        assert_eq!(s.diff.total_added, 1, "only the genuinely new term is added");
    }

    /// A regenerate that persists NOTHING must not report a wipe. Both saves
    /// are guarded by `!is_empty()`, so the original glossary is untouched on
    /// disk — a diff of `result` (empty) against the on-disk base would strike
    /// every curated term through in the post-build "Glossary changes" modal
    /// at the exact moment nothing was lost.
    #[tokio::test(start_paused = true)]
    async fn regenerate_with_no_terms_reports_no_removals() {
        let dir = tempfile::tempdir().unwrap();
        let mut existing = Glossary::new("xianxia");
        existing.characters.insert("应欢欢".into(), "Ying Huanhuan".into());
        existing.locations.insert("青阳镇".into(), "Qingyang Town".into());
        crate::glossary::io::save_folder_glossary(dir.path(), &existing).unwrap();

        write_ass(dir.path(), "e1.ass", &["一", "二"]); // 2 batches
        let cancel = CancellationToken::new();
        let d = ScriptedDriver::new(vec![
            Err(LlmError::Http { status: 400, body: "bad".into(), retry_after: None }),
            Err(LlmError::Http { status: 400, body: "bad".into(), retry_after: None }),
        ]);
        let svc = svc1(d, cancel.clone());
        let mut j = job(dir.path(), vec!["e1.ass".into()], cancel);
        j.mode = BuildMode::Regenerate;
        let (_events, s) = run_and_collect(j, &svc, None).await;

        assert_eq!(s.diff.total_removed, 0, "nothing was written, so nothing was removed");
        assert!(!s.diff.has_changes, "an unchanged glossary must not pop the diff modal");
        // …and the glossary really is intact.
        let saved = load_folder_glossary(dir.path()).unwrap();
        assert_eq!(saved.count(), 2);
        assert_eq!(saved.characters.get("应欢欢").unwrap(), "Ying Huanhuan");
        assert_eq!(saved.locations.get("青阳镇").unwrap(), "Qingyang Town");
    }

    /// The other side of the same coin: a regenerate that DOES persist must
    /// still report what it discarded (the diff base is the pre-run on-disk
    /// glossary, not the empty merge base).
    #[tokio::test(start_paused = true)]
    async fn regenerate_that_persists_reports_removals() {
        let dir = tempfile::tempdir().unwrap();
        let mut existing = Glossary::new("xianxia");
        existing.characters.insert("应欢欢".into(), "Ying Huanhuan".into());
        crate::glossary::io::save_folder_glossary(dir.path(), &existing).unwrap();

        write_ass(dir.path(), "e1.ass", &["一"]); // 1 batch
        let cancel = CancellationToken::new();
        // The rebuild never produces 应欢欢 — the curated term is gone.
        let d = ScriptedDriver::new(vec![Ok(r#"{"characters":{"林动":"Lin Dong"}}"#.into())]);
        let svc = svc1(d, cancel.clone());
        let mut j = job(dir.path(), vec!["e1.ass".into()], cancel);
        j.mode = BuildMode::Regenerate;
        let (_events, s) = run_and_collect(j, &svc, None).await;

        assert!(s.diff.total_removed > 0, "a persisted regenerate must report its losses");
        assert!(s.diff.has_changes);
        let saved = load_folder_glossary(dir.path()).unwrap();
        assert!(!saved.characters.contains_key("应欢欢"));
    }

    /// A regenerate whose every batch fails must be a no-op, not a wipe: no
    /// incremental save fires, so the original glossary is still on disk.
    #[tokio::test(start_paused = true)]
    async fn regenerate_with_all_batches_failing_leaves_old_glossary() {
        let dir = tempfile::tempdir().unwrap();
        let mut existing = Glossary::new("xianxia");
        existing.characters.insert("应欢欢".into(), "Ying Huanhuan".into());
        crate::glossary::io::save_folder_glossary(dir.path(), &existing).unwrap();

        write_ass(dir.path(), "e1.ass", &["一", "二"]); // 2 batches
        let cancel = CancellationToken::new();
        let d = ScriptedDriver::new(vec![
            Err(LlmError::Http { status: 400, body: "bad".into(), retry_after: None }),
            Err(LlmError::Http { status: 400, body: "bad".into(), retry_after: None }),
        ]);
        let svc = svc1(d, cancel.clone());
        let mut j = job(dir.path(), vec!["e1.ass".into()], cancel);
        j.mode = BuildMode::Regenerate;
        let (_events, s) = run_and_collect(j, &svc, None).await;

        assert_eq!(s.batches_processed, 0);
        assert_eq!(s.terms_final, 0);
        let saved = load_folder_glossary(dir.path()).unwrap();
        assert_eq!(
            saved.characters.get("应欢欢").unwrap(),
            "Ying Huanhuan",
            "a failed regenerate must not destroy the glossary"
        );
    }
}
