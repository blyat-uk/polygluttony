//! Per-file translation pipeline — the orchestrating state machine of a run:
//! parse → batch loop (halving + context carryover) → cleanup pass →
//! auto-verify → scoped retranslation (≤ MAX_RETRANSLATION_ATTEMPTS) → write
//! output. Port of `core/translator.py:96-330` (translate_file + verify loop)
//! and `:399-581` (_translate_all_batches).
//!
//! # Deviations from Python (deliberate)
//!
//! - **No temp output file for verification** (`translator.py:177-181`): the
//!   Python verifier re-reads source + temp output from disk; we verify the
//!   in-memory (stripped src, stripped tgt) map directly.
//! - **Attempt budget shape**: Python runs up to MAX retranslations (so up to
//!   MAX + 1 verifies). We loop `for attempt in 1..=MAX` over *verifies* and
//!   stop retranslating when `attempt == MAX` — one verify fewer; the budget
//!   still caps runaway loops, and the final verify's issues are kept either
//!   way.
//! - **Out-of-sample flags** (see [`scopes_for`]): the verifier here can flag
//!   line ids that were never sampled (Python structurally cannot). When the
//!   trust-boundary walk consequently finds no scopes, we recompute treating
//!   every line as a trust boundary, producing tight scopes around exactly the
//!   flagged lines instead of discarding the signal.
//! - **Untranslated lines are dropped from the output** (spec): Python wrote
//!   them back with their source text.
//! - **Drift is log-only at the batch level** (spec §6): a suspicious batch is
//!   reported to the UI log but not retried mid-loop; the verify stage is the
//!   enforcement point.
//! - **Scoped retranslation is chunked** (`retranslate_scope`): Python sends
//!   the whole scope as a single request; we chunk by `initial_batch_size` to
//!   avoid exceeding model context when a scope spans many lines.

use std::collections::BTreeMap;
use std::path::PathBuf;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::ass::decode::decode_file;
use crate::ass::parse::{parse_dialogues, DialogueLine};
use crate::ass::tags::strip_for_text;
use crate::ass::writer::write_translated;
use crate::events::{FileResult, FileStateKind, LogLevel, LogPhase, RunEvent, VerifyIssue};
use crate::glossary::model::Glossary;
use crate::llm::service::LlmService;
use crate::models::language_pair::{output_filename, warning_filename, LanguagePair};
use crate::translation::batch::{translate_batch_tagged, BatchOutcome, BatchSettings};
use crate::translation::batching::{
    halved, initial_batch_size, CONTEXT_CARRYOVER_LINES, MAX_RETRANSLATION_ATTEMPTS,
};
use crate::translation::cleanup::cleanup_pass;
use crate::translation::source_detect::SourceDetector;
use crate::translation::verify::{verify_file, VerifyReport};
use crate::validation::drift;
use crate::validation::scopes::{compute_scopes, is_full_file, Scope};
use crate::validation::LinePair;

/// Context padding for scoped retranslation (`translator.py:243`).
const SCOPE_PADDING: u32 = 5;

/// Cap on per-line "untranslated" issues synthesized for the warning panel;
/// anything beyond folds into one aggregate entry.
const MAX_UNTRANSLATED_ISSUES: usize = 10;

/// Everything needed to translate one file. Borrowed pieces (`svc`,
/// `verify_svc`, `glossary`) are shared across the files of a run.
pub struct FileJob<'a> {
    pub input: PathBuf,
    /// Relative file name — every emitted event uses this, never a path.
    pub file_name: String,
    /// The Translate step: the batches, the cleanup passes and the
    /// retranslations verify asks for.
    pub svc: &'a LlmService,
    /// The Verify step, which may run on another model.
    pub verify_svc: &'a LlmService,
    pub glossary: &'a Glossary,
    pub pair: LanguagePair,
    pub prompts: &'a crate::prompts::TranslationPrompts,
    pub batch_limit: Option<u32>,
    pub cancel: CancellationToken,
    pub tx: mpsc::Sender<RunEvent>,
}

/// Translate one `.ass` file end-to-end. Never panics outward: any internal
/// error is converted into `RunEvent::Error` + `State(Failed)` and a failed
/// [`FileResult`].
pub async fn translate_file(job: FileJob<'_>) -> FileResult {
    let file = job.file_name.clone();
    let tx = job.tx.clone();
    match run(job).await {
        Ok(result) => result,
        Err(message) => {
            let _ = tx.send(RunEvent::Error { file: file.clone(), message }).await;
            let _ = tx
                .send(RunEvent::State {
                    file: file.clone(),
                    state: FileStateKind::Failed,
                    detail: None,
                })
                .await;
            FileResult {
                file,
                success: false,
                total_lines: 0,
                translated_lines: 0,
                has_warnings: false,
                issues: Vec::new(),
                output_path: None,
            }
        }
    }
}

async fn run(job: FileJob<'_>) -> Result<FileResult, String> {
    // 1. Parse.
    let original = decode_file(&job.input).map_err(|e| e.to_string())?;
    let lines = parse_dialogues(&original);
    let settings = BatchSettings {
        pair: job.pair.clone(),
        template: job.prompts.template.clone(),
        tone_text: job.prompts.tone.clone(),
    };
    let mut st = FileState {
        job: &job,
        lines,
        translations: BTreeMap::new(),
        context: Vec::new(),
        retries: 0,
        settings,
    };

    st.state(FileStateKind::Translating, None).await;
    st.log(LogLevel::Info, LogPhase::Parse, format!("{} dialogue lines", st.lines.len())).await;
    if st.lines.is_empty() {
        return Err("no dialogue lines".into());
    }
    let detector = SourceDetector::for_language(&job.pair.source);

    // 2. Initial batch loop + 3. cleanup pass.
    st.translate_all().await?;
    st.run_cleanup(detector.as_ref()).await?;

    // 4. Verify + scoped retranslation loop (`translator.py:176-311`).
    let all_ids: Vec<u32> = (1..=st.lines.len() as u32).collect();
    let mut issues: Vec<VerifyIssue> = Vec::new();
    let mut has_warnings = false;
    for attempt in 1..=MAX_RETRANSLATION_ATTEMPTS {
        if job.cancel.is_cancelled() {
            return Err("translation cancelled".into());
        }
        st.state(FileStateKind::Verifying, None).await;
        let stripped = st.stripped_pairs();
        let report =
            verify_file(job.verify_svc, &stripped, &job.glossary.all_terms(), &job.prompts.verify).await;
        let report = match report {
            Ok(r) => r,
            Err(e) => {
                // Dead key mid-verify: fail the file and doom the run, mirroring
                // translation's Fatal handling (pipeline.rs BatchOutcome::Fatal).
                job.cancel.cancel();
                return Err(format!("verification aborted: {e}"));
            }
        };
        if report.issues.is_empty() {
            break; // clean
        }
        if attempt == MAX_RETRANSLATION_ATTEMPTS {
            issues = report.issues;
            has_warnings = true;
            break;
        }
        let scopes = scopes_for(&report, &all_ids);
        if scopes.is_empty() {
            issues = report.issues;
            has_warnings = true;
            break;
        }
        st.state(
            FileStateKind::Retranslating,
            Some(format!("attempt {attempt}/{MAX_RETRANSLATION_ATTEMPTS}")),
        )
        .await;
        if is_full_file(&scopes, &all_ids) {
            st.log(
                LogLevel::Info,
                LogPhase::Retranslate,
                format!(
                    "verify found {} issues — full retranslation (attempt {attempt}/{MAX_RETRANSLATION_ATTEMPTS})",
                    report.issues.len()
                ),
            )
            .await;
            // Stash the prior (drift-flagged but complete) translations before
            // clearing. If translate_all() comes back with fewer lines (e.g.
            // provider outage kills batches mid-run), we backfill the gaps so
            // the output is never *less* complete than what we already had.
            // The backfilled lines are still flagged by the verifier, so
            // has_warnings will be set — correct behaviour.
            let prior = std::mem::take(&mut st.translations);
            st.translate_all().await?;
            // Backfill: keep any id that translate_all() did not produce.
            for (id, t) in prior {
                st.translations.entry(id).or_insert(t);
            }
        } else {
            let ranges = scopes
                .iter()
                .map(|s| format!("{}-{}", s.start_line, s.end_line))
                .collect::<Vec<_>>()
                .join(", ");
            st.log(
                LogLevel::Info,
                LogPhase::Retranslate,
                format!(
                    "verify found {} issues — retranslating scopes {ranges} (attempt {attempt}/{MAX_RETRANSLATION_ATTEMPTS})",
                    report.issues.len()
                ),
            )
            .await;
            for scope in &scopes {
                st.retranslate_scope(scope).await?;
            }
        }
        // Re-run cleanup after any retranslation (`translator.py:307-311`).
        st.run_cleanup(detector.as_ref()).await?;
    }

    // 5. Final write.
    let untranslated = st.lines.len() - st.translations.len();
    has_warnings |= untranslated > 0 || !issues.is_empty();

    // Warning-only files (untranslated lines, no verify issues) must still be
    // expandable in the UI: synthesize one issue per untranslated line
    // (capped) AFTER the real verify issues, plus an aggregate overflow entry.
    // FileDone and FileResult share this vec, so both stay identical.
    if untranslated > 0 {
        let missing: Vec<u32> = (1..=st.lines.len() as u32)
            .filter(|id| !st.translations.contains_key(id))
            .collect();
        for id in missing.iter().take(MAX_UNTRANSLATED_ISSUES) {
            issues.push(VerifyIssue {
                line_id: *id,
                source: strip_for_text(st.raw_text(*id)),
                translation: String::new(),
                issue_type: "untranslated".into(),
                description: "line was not translated".into(),
                severity: "warning".into(),
            });
        }
        if missing.len() > MAX_UNTRANSLATED_ISSUES {
            issues.push(VerifyIssue {
                line_id: 0,
                source: String::new(),
                translation: String::new(),
                issue_type: "untranslated".into(),
                description: format!(
                    "…and {} more untranslated lines",
                    missing.len() - MAX_UNTRANSLATED_ISSUES
                ),
                severity: "warning".into(),
            });
        }
    }

    let name = if has_warnings {
        warning_filename(&job.input, &job.pair)
    } else {
        output_filename(&job.input, &job.pair)
    };
    let path = match job.input.parent() {
        Some(parent) => parent.join(name),
        None => PathBuf::from(name),
    };
    // Untranslated lines are dropped, not written back as source text (spec).
    let out_lines: Vec<DialogueLine> = st
        .lines
        .iter()
        .enumerate()
        .filter_map(|(idx, d)| {
            st.translations.get(&(idx as u32 + 1)).map(|t| {
                let mut d = d.clone();
                d.text = t.clone();
                d
            })
        })
        .collect();
    write_translated(&path, &original, &out_lines).map_err(|e| e.to_string())?;

    st.emit(RunEvent::FileDone {
        file: job.file_name.clone(),
        has_warnings,
        issues: issues.clone(),
    })
    .await;
    st.state(
        if has_warnings { FileStateKind::Warning } else { FileStateKind::Done },
        None,
    )
    .await;

    Ok(FileResult {
        file: job.file_name.clone(),
        success: true,
        total_lines: st.lines.len() as u32,
        translated_lines: st.translations.len() as u32,
        has_warnings,
        issues,
        output_path: Some(path.display().to_string()),
    })
}

/// Compute retranslation scopes from a verify report (`translator.py:242-251`).
///
/// Python guarantees `failed ⊆ sampled` (issues can only come from samples), so
/// the trust-boundary walk in `compute_scopes` always finds its failed ids
/// among the sampled ones. Our verifier accepts any line id the LLM names, and
/// its stage-1 drift short-circuit reports flagged ids with *no* samples at
/// all. In both cases the first computation comes back empty even though we
/// know exactly which lines are bad — so recompute treating every line as a
/// trust boundary, yielding tight scopes around exactly the flagged lines.
/// `is_full_file` still escalates to a full redo when the flags blanket the
/// file (and the all-samples-failed case already returns a full-file scope
/// from the first call).
fn scopes_for(report: &VerifyReport, all_ids: &[u32]) -> Vec<Scope> {
    let scopes =
        compute_scopes(&report.sampled_line_ids, &report.failed_line_ids, all_ids, SCOPE_PADDING);
    if !scopes.is_empty() || report.failed_line_ids.is_empty() {
        return scopes;
    }
    compute_scopes(all_ids, &report.failed_line_ids, all_ids, SCOPE_PADDING)
}

/// Mutable per-file state threaded through the pipeline stages. Line ids are
/// 1-based indexes into `lines` (id = index + 1), matching the Python parser.
struct FileState<'j, 'a> {
    job: &'j FileJob<'a>,
    lines: Vec<DialogueLine>,
    /// id → tagged translation (ASS override tags reapplied).
    translations: BTreeMap<u32, String>,
    /// Carryover context for the batch loop: (stripped src, stripped tgt),
    /// capped at [`CONTEXT_CARRYOVER_LINES`]. Reset on every loop (re)entry.
    context: Vec<(String, String)>,
    retries: u32,
    settings: BatchSettings,
}

impl FileState<'_, '_> {
    fn raw_text(&self, id: u32) -> &str {
        &self.lines[(id - 1) as usize].text
    }

    async fn emit(&self, event: RunEvent) {
        let _ = self.job.tx.send(event).await;
    }

    async fn state(&self, state: FileStateKind, detail: Option<String>) {
        self.emit(RunEvent::State { file: self.job.file_name.clone(), state, detail }).await;
    }

    async fn log(&self, level: LogLevel, phase: LogPhase, message: String) {
        self.emit(RunEvent::Log {
            file: Some(self.job.file_name.clone()),
            level,
            phase,
            message,
        })
        .await;
    }

    async fn progress(&self, batch: u32, total_batches: u32) {
        self.emit(RunEvent::Progress {
            file: self.job.file_name.clone(),
            translated: self.translations.len() as u32,
            total: self.lines.len() as u32,
            batch,
            total_batches,
            retries: self.retries,
        })
        .await;
    }

    /// id → (stripped src, stripped tgt) for every translated line — the
    /// verifier's input shape.
    fn stripped_pairs(&self) -> BTreeMap<u32, (String, String)> {
        self.translations
            .iter()
            .map(|(id, tgt)| (*id, (strip_for_text(self.raw_text(*id)), strip_for_text(tgt))))
            .collect()
    }

    /// The batch loop (`translator.py:399-581`): translate every line not yet
    /// in `translations`, halving the batch size on failure, carrying the last
    /// ≤7 translated pairs forward as context. Also serves the full-redo path
    /// (caller clears `translations` first).
    async fn translate_all(&mut self) -> Result<(), String> {
        let mut pending: Vec<u32> = (1..=self.lines.len() as u32)
            .filter(|id| !self.translations.contains_key(id))
            .collect();
        let mut batch_size = initial_batch_size(self.job.batch_limit);
        self.context.clear();
        let mut batch_num: u32 = 0;
        let mut total_batches = pending.len().div_ceil(batch_size as usize) as u32;

        while !pending.is_empty() {
            if self.job.cancel.is_cancelled() {
                return Err("translation cancelled".into());
            }
            batch_num += 1;
            let n = (batch_size as usize).min(pending.len());
            let take: Vec<u32> = pending.drain(..n).collect();
            let raw: Vec<(u32, String)> =
                take.iter().map(|id| (*id, self.raw_text(*id).to_string())).collect();

            let outcome = translate_batch_tagged(
                self.job.svc,
                &raw,
                self.job.glossary,
                &self.context,
                &self.settings,
            )
            .await;
            match outcome {
                BatchOutcome::Success(map) => {
                    // Layer-2 drift over the finished batch — log-only (spec §6);
                    // the verify stage is the enforcement point.
                    let pairs: Vec<LinePair> = map
                        .iter()
                        .map(|(id, tgt)| LinePair {
                            id: *id,
                            src: strip_for_text(self.raw_text(*id)),
                            tgt: strip_for_text(tgt),
                        })
                        .collect();
                    let d = drift::detect(&pairs, &self.job.glossary.all_terms());
                    let merged: Vec<u32> = map.keys().copied().collect();
                    self.translations.extend(map);
                    // Carry the last ≤7 freshly translated pairs forward.
                    let from = merged.len().saturating_sub(CONTEXT_CARRYOVER_LINES);
                    let context: Vec<(String, String)> = merged[from..]
                        .iter()
                        .map(|id| {
                            (
                                strip_for_text(self.raw_text(*id)),
                                strip_for_text(&self.translations[id]),
                            )
                        })
                        .collect();
                    self.context = context;
                    self.progress(batch_num, total_batches).await;
                    if d.has_suspected_drift {
                        self.log(
                            LogLevel::Debug,
                            LogPhase::Batch,
                            format!(
                                "drift score {:.2} (threshold {}) suspected from line {:?}",
                                d.score,
                                drift::THRESHOLD,
                                d.suspected_drift_start_id
                            ),
                        )
                        .await;
                    }
                }
                BatchOutcome::Partial { translated, failed_from } => {
                    let merged_count = translated.len();
                    self.translations.extend(translated);
                    // Re-queue the unfinished tail at the front
                    // (`translator.py:518-521`); unmerged ids below
                    // `failed_from` are dropped, exactly as in Python.
                    let mut requeue: Vec<u32> = take
                        .iter()
                        .copied()
                        .filter(|id| *id >= failed_from && !self.translations.contains_key(id))
                        .collect();
                    requeue.append(&mut pending);
                    pending = requeue;
                    self.retries += 1;
                    self.state(
                        FileStateKind::Retranslating,
                        Some(format!("partial batch {batch_num}")),
                    )
                    .await;
                    // Recompute total_batches after every Partial requeue so
                    // Progress never reports batch > total_batches.
                    total_batches =
                        batch_num + (pending.len() as u32).div_ceil(batch_size);
                    self.progress(batch_num, total_batches).await;
                    // Halve on <10% success (`translator.py:524-527`). At the
                    // floor keep the size: a Partial always merges at least one
                    // line, so the loop still advances.
                    if merged_count * 10 < take.len() {
                        if let Some(s) = halved(batch_size) {
                            batch_size = s;
                            // Recompute again after halving.
                            total_batches =
                                batch_num + (pending.len() as u32).div_ceil(batch_size);
                        }
                    }
                }
                BatchOutcome::Failure(message) => {
                    self.retries += 1;
                    self.log(LogLevel::Warning, LogPhase::Batch, message).await;
                    match halved(batch_size) {
                        Some(s) => {
                            batch_size = s;
                            let mut requeue = take;
                            requeue.append(&mut pending);
                            pending = requeue;
                            total_batches =
                                batch_num + pending.len().div_ceil(batch_size as usize) as u32;
                        }
                        None => {
                            // Floor reached: give up on these lines — they stay
                            // untranslated (`translator.py:570-579`).
                            self.log(
                                LogLevel::Error,
                                LogPhase::Batch,
                                format!("giving up on {} lines", take.len()),
                            )
                            .await;
                        }
                    }
                }
                BatchOutcome::Fatal(message) => {
                    // Auth death: the whole run is doomed — stop everything.
                    self.job.cancel.cancel();
                    return Err(message);
                }
            }
        }
        Ok(())
    }

    /// Cleanup pass for residual source text (`translator.py:583-684`). No-op
    /// when the source language has no character pattern (detector is None).
    async fn run_cleanup(&mut self, detector: Option<&SourceDetector>) -> Result<(), String> {
        let Some(det) = detector else { return Ok(()) };
        self.state(FileStateKind::Cleanup, None).await;
        let sources: BTreeMap<u32, String> = self
            .lines
            .iter()
            .enumerate()
            .map(|(idx, d)| (idx as u32 + 1, d.text.clone()))
            .collect();
        let report = cleanup_pass(
            self.job.svc,
            det,
            &sources,
            &mut self.translations,
            self.job.glossary,
            &self.settings,
        )
        .await;
        if let Some(msg) = report.fatal {
            self.job.cancel.cancel();
            return Err(format!("cleanup aborted: {msg}"));
        }
        if report.skipped_too_many {
            self.log(
                LogLevel::Warning,
                LogPhase::Cleanup,
                format!("too many lines ({}), skipping", report.failed.len()),
            )
            .await;
        } else {
            self.log(
                LogLevel::Info,
                LogPhase::Cleanup,
                format!("{} cleaned, {} still dirty", report.cleaned.len(), report.failed.len()),
            )
            .await;
        }
        Ok(())
    }

    /// Retranslate one scope with up-to-7 preceding translated pairs as
    /// context (`translator.py:687-778`, simplified: only the scope lines are
    /// re-sent; the padding region supplies context instead of being
    /// retranslated and filtered back out).
    ///
    /// Deviation from Python: scope lines are chunked by `initial_batch_size`
    /// (sequential, merging each outcome) so large scopes don't blow the
    /// model's context window.
    async fn retranslate_scope(&mut self, scope: &Scope) -> Result<(), String> {
        let mut context: Vec<(String, String)> = self
            .translations
            .range(..scope.context_start)
            .map(|(id, tgt)| (strip_for_text(self.raw_text(*id)), strip_for_text(tgt)))
            .collect();
        if context.len() > CONTEXT_CARRYOVER_LINES {
            context.drain(..context.len() - CONTEXT_CARRYOVER_LINES);
        }
        let last_id = self.lines.len() as u32;
        let raw: Vec<(u32, String)> = (scope.start_line..=scope.end_line)
            .filter(|id| *id >= 1 && *id <= last_id)
            .map(|id| (id, self.raw_text(id).to_string()))
            .collect();
        if raw.is_empty() {
            return Ok(());
        }
        let chunk_size = initial_batch_size(self.job.batch_limit) as usize;
        for chunk in raw.chunks(chunk_size) {
            match translate_batch_tagged(
                self.job.svc,
                chunk,
                self.job.glossary,
                &context,
                &self.settings,
            )
            .await
            {
                BatchOutcome::Success(map) => {
                    self.translations.extend(map);
                }
                BatchOutcome::Partial { translated, .. } => {
                    // Keep what we got; the next verify pass decides if it stuck.
                    self.translations.extend(translated);
                }
                BatchOutcome::Failure(message) => {
                    self.log(LogLevel::Warning, LogPhase::Retranslate, message).await;
                    // Log and continue to the next chunk; don't abort the scope.
                }
                BatchOutcome::Fatal(message) => {
                    self.job.cancel.cancel();
                    return Err(message);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{FileStateKind, RunEvent};
    use crate::glossary::model::Glossary;
    use crate::llm::service::LlmService;
    use crate::llm::test_support::ScriptedDriver;
    use crate::models::language_pair::LanguagePair;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    fn ass_source(lines: &[&str]) -> String {
        let mut s = String::from(
            "[Script Info]\nTitle: t\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
        );
        for (i, text) in lines.iter().enumerate() {
            s.push_str(&format!(
                "Dialogue: 0,0:00:{:02}.00,0:00:{:02}.50,Default,,0,0,0,,{}\n",
                i + 1,
                i + 1,
                text
            ));
        }
        s
    }

    fn ok_batch(ids: &[(u32, &str)]) -> String {
        let items: Vec<String> = ids
            .iter()
            .map(|(id, t)| format!(r#"{{"id":{id},"tgt":"<{id:04}:D> {t}"}}"#))
            .collect();
        format!("[{}]", items.join(","))
    }

    async fn run_pipeline(
        source: &str,
        responses: Vec<Result<String, crate::llm::error::LlmError>>,
    ) -> (FileResult, Vec<RunEvent>, std::sync::Arc<ScriptedDriver>, tempfile::TempDir) {
        let driver = ScriptedDriver::new(responses);
        let (tx, mut rx) = mpsc::channel(256);
        let svc = LlmService::new(driver.clone(), 2, CancellationToken::new(), tx.clone());
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("ep01.ass");
        std::fs::write(&input, source).unwrap();
        let prompts = crate::prompts::TranslationPrompts::defaults(
            &LanguagePair::from_codes("zh", "en").unwrap(),
            crate::config::projects::Tone::Standard,
        );

        // One scripted driver behind both steps, so a script reads in call order.
        let result = translate_file(FileJob {
            input: input.clone(),
            file_name: "ep01.ass".into(),
            svc: &svc,
            verify_svc: &svc,
            glossary: &Glossary::default(),
            pair: LanguagePair::from_codes("zh", "en").unwrap(),
            prompts: &prompts,
            batch_limit: Some(100),
            cancel: CancellationToken::new(),
            tx: tx.clone(),
        })
        .await;

        drop(tx);
        drop(svc); // the service holds a tx clone — drop it so recv() can end
        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        // Return `dir` so callers keep it alive for as long as needed (e.g.
        // reading the output file). Bind to `_dir` when not needed.
        (result, events, driver, dir)
    }

    #[tokio::test]
    async fn happy_path_writes_clean_output() {
        let src = ass_source(&["你好", "再见"]);
        let (result, events, _, _dir) = run_pipeline(
            &src,
            vec![
                Ok(ok_batch(&[(1, "Hello there friend"), (2, "Goodbye for now")])),
                Ok(r#"{"issues":[]}"#.into()),
            ],
        )
        .await;
        assert!(result.success);
        assert!(!result.has_warnings);
        assert_eq!(result.translated_lines, 2);
        let out = std::path::PathBuf::from(result.output_path.unwrap());
        assert_eq!(out.file_name().unwrap().to_str().unwrap(), "ep01.eng.ass");
        let written = std::fs::read_to_string(&out).unwrap();
        assert!(written.contains("Hello there friend"));
        assert!(written.contains("; Translated at home with Polygluttony"));
        assert!(events.iter().any(
            |e| matches!(e, RunEvent::State { state: FileStateKind::Verifying, .. })
        ));
        assert!(events.iter().any(|e| matches!(e, RunEvent::FileDone { has_warnings: false, .. })));
    }

    #[tokio::test]
    async fn partial_batch_retries_remainder() {
        let src = ass_source(&["你好", "再见", "走吧"]);
        let (result, _, driver, _dir) = run_pipeline(
            &src,
            vec![
                Ok(ok_batch(&[(1, "Hello my good friend")])), // ids 2,3 missing → partial
                Ok(ok_batch(&[(2, "Goodbye then friend"), (3, "Off we go now")])),
                Ok(r#"{"issues":[]}"#.into()),
            ],
        )
        .await;
        assert!(result.success);
        assert_eq!(result.translated_lines, 3);
        assert_eq!(driver.call_count(), 3);
    }

    #[tokio::test]
    async fn cleanup_pass_fixes_residual_source() {
        let src = ass_source(&["你好"]);
        let (result, events, _, _dir) = run_pipeline(
            &src,
            vec![
                // 4 CJK chars of ~18 → ratio ≈ 0.22 > 0.1 ⇒ needs cleanup.
                Ok(ok_batch(&[(1, "你好朋友 still Chinese")])),
                Ok(ok_batch(&[(1, "Fully English now friend")])), // cleanup batch
                Ok(r#"{"issues":[]}"#.into()),
            ],
        )
        .await;
        assert!(result.success && !result.has_warnings);
        assert!(events
            .iter()
            .any(|e| matches!(e, RunEvent::State { state: FileStateKind::Cleanup, .. })));
    }

    #[tokio::test]
    async fn verify_issues_trigger_scoped_retranslation() {
        let src = ass_source(&["你好", "再见", "走吧", "好的", "不行", "可以", "什么", "哪里",
                               "怎么", "为何", "真的", "假的"]);
        let all_ok: Vec<(u32, &str)> = (1..=12).map(|i| (i, "A clean English line here")).collect();
        let (result, events, _, _dir) = run_pipeline(
            &src,
            vec![
                Ok(ok_batch(&all_ok)),
                Ok(r#"{"issues":[{"id":6,"reason":"unrelated"}]}"#.into()), // verify #1 flags id 6
                Ok(ok_batch(&[(6, "Retranslated line six properly")])),    // scoped redo
                Ok(r#"{"issues":[]}"#.into()),                              // verify #2 clean
            ],
        )
        .await;
        assert!(result.success && !result.has_warnings);
        assert!(events.iter().any(
            |e| matches!(e, RunEvent::State { state: FileStateKind::Retranslating, .. })
        ));
    }

    #[tokio::test]
    async fn verify_runs_on_its_own_service_and_retranslation_on_translates() {
        // Translate and Verify on different models: each driver gets only its
        // step's requests (a request to the wrong one would exhaust its script).
        let src = ass_source(&["你好", "再见", "走吧", "好的", "不行", "可以", "什么", "哪里",
                               "怎么", "为何", "真的", "假的"]);
        let all_ok: Vec<(u32, &str)> = (1..=12).map(|i| (i, "A clean English line here")).collect();
        let translator = ScriptedDriver::new(vec![
            Ok(ok_batch(&all_ok)),
            Ok(ok_batch(&[(6, "Retranslated line six properly")])), // the redo verify asked for
        ]);
        let verifier = ScriptedDriver::new(vec![
            Ok(r#"{"issues":[{"id":6,"reason":"unrelated"}]}"#.into()),
            Ok(r#"{"issues":[]}"#.into()),
        ]);
        let (tx, _rx) = mpsc::channel(256);
        let cancel = CancellationToken::new();
        let svc = LlmService::new(translator.clone(), 2, cancel.clone(), tx.clone());
        let verify_svc = LlmService::new(verifier.clone(), 2, cancel.clone(), tx.clone());
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("ep01.ass");
        std::fs::write(&input, &src).unwrap();
        let pair = LanguagePair::from_codes("zh", "en").unwrap();
        let prompts =
            crate::prompts::TranslationPrompts::defaults(&pair, crate::config::projects::Tone::Standard);
        let result = translate_file(FileJob {
            input,
            file_name: "ep01.ass".into(),
            svc: &svc,
            verify_svc: &verify_svc,
            glossary: &Glossary::default(),
            pair,
            prompts: &prompts,
            batch_limit: Some(100),
            cancel,
            tx,
        })
        .await;
        assert!(result.success && !result.has_warnings, "{result:?}");
        assert_eq!((translator.call_count(), verifier.call_count()), (2, 2));
        // The redo of line 6 went to Translate; the checks to Verify.
        assert!(translator.last_request().unwrap().user.contains("<0006:D>"));
        assert_eq!(verifier.last_request().unwrap().system, prompts.verify.to_string());
    }

    #[tokio::test]
    async fn full_coverage_flags_trigger_full_retranslation() {
        let src = ass_source(&["你好", "再见", "走吧", "好的", "不行", "可以", "什么", "哪里",
                               "怎么", "为何", "真的", "假的"]);
        let all_ok: Vec<(u32, &str)> = (1..=12).map(|i| (i, "A clean English line here")).collect();
        let all_flagged = format!(
            r#"{{"issues":[{}]}}"#,
            (1..=12).map(|i| format!(r#"{{"id":{i},"reason":"x"}}"#)).collect::<Vec<_>>().join(",")
        );
        let (result, _, driver, _dir) = run_pipeline(
            &src,
            vec![
                Ok(ok_batch(&all_ok)),
                Ok(all_flagged),
                Ok(ok_batch(&all_ok)),          // full redo
                Ok(r#"{"issues":[]}"#.into()),  // verify #2 clean
            ],
        )
        .await;
        assert!(result.success && !result.has_warnings);
        assert_eq!(driver.call_count(), 4);
    }

    #[tokio::test]
    async fn exhausted_retranslation_writes_warning_file() {
        let src = ass_source(&["你好", "再见", "走吧", "好的", "不行", "可以", "什么", "哪里",
                               "怎么", "为何", "真的", "假的"]);
        let all_ok: Vec<(u32, &str)> = (1..=12).map(|i| (i, "A clean English line here")).collect();
        let flagged = r#"{"issues":[{"id":6,"reason":"unrelated"}]}"#;
        let redo = ok_batch(&[(6, "Still flagged line six")]);
        let (result, _, _, _dir) = run_pipeline(
            &src,
            vec![
                Ok(ok_batch(&all_ok)),
                Ok(flagged.into()),
                Ok(redo.clone()),
                Ok(flagged.into()),
                Ok(redo.clone()),
                Ok(flagged.into()),
                Ok(redo.clone()),
                Ok(flagged.into()),
            ],
        )
        .await;
        assert!(result.success);
        assert!(result.has_warnings);
        assert!(!result.issues.is_empty());
        assert!(result.output_path.unwrap().ends_with("ep01.warning.eng.ass"));
    }

    #[tokio::test]
    async fn auth_error_is_fatal_no_halving() {
        let src = ass_source(&["你好", "再见"]);
        let (result, _, driver, _dir) = run_pipeline(
            &src,
            vec![Err(crate::llm::error::LlmError::Http { status: 401, body: "no".into(), retry_after: None })],
        )
        .await;
        assert!(!result.success);
        assert_eq!(driver.call_count(), 1);
    }

    #[tokio::test]
    async fn batch_failure_halves_then_gives_up_marks_failed_lines() {
        let src = ass_source(&["你好", "再见"]);
        let hopeless = || Ok::<String, crate::llm::error::LlmError>("not json at all".into());
        let mut responses = Vec::new();
        for _ in 0..30 {
            responses.push(hopeless());
        }
        let (result, _, _, _dir) = run_pipeline(&src, responses).await;
        assert!(result.has_warnings || !result.success);
        assert_eq!(result.translated_lines, 0);
    }

    // Fix 1 (cleanup): verify the partial-drop fix works end-to-end at the
    // pipeline level.  Two lines; initial pass returns only id 1 clean → id 2
    // stays dirty through cleanup iterations and ends up in the output as
    // untranslated (has_warnings = true).  See cleanup.rs for the unit-level
    // test of the exact retain logic.
    #[tokio::test]
    async fn redo_collapse_keeps_prior_translations() {
        // 12 lines; all flagged on every verify → full redo path is exercised.
        // Call sequence (batch_limit=100, 12 lines):
        //   call 1  : initial translate_all batch(12) → ok
        //   call 2  : verify attempt=1 → all_flagged → full redo
        //   calls 3-8 : redo translate_all: 6 hopeless ("not json") through the
        //               halving ladder (100→50→25→12→10→give-up×2)
        //               → 0 lines translated; backfill restores all 12 from prior
        //   call 9  : verify attempt=2 → all_flagged → full redo
        //   calls 10-15: same 6 hopeless calls, backfill again
        //   call 16 : verify attempt=3 → all_flagged → attempt==MAX → break
        //
        // Final state: translations = 12 (from prior backfill), has_warnings=true.
        // Oversupply hopeless+flagged to avoid panicking if our count is off.
        let src = ass_source(&["你好", "再见", "走吧", "好的", "不行", "可以",
                               "什么", "哪里", "怎么", "为何", "真的", "假的"]);
        let all_ok: Vec<(u32, &str)> =
            (1..=12).map(|i| (i, "A clean English line here")).collect();
        let all_flagged = format!(
            r#"{{"issues":[{}]}}"#,
            (1..=12)
                .map(|i| format!(r#"{{"id":{i},"reason":"drift"}}"#))
                .collect::<Vec<_>>()
                .join(","),
        );
        let hopeless = || Ok::<String, crate::llm::error::LlmError>("not json".into());
        let flagged = || Ok::<String, crate::llm::error::LlmError>(all_flagged.clone());
        // 1 ok + 1 flagged + 6 hopeless + 1 flagged + 6 hopeless + 1 flagged = 16
        // plus 20 extras (harmless; ScriptedDriver just returns Failure on empty).
        let mut responses: Vec<_> = vec![Ok(ok_batch(&all_ok)), flagged()];
        for _ in 0..6 { responses.push(hopeless()); }
        responses.push(flagged());
        for _ in 0..6 { responses.push(hopeless()); }
        responses.push(flagged());
        // 20 extra entries
        for _ in 0..10 { responses.push(hopeless()); }
        for _ in 0..10 { responses.push(flagged()); }

        let (result, _, _, _dir) = run_pipeline(&src, responses).await;

        // Redo gave 0 new translations but backfill restored the prior 12.
        assert!(result.success, "file write should still succeed");
        assert!(result.has_warnings, "verify issues must propagate to has_warnings");
        assert_eq!(
            result.translated_lines, 12,
            "prior translations must survive a total-outage redo"
        );
    }

    #[tokio::test]
    async fn dead_key_during_verify_fails_file_not_silent_clean() {
        let src = ass_source(&["你好", "再见"]);
        let (result, events, _, _dir) = run_pipeline(
            &src,
            vec![
                Ok(ok_batch(&[(1, "Hello there friend"), (2, "Goodbye for now")])),
                Err(crate::llm::error::LlmError::Http {
                    status: 401,
                    body: "dead key".into(),
                    retry_after: None,
                }),
            ],
        )
        .await;
        assert!(!result.success);
        assert!(result.output_path.is_none());
        assert!(events.iter().any(|e| matches!(
            e,
            RunEvent::Error { message, .. } if message.contains("verification aborted")
        )));
    }

    #[tokio::test]
    async fn dead_key_during_cleanup_fails_file() {
        let src = ass_source(&["你好"]);
        // The translated text still contains source-language characters, so the
        // cleanup pass fires; its single request dies with 401.
        let (result, events, _, _dir) = run_pipeline(
            &src,
            vec![
                Ok(ok_batch(&[(1, "还是中文 leftover")])),
                Err(crate::llm::error::LlmError::Http {
                    status: 401,
                    body: "dead".into(),
                    retry_after: None,
                }),
            ],
        )
        .await;
        assert!(!result.success);
        assert!(events.iter().any(|e| matches!(
            e,
            RunEvent::Error { message, .. } if message.contains("cleanup aborted")
        )));
    }

    #[tokio::test]
    async fn untranslated_lines_without_verify_issues_synthesize_issues() {
        let src = ass_source(&["你好", "{\\an8}再见"]);
        // Batch 1 returns only id 1 → Partial; id 2 requeues and dies down the
        // halving ladder on "not json". The verify call also gets "not json",
        // which degrades to a clean report — leaving id 2 untranslated with NO
        // verify issues: the warning-only shape this test pins down.
        let mut responses: Vec<Result<String, crate::llm::error::LlmError>> =
            vec![Ok(ok_batch(&[(1, "Hello there friend")]))];
        for _ in 0..15 {
            responses.push(Ok("not json".into()));
        }
        let (result, events, _, _dir) = run_pipeline(&src, responses).await;

        assert!(result.success);
        assert!(result.has_warnings);
        assert_eq!(result.translated_lines, 1);
        let synth: Vec<_> =
            result.issues.iter().filter(|i| i.issue_type == "untranslated").collect();
        assert_eq!(synth.len(), 1);
        assert_eq!(synth[0].line_id, 2);
        assert_eq!(synth[0].source, "再见");
        assert_eq!(synth[0].translation, "");
        assert_eq!(synth[0].severity, "warning");
        // FileDone carries the identical vec.
        let done = events
            .iter()
            .find_map(|e| match e {
                RunEvent::FileDone { issues, has_warnings: true, .. } => Some(issues.clone()),
                _ => None,
            })
            .expect("FileDone with warnings");
        assert_eq!(done.len(), result.issues.len());
    }

    #[tokio::test]
    async fn untranslated_issue_cap_adds_aggregate_overflow_entry() {
        // 14 lines; only id 1 translates → 13 untranslated → 10 per-line
        // issues (ids 2..=11) + one aggregate "…and 3 more".
        let lines: Vec<&str> = std::iter::repeat_n("你好", 14).collect();
        let src = ass_source(&lines);
        let mut responses: Vec<Result<String, crate::llm::error::LlmError>> =
            vec![Ok(ok_batch(&[(1, "Hello there friend")]))];
        for _ in 0..15 {
            responses.push(Ok("not json".into()));
        }
        let (result, _, _, _dir) = run_pipeline(&src, responses).await;

        assert!(result.success && result.has_warnings);
        assert_eq!(result.translated_lines, 1);
        assert_eq!(result.issues.len(), 11);
        for (i, issue) in result.issues[..10].iter().enumerate() {
            assert_eq!(issue.issue_type, "untranslated");
            assert_eq!(issue.line_id, i as u32 + 2); // ids 2..=11
        }
        let agg = &result.issues[10];
        assert_eq!(agg.line_id, 0);
        assert_eq!(agg.source, "");
        assert!(agg.description.contains("3 more untranslated"), "{}", agg.description);
    }

    #[tokio::test]
    async fn cancellation_stops_between_batches() {
        let src = ass_source(&["你好", "再见"]);
        let driver = ScriptedDriver::new(vec![Ok(ok_batch(&[(1, "Hello")]))]);
        let (tx, _rx) = mpsc::channel(256);
        let cancel = CancellationToken::new();
        let svc = LlmService::new(driver, 2, cancel.clone(), tx.clone());
        cancel.cancel();
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("ep01.ass");
        std::fs::write(&input, &src).unwrap();
        let prompts = crate::prompts::TranslationPrompts::defaults(
            &LanguagePair::from_codes("zh", "en").unwrap(),
            crate::config::projects::Tone::Standard,
        );
        let result = translate_file(FileJob {
            input,
            file_name: "ep01.ass".into(),
            svc: &svc,
            verify_svc: &svc,
            glossary: &Glossary::default(),
            pair: LanguagePair::from_codes("zh", "en").unwrap(),
            prompts: &prompts,
            batch_limit: Some(100),
            cancel,
            tx,
        })
        .await;
        assert!(!result.success);
        assert!(result.output_path.is_none());
    }

    #[tokio::test]
    async fn warning_file_done_carries_issues() {
        let src = ass_source(&["很长的中文第一句？", "很长的中文第二句？"]);
        // Empty translations make stage-1 drift fire on every verify attempt with
        // zero verify LLM calls (same trick as verify.rs::fast_drift_stage_short_circuits),
        // so the script is: initial batch + one full retranslation per non-final
        // attempt, all returning empty text.
        let empty = ok_batch(&[(1, ""), (2, "")]);
        let mut responses = vec![Ok(empty.clone())];
        for _ in 1..MAX_RETRANSLATION_ATTEMPTS {
            responses.push(Ok(empty.clone()));
        }
        let (result, events, _, _dir) = run_pipeline(&src, responses).await;
        assert!(result.success);
        assert!(result.has_warnings);
        assert!(!result.issues.is_empty());
        let done_issues = events
            .iter()
            .find_map(|e| match e {
                RunEvent::FileDone { issues, has_warnings: true, .. } => Some(issues.clone()),
                _ => None,
            })
            .expect("FileDone with warnings");
        assert_eq!(done_issues.len(), result.issues.len());
        assert_eq!(done_issues[0].issue_type, "drift");
    }
}
