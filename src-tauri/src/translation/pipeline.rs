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
//!
//! # How a file ends
//!
//! Every file ends one honest way ([`FileEnd`]): done, needs a look, failed
//! on its own (unreadable, no dialogue, nothing usable came back), or
//! stopped because the run stopped (the user's Stop, or an error no retry
//! fixes, which stops the whole run through [`RunStop`]). A stopped file keeps
//! its translations so far; Resume seeds them back ([`FileJob::seed`]) and
//! sends only the missing lines.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::ass::decode::decode_file;
use crate::ass::parse::{parse_dialogues, DialogueLine};
use crate::ass::tags::strip_for_text;
use crate::ass::writer::write_translated;
use crate::config::Step;
use crate::events::{
    EpisodeActivity, ErrorKind, FileResult, FileStateKind, LogLevel, LogPhase, RunEvent,
    VerifyIssue,
};
use crate::glossary::model::Glossary;
use crate::llm::service::{LlmService, REQUEST_FILE};
use crate::models::language_pair::{output_filename, warning_filename, LanguagePair};
use crate::translation::batch::{translate_batch_tagged, BatchOutcome, BatchSettings};
use crate::translation::batching::{
    halved, initial_batch_size, CONTEXT_CARRYOVER_LINES, MAX_CLEANUP_LINES,
    MAX_RETRANSLATION_ATTEMPTS,
};
use crate::translation::cleanup::cleanup_pass;
use crate::translation::live::episode_sources;
use crate::translation::results::fingerprint;
use crate::translation::source_detect::SourceDetector;
use crate::translation::stop::{RunStop, StopCause};
use crate::translation::verify::{verify_file, VerifyReport};
use crate::validation::drift;
use crate::validation::scopes::{compute_scopes, is_full_file, Scope};
use crate::validation::LinePair;

/// Context padding for scoped retranslation (`translator.py:243`).
const SCOPE_PADDING: u32 = 5;

/// Cap on per-line "untranslated" issues synthesized for the warning panel;
/// anything beyond folds into one aggregate entry.
const MAX_UNTRANSLATED_ISSUES: usize = 10;

/// What an untranslated line's issue says.
pub const UNTRANSLATED_DESCRIPTION: &str =
    "This line couldn't be translated and was left out of the file.";

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
    /// The run's Stop: watched between requests, tripped by a fatal error.
    pub stop: RunStop,
    pub tx: mpsc::Sender<RunEvent>,
    /// Translations an earlier run left for this file. Used only when they
    /// belong to the source as it is now; only the missing lines are sent.
    pub seed: Option<Seed>,
}

/// Stored translations to start from.
#[derive(Debug, Clone, Default)]
pub struct Seed {
    /// Fingerprint of the source they were made from.
    pub source_hash: String,
    /// Line id → translation as written, tags in.
    pub translations: BTreeMap<u32, String>,
}

/// How one file ended.
#[derive(Debug, Clone, PartialEq)]
pub enum FileEnd {
    Done,
    NeedsLook,
    /// It failed on its own; `reason` is one plain sentence.
    Failed { reason: String, kind: ErrorKind },
    /// The run stopped before it finished. Its translations so far are kept.
    Stopped,
}

/// A file's end, with what the results store keeps of it.
#[derive(Debug)]
pub struct FileOutcome {
    pub result: FileResult,
    pub end: FileEnd,
    /// Every translation it has (tags in): all of a finished file's, the
    /// partial ones of a stopped file, none of a failed one.
    pub translations: BTreeMap<u32, String>,
    /// Fingerprint of the source, when it could be read.
    pub source_hash: Option<String>,
    /// The output written next to the source, if any.
    pub output_name: Option<String>,
}

/// Why a file didn't get to the end.
#[derive(Debug)]
enum FileError {
    /// The run stopped (by the user, or by a fatal error here or elsewhere).
    Stopped,
    /// This file failed on its own.
    Failed { message: String, kind: ErrorKind },
}

/// A finished file, before its events go out.
struct Finished {
    has_warnings: bool,
    issues: Vec<VerifyIssue>,
    output_name: String,
    output_path: PathBuf,
}

/// Translate one `.ass` file end-to-end. Never panics outward: every way it
/// can end becomes a [`FileOutcome`], with the matching events sent (a
/// failure also sends `RunEvent::Error`; a stop sends `State(Stopped)`).
pub async fn translate_file(job: FileJob<'_>) -> FileOutcome {
    REQUEST_FILE.scope(job.file_name.clone(), translate_in_scope(job)).await
}

async fn translate_in_scope(job: FileJob<'_>) -> FileOutcome {
    let original = match decode_file(&job.input) {
        Ok(text) => text,
        Err(e) => {
            let st = FileState::new(&job, Vec::new());
            let failed = FileError::Failed {
                message: format!("Couldn't read this file: {e}"),
                kind: ErrorKind::Other,
            };
            return st.finish(Err(failed), None).await;
        }
    };
    let source_hash = fingerprint(&original);
    let mut st = FileState::new(&job, parse_dialogues(&original));
    if let Some(seed) = job.seed.as_ref().filter(|s| s.source_hash == source_hash) {
        st.translations = st.valid(&seed.translations);
    }
    let result = st.run(&original).await;
    st.finish(result, Some(source_hash)).await
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

/// Pure: the lines a reviewer asked for, as scopes of adjacent ids. Each
/// scope's context is the translated lines just before it.
pub fn review_scopes(ids: &[u32]) -> Vec<Scope> {
    let sorted: BTreeSet<u32> = ids.iter().copied().collect();
    let mut scopes: Vec<Scope> = Vec::new();
    for id in sorted {
        match scopes.last_mut() {
            Some(s) if s.end_line + 1 == id => {
                s.end_line = id;
                s.context_end = id;
            }
            _ => scopes.push(Scope { start_line: id, end_line: id, context_start: id, context_end: id }),
        }
    }
    scopes
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
    /// Why the last retry happened, in plain words.
    retry_reason: Option<String>,
    /// The kind of the last batch that brought nothing back.
    last_failure: Option<ErrorKind>,
    settings: BatchSettings,
}

impl<'j, 'a> FileState<'j, 'a> {
    fn new(job: &'j FileJob<'a>, lines: Vec<DialogueLine>) -> Self {
        Self {
            job,
            lines,
            translations: BTreeMap::new(),
            context: Vec::new(),
            retries: 0,
            retry_reason: None,
            last_failure: None,
            settings: BatchSettings {
                pair: job.pair.clone(),
                template: job.prompts.template.clone(),
                tone_text: job.prompts.tone.clone(),
            },
        }
    }

    /// The stored translations that fit this file's lines.
    fn valid(&self, stored: &BTreeMap<u32, String>) -> BTreeMap<u32, String> {
        let last = self.lines.len() as u32;
        stored
            .iter()
            .filter(|(id, _)| (1..=last).contains(*id))
            .map(|(id, t)| (*id, t.clone()))
            .collect()
    }

    async fn run(&mut self, original: &str) -> Result<Finished, FileError> {
        let job = self.job;
        // 1. Parse.
        self.emit(RunEvent::EpisodeLines {
            file: job.file_name.clone(),
            lines: episode_sources(&self.lines),
        })
        .await;
        if !self.translations.is_empty() {
            let kept: Vec<u32> = self.translations.keys().copied().collect();
            self.landed(&kept).await;
            self.log(
                LogLevel::Info,
                LogPhase::Parse,
                format!("carrying on: {} of {} lines already translated", kept.len(), self.lines.len()),
            )
            .await;
        }
        self.state(FileStateKind::Translating, None).await;
        self.log(LogLevel::Info, LogPhase::Parse, format!("{} dialogue lines", self.lines.len())).await;
        if self.lines.is_empty() {
            return Err(FileError::Failed {
                message: "This file has no dialogue lines.".into(),
                kind: ErrorKind::Other,
            });
        }
        let detector = SourceDetector::for_language(&job.pair.source);

        // 2. Initial batch loop + 3. cleanup pass.
        self.translate_all().await?;
        if self.translations.is_empty() {
            return Err(self.nothing_translated());
        }
        self.run_cleanup(detector.as_ref()).await?;

        // 4. Verify + scoped retranslation loop (`translator.py:176-311`).
        let all_ids: Vec<u32> = (1..=self.lines.len() as u32).collect();
        let mut issues: Vec<VerifyIssue> = Vec::new();
        let mut has_warnings = false;
        for attempt in 1..=MAX_RETRANSLATION_ATTEMPTS {
            if job.stop.is_stopped() {
                return Err(FileError::Stopped);
            }
            self.state(FileStateKind::Verifying, None).await;
            let stripped = self.stripped_pairs();
            let report =
                verify_file(job.verify_svc, &stripped, &job.glossary.all_terms(), &job.prompts.verify)
                    .await;
            let report = match report {
                Ok(r) => r,
                Err(e) if e.is_cancelled() || job.stop.is_stopped() => {
                    return Err(FileError::Stopped)
                }
                // A dead key (or credit, or model) mid-verify stops the run,
                // as it does in translation: never write unverified output.
                Err(e) => {
                    return Err(self.fatal(Step::Verify, job.verify_svc, e.kind(), e.to_string()).await)
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
            self.state(
                FileStateKind::Retranslating,
                Some(format!("attempt {attempt}/{MAX_RETRANSLATION_ATTEMPTS}")),
            )
            .await;
            if is_full_file(&scopes, &all_ids) {
                self.log(
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
                // has_warnings will be set — correct behaviour. A stop mid-redo
                // keeps them too.
                let prior = std::mem::take(&mut self.translations);
                let redo = self.translate_all().await;
                // Backfill: keep any id that translate_all() did not produce.
                for (id, t) in prior {
                    self.translations.entry(id).or_insert(t);
                }
                redo?;
            } else {
                let ranges = scopes
                    .iter()
                    .map(|s| format!("{}-{}", s.start_line, s.end_line))
                    .collect::<Vec<_>>()
                    .join(", ");
                self.log(
                    LogLevel::Info,
                    LogPhase::Retranslate,
                    format!(
                        "verify found {} issues — retranslating scopes {ranges} (attempt {attempt}/{MAX_RETRANSLATION_ATTEMPTS})",
                        report.issues.len()
                    ),
                )
                .await;
                for scope in &scopes {
                    self.retranslate_scope(scope, attempt, MAX_RETRANSLATION_ATTEMPTS).await?;
                }
            }
            // Re-run cleanup after any retranslation (`translator.py:307-311`).
            self.run_cleanup(detector.as_ref()).await?;
        }

        // 5. Final write.
        let untranslated = self.lines.len() - self.translations.len();
        has_warnings |= untranslated > 0 || !issues.is_empty();

        // Warning-only files (untranslated lines, no verify issues) must still be
        // expandable in the UI: synthesize one issue per untranslated line
        // (capped) AFTER the real verify issues, plus an aggregate overflow entry.
        // FileDone and FileResult share this vec, so both stay identical.
        if untranslated > 0 {
            let missing: Vec<u32> = (1..=self.lines.len() as u32)
                .filter(|id| !self.translations.contains_key(id))
                .collect();
            for id in missing.iter().take(MAX_UNTRANSLATED_ISSUES) {
                issues.push(VerifyIssue {
                    line_id: *id,
                    source: strip_for_text(self.raw_text(*id)),
                    translation: String::new(),
                    issue_type: "untranslated".into(),
                    description: UNTRANSLATED_DESCRIPTION.into(),
                    severity: "warning".into(),
                    term: None,
                    expected: None,
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
                    term: None,
                    expected: None,
                });
            }
        }

        let output_name = if has_warnings {
            warning_filename(&job.input, &job.pair)
        } else {
            output_filename(&job.input, &job.pair)
        };
        let output_path = self.write_output(original, &output_name)?;
        Ok(Finished { has_warnings, issues, output_name, output_path })
    }

    /// Write every translated line next to the source. Untranslated lines are
    /// dropped, not written back as source text (spec).
    fn write_output(&self, original: &str, name: &str) -> Result<PathBuf, FileError> {
        let path = match self.job.input.parent() {
            Some(parent) => parent.join(name),
            None => PathBuf::from(name),
        };
        let out_lines: Vec<DialogueLine> = self
            .lines
            .iter()
            .enumerate()
            .filter_map(|(idx, d)| {
                self.translations.get(&(idx as u32 + 1)).map(|t| {
                    let mut d = d.clone();
                    d.text = t.clone();
                    d
                })
            })
            .collect();
        write_translated(&path, original, &out_lines).map_err(|e| FileError::Failed {
            message: format!("Couldn't write {name}: {e}"),
            kind: ErrorKind::Other,
        })?;
        Ok(path)
    }

    /// Send the file's last events and say how it ended.
    async fn finish(self, result: Result<Finished, FileError>, source_hash: Option<String>) -> FileOutcome {
        let file = self.job.file_name.clone();
        let total_lines = self.lines.len() as u32;
        let translated_lines = self.translations.len() as u32;
        match result {
            Ok(f) => {
                self.emit(RunEvent::FileDone {
                    file: file.clone(),
                    has_warnings: f.has_warnings,
                    issues: f.issues.clone(),
                })
                .await;
                self.state(
                    if f.has_warnings { FileStateKind::Warning } else { FileStateKind::Done },
                    None,
                )
                .await;
                FileOutcome {
                    result: FileResult {
                        file,
                        success: true,
                        total_lines,
                        translated_lines,
                        has_warnings: f.has_warnings,
                        issues: f.issues,
                        output_path: Some(f.output_path.display().to_string()),
                    },
                    end: if f.has_warnings { FileEnd::NeedsLook } else { FileEnd::Done },
                    translations: self.translations,
                    source_hash,
                    output_name: Some(f.output_name),
                }
            }
            Err(FileError::Stopped) => {
                self.state(FileStateKind::Stopped, None).await;
                FileOutcome {
                    result: FileResult {
                        file,
                        success: false,
                        total_lines,
                        translated_lines,
                        has_warnings: false,
                        issues: Vec::new(),
                        output_path: None,
                    },
                    end: FileEnd::Stopped,
                    translations: self.translations,
                    source_hash,
                    output_name: None,
                }
            }
            Err(FileError::Failed { message, kind }) => {
                self.log(LogLevel::Error, LogPhase::Error, message.clone()).await;
                self.emit(RunEvent::Error { file: file.clone(), message: message.clone(), error_kind: kind })
                    .await;
                self.state(FileStateKind::Failed, None).await;
                FileOutcome {
                    result: FileResult {
                        file,
                        success: false,
                        total_lines,
                        translated_lines: 0,
                        has_warnings: false,
                        issues: Vec::new(),
                        output_path: None,
                    },
                    end: FileEnd::Failed { reason: message, kind },
                    translations: BTreeMap::new(),
                    source_hash,
                    output_name: None,
                }
            }
        }
    }

    /// An error no retry fixes: log it as it came, stop the run with a plain
    /// cause, and end this file as stopped like every other unfinished one.
    async fn fatal(&self, step: Step, svc: &LlmService, kind: ErrorKind, message: String) -> FileError {
        self.log(LogLevel::Error, LogPhase::Error, format!("{} ({}): {message}", step.label(), svc.who()))
            .await;
        self.job.stop.by_error(StopCause {
            kind,
            step,
            who: svc.who().to_string(),
            model: svc.model().to_string(),
        });
        FileError::Stopped
    }

    /// Every batch came back empty-handed: say why in plain words.
    fn nothing_translated(&self) -> FileError {
        let who = self.job.svc.who();
        let kind = self.last_failure.unwrap_or(ErrorKind::Other);
        let message = match kind {
            ErrorKind::RateLimit => format!("{who} kept asking us to slow down, and no line got translated."),
            ErrorKind::Network => format!("Couldn't reach {who}, so no line got translated."),
            ErrorKind::Server => format!("{who} kept failing on its side, so no line got translated."),
            _ => "No reply could be used, so no line got translated.".to_string(),
        };
        FileError::Failed { message, kind }
    }

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
            retry_reason: self.retry_reason.clone(),
        })
        .await;
    }

    /// A batch has to go again; say why on the next progress.
    fn retried(&mut self, reason: &str) {
        self.retries += 1;
        self.retry_reason = Some(reason.to_string());
    }

    /// Live board: these ids just went to the model.
    async fn sending(&self, ids: Vec<u32>, activity: EpisodeActivity) {
        self.emit(RunEvent::Sending { file: self.job.file_name.clone(), ids, activity }).await;
    }

    /// Live board: what these ids read now that their translations merged.
    async fn landed(&self, ids: &[u32]) {
        let lines: Vec<(u32, String)> = ids
            .iter()
            .filter_map(|id| self.translations.get(id).map(|t| (*id, strip_for_text(t))))
            .collect();
        if !lines.is_empty() {
            self.emit(RunEvent::Landed { file: self.job.file_name.clone(), lines }).await;
        }
    }

    /// Live board: the request announced by the last `sending` is over. Not
    /// sent after a fatal outcome or a stop: the file's final state settles
    /// the board instead.
    async fn idle(&self) {
        self.emit(RunEvent::Idle { file: self.job.file_name.clone() }).await;
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
    /// (caller clears `translations` first) and Resume (seeded lines skip).
    async fn translate_all(&mut self) -> Result<(), FileError> {
        let mut pending: Vec<u32> = (1..=self.lines.len() as u32)
            .filter(|id| !self.translations.contains_key(id))
            .collect();
        let mut batch_size = initial_batch_size(self.job.batch_limit);
        self.context.clear();
        let mut batch_num: u32 = 0;
        let mut total_batches = pending.len().div_ceil(batch_size as usize) as u32;

        while !pending.is_empty() {
            if self.job.stop.is_stopped() {
                return Err(FileError::Stopped);
            }
            batch_num += 1;
            let n = (batch_size as usize).min(pending.len());
            let take: Vec<u32> = pending.drain(..n).collect();
            let raw: Vec<(u32, String)> =
                take.iter().map(|id| (*id, self.raw_text(*id).to_string())).collect();

            self.sending(take.clone(), EpisodeActivity::Batch { batch: batch_num, total_batches })
                .await;
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
                    self.landed(&merged).await;
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
                            match d.suspected_drift_start_id {
                                Some(line) => format!(
                                    "drift suspected from line {line} (score {:.2}, threshold {})",
                                    d.score,
                                    drift::THRESHOLD
                                ),
                                None => format!(
                                    "drift suspected (score {:.2}, threshold {})",
                                    d.score,
                                    drift::THRESHOLD
                                ),
                            },
                        )
                        .await;
                    }
                }
                BatchOutcome::Partial { translated, failed_from, reason } => {
                    let merged: Vec<u32> = translated.keys().copied().collect();
                    let merged_count = merged.len();
                    self.translations.extend(translated);
                    self.landed(&merged).await;
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
                    self.retried(reason);
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
                // The run stopped while the request was out: nothing failed.
                BatchOutcome::Failure(f) if f.cancelled || self.job.stop.is_stopped() => {
                    return Err(FileError::Stopped);
                }
                BatchOutcome::Failure(f) => {
                    self.retried(f.reason);
                    self.last_failure = Some(f.kind);
                    self.log(LogLevel::Warning, LogPhase::Batch, f.message).await;
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
                BatchOutcome::Fatal(f) => {
                    // A dead key (or credit, or model): the whole run is doomed.
                    return Err(self.fatal(Step::Translate, self.job.svc, f.kind, f.message).await);
                }
            }
            self.idle().await;
        }
        Ok(())
    }

    /// Cleanup pass for residual source text (`translator.py:583-684`). No-op
    /// when the source language has no character pattern (detector is None).
    async fn run_cleanup(&mut self, detector: Option<&SourceDetector>) -> Result<(), FileError> {
        let Some(det) = detector else { return Ok(()) };
        self.state(FileStateKind::Cleanup, None).await;
        let sources: BTreeMap<u32, String> = self
            .lines
            .iter()
            .enumerate()
            .map(|(idx, d)| (idx as u32 + 1, d.text.clone()))
            .collect();
        // The same dirty set `cleanup_pass` computes, so the live board can
        // show the request it is about to make. Over the cap it sends nothing.
        let dirty: Vec<u32> = self
            .translations
            .iter()
            .filter(|(_, text)| det.needs_cleanup(text))
            .map(|(id, _)| *id)
            .collect();
        let announced = !dirty.is_empty() && dirty.len() <= MAX_CLEANUP_LINES;
        if announced {
            self.sending(dirty, EpisodeActivity::Cleanup).await;
        }
        let report = cleanup_pass(
            self.job.svc,
            det,
            &sources,
            &mut self.translations,
            self.job.glossary,
            &self.settings,
        )
        .await;
        self.landed(&report.cleaned).await;
        if let Some(f) = report.fatal {
            let message = format!("cleanup: {}", f.message);
            return Err(self.fatal(Step::Translate, self.job.svc, f.kind, message).await);
        }
        if self.job.stop.is_stopped() {
            return Err(FileError::Stopped);
        }
        if announced {
            self.idle().await;
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
    /// retranslated and filtered back out). Returns the ids that got new text.
    ///
    /// Deviation from Python: scope lines are chunked by `initial_batch_size`
    /// (sequential, merging each outcome) so large scopes don't blow the
    /// model's context window.
    ///
    /// `attempt` / `max_attempts` are the verify loop's (1 of 1 for a
    /// reviewer's request), for the live board's activity line.
    async fn retranslate_scope(
        &mut self,
        scope: &Scope,
        attempt: u32,
        max_attempts: u32,
    ) -> Result<Vec<u32>, FileError> {
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
        let mut landed = Vec::new();
        if raw.is_empty() {
            return Ok(landed);
        }
        let chunk_size = initial_batch_size(self.job.batch_limit) as usize;
        for chunk in raw.chunks(chunk_size) {
            if self.job.stop.is_stopped() {
                return Err(FileError::Stopped);
            }
            self.sending(
                chunk.iter().map(|(id, _)| *id).collect(),
                EpisodeActivity::Retranslate { attempt, max_attempts },
            )
            .await;
            match translate_batch_tagged(
                self.job.svc,
                chunk,
                self.job.glossary,
                &context,
                &self.settings,
            )
            .await
            {
                // Partial: keep what we got; the next verify pass decides if it stuck.
                BatchOutcome::Success(map) | BatchOutcome::Partial { translated: map, .. } => {
                    let merged: Vec<u32> = map.keys().copied().collect();
                    self.translations.extend(map);
                    self.landed(&merged).await;
                    landed.extend(merged);
                }
                BatchOutcome::Failure(f) if f.cancelled || self.job.stop.is_stopped() => {
                    return Err(FileError::Stopped);
                }
                BatchOutcome::Failure(f) => {
                    self.retried(f.reason);
                    self.log(LogLevel::Warning, LogPhase::Retranslate, f.message).await;
                    // Log and continue to the next chunk; don't abort the scope.
                }
                BatchOutcome::Fatal(f) => {
                    return Err(self.fatal(Step::Translate, self.job.svc, f.kind, f.message).await);
                }
            }
            self.idle().await;
        }
        Ok(landed)
    }
}

// ── review: retranslate chosen lines of a finished episode ──────────────────

/// A reviewer's request to retranslate some lines of a finished episode.
pub struct ReviewJob<'a> {
    /// `seed` holds the episode's stored translations (required).
    pub job: FileJob<'a>,
    pub line_ids: Vec<u32>,
    /// The output to rewrite, next to the source.
    pub output_name: String,
    /// Issues still open before this; those on lines that get new text go.
    pub issues: Vec<VerifyIssue>,
    /// The episode still needs a look (its output is the `.warning` file).
    pub needs_look: bool,
}

/// How a review retranslation went.
#[derive(Debug)]
pub struct ReviewOutcome {
    pub result: FileResult,
    /// Every translation the episode has now (tags in). `result.issues`
    /// holds the issues still open.
    pub translations: BTreeMap<u32, String>,
    /// Lines that got new text.
    pub retranslated: Vec<u32>,
    /// Why nothing could be done (unreadable or changed source, a write
    /// error), as one plain sentence. Nothing was written or stored.
    pub failed: Option<String>,
}

/// Retranslate just `line_ids`, each run of adjacent lines as one scope with
/// the translated lines before it as context, then rewrite the output with
/// everything else as it was. Runs as a small run: the episode's events go
/// out like any file's, ending in its previous state (done or needs a look).
pub async fn retranslate_episode_lines(review: ReviewJob<'_>) -> ReviewOutcome {
    REQUEST_FILE.scope(review.job.file_name.clone(), review_in_scope(review)).await
}

async fn review_in_scope(review: ReviewJob<'_>) -> ReviewOutcome {
    let job = &review.job;
    let fail = |st: &FileState<'_, '_>, message: String| ReviewOutcome {
        result: FileResult {
            file: job.file_name.clone(),
            success: false,
            total_lines: st.lines.len() as u32,
            translated_lines: 0,
            has_warnings: review.needs_look,
            issues: review.issues.clone(),
            output_path: None,
        },
        translations: BTreeMap::new(),
        retranslated: Vec::new(),
        failed: Some(message),
    };
    let original = match decode_file(&job.input) {
        Ok(text) => text,
        Err(e) => {
            let st = FileState::new(job, Vec::new());
            let message = format!("Couldn't read the original file: {e}");
            st.emit(RunEvent::Error {
                file: job.file_name.clone(),
                message: message.clone(),
                error_kind: ErrorKind::Other,
            })
            .await;
            st.state(FileStateKind::Failed, None).await;
            return fail(&st, message);
        }
    };
    let mut st = FileState::new(job, parse_dialogues(&original));
    let prior_state = if review.needs_look { FileStateKind::Warning } else { FileStateKind::Done };
    let seed = job.seed.as_ref().filter(|s| s.source_hash == fingerprint(&original));
    let Some(seed) = seed else {
        let message =
            "The original file changed since it was translated. Translate it again to review it."
                .to_string();
        st.emit(RunEvent::Error {
            file: job.file_name.clone(),
            message: message.clone(),
            error_kind: ErrorKind::Other,
        })
        .await;
        st.state(prior_state, None).await;
        return fail(&st, message);
    };
    st.translations = st.valid(&seed.translations);

    st.emit(RunEvent::EpisodeLines { file: job.file_name.clone(), lines: episode_sources(&st.lines) })
        .await;
    let kept: Vec<u32> = st.translations.keys().copied().collect();
    st.landed(&kept).await;

    let last = st.lines.len() as u32;
    let ids: Vec<u32> = review.line_ids.iter().copied().filter(|id| (1..=last).contains(id)).collect();
    let scopes = review_scopes(&ids);
    let detail = match ids.len() {
        1 => format!("line {}", ids[0]),
        n => format!("{n} lines"),
    };
    st.state(FileStateKind::Retranslating, Some(detail)).await;

    let mut retranslated: Vec<u32> = Vec::new();
    let total = scopes.len() as u32;
    for (i, scope) in scopes.iter().enumerate() {
        match st.retranslate_scope(scope, 1, 1).await {
            Ok(ids) => retranslated.extend(ids),
            // Stopped: keep what landed; the run says why.
            Err(_) => break,
        }
        st.progress(i as u32 + 1, total).await;
    }
    retranslated.sort_unstable();
    retranslated.dedup();

    let fixed: BTreeSet<u32> = retranslated.iter().copied().collect();
    let issues: Vec<VerifyIssue> =
        review.issues.iter().filter(|i| !fixed.contains(&i.line_id)).cloned().collect();
    let mut output_path = None;
    if !retranslated.is_empty() {
        match st.write_output(&original, &review.output_name) {
            Ok(path) => output_path = Some(path.display().to_string()),
            Err(FileError::Failed { message, kind }) => {
                st.emit(RunEvent::Error { file: job.file_name.clone(), message: message.clone(), error_kind: kind })
                    .await;
                st.state(prior_state, None).await;
                return fail(&st, message);
            }
            Err(FileError::Stopped) => unreachable!("writing never stops"),
        }
    }
    st.emit(RunEvent::FileDone {
        file: job.file_name.clone(),
        has_warnings: review.needs_look,
        issues: issues.clone(),
    })
    .await;
    st.state(prior_state, None).await;
    ReviewOutcome {
        result: FileResult {
            file: job.file_name.clone(),
            success: true,
            total_lines: last,
            translated_lines: st.translations.len() as u32,
            has_warnings: review.needs_look,
            issues,
            output_path,
        },
        translations: st.translations,
        retranslated,
        failed: None,
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

    /// One file through the pipeline, with everything a test may look at.
    struct Ran {
        outcome: FileOutcome,
        events: Vec<RunEvent>,
        driver: std::sync::Arc<ScriptedDriver>,
        /// Keeps the temp folder (source and output) alive.
        dir: tempfile::TempDir,
        stop: RunStop,
    }

    async fn run_with(
        source: &str,
        responses: Vec<Result<String, crate::llm::error::LlmError>>,
        seed: Option<Seed>,
    ) -> Ran {
        let driver = ScriptedDriver::new(responses);
        let (tx, mut rx) = mpsc::channel(256);
        let stop = RunStop::default();
        let svc = LlmService::new(driver.clone(), 2, stop.token().clone(), tx.clone())
            .with_provider("Anthropic");
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("ep01.ass");
        std::fs::write(&input, source).unwrap();
        let prompts = crate::prompts::TranslationPrompts::defaults(
            &LanguagePair::from_codes("zh", "en").unwrap(),
            crate::config::projects::Tone::Standard,
        );

        // One scripted driver behind both steps, so a script reads in call order.
        let outcome = translate_file(FileJob {
            input: input.clone(),
            file_name: "ep01.ass".into(),
            svc: &svc,
            verify_svc: &svc,
            glossary: &Glossary::default(),
            pair: LanguagePair::from_codes("zh", "en").unwrap(),
            prompts: &prompts,
            batch_limit: Some(100),
            stop: stop.clone(),
            tx: tx.clone(),
            seed,
        })
        .await;

        drop(tx);
        drop(svc); // the service holds a tx clone — drop it so recv() can end
        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        Ran { outcome, events, driver, dir, stop }
    }

    async fn run_pipeline(
        source: &str,
        responses: Vec<Result<String, crate::llm::error::LlmError>>,
    ) -> (FileResult, Vec<RunEvent>, std::sync::Arc<ScriptedDriver>, tempfile::TempDir) {
        let ran = run_with(source, responses, None).await;
        // Return `dir` so callers keep it alive for as long as needed (e.g.
        // reading the output file). Bind to `_dir` when not needed.
        (ran.outcome.result, ran.events, ran.driver, ran.dir)
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

    /// The live-board events, in order, as compact tags. The service's
    /// request timing events are left out (see `requests_tell_the_board…`).
    fn board_events(events: &[RunEvent]) -> Vec<String> {
        events
            .iter()
            .filter(|e| e.is_internal())
            .filter(|e| !matches!(e, RunEvent::RequestSent { .. } | RunEvent::RequestWaiting { .. }))
            .map(|e| match e {
                RunEvent::EpisodeLines { lines, .. } => format!("lines {}", lines.len()),
                RunEvent::Sending { ids, activity, .. } => format!("sending {ids:?} {activity:?}"),
                RunEvent::Landed { lines, .. } => format!("landed {lines:?}"),
                RunEvent::Idle { .. } => "idle".into(),
                _ => unreachable!("filtered to internal events"),
            })
            .collect()
    }

    #[tokio::test]
    async fn batch_tells_the_live_board_sending_landed_idle() {
        let src = ass_source(&["{\\an8}你好", "再见"]);
        let (result, events, _, _dir) = run_pipeline(
            &src,
            vec![
                Ok(ok_batch(&[(1, "Hello there friend"), (2, "Goodbye for now")])),
                Ok(r#"{"issues":[]}"#.into()),
            ],
        )
        .await;
        assert!(result.success);
        let lines = events
            .iter()
            .find_map(|e| match e {
                RunEvent::EpisodeLines { lines, .. } => Some(lines.clone()),
                _ => None,
            })
            .expect("EpisodeLines after parse");
        assert_eq!((lines[0].id, lines[0].text.as_str()), (1, "你好"));
        assert_eq!((lines[0].start_cs, lines[0].end_cs), (100, 150));
        // Landed text is stripped: the reapplied {\an8} never reaches the board.
        assert_eq!(
            board_events(&events),
            vec![
                "lines 2".to_string(),
                "sending [1, 2] Batch { batch: 1, total_batches: 1 }".into(),
                r#"landed [(1, "Hello there friend"), (2, "Goodbye for now")]"#.into(),
                "idle".into(),
            ]
        );
    }

    #[tokio::test]
    async fn partial_batch_lands_and_goes_idle_before_its_resend() {
        let src = ass_source(&["你好", "再见", "走吧"]);
        let (_, events, _, _dir) = run_pipeline(
            &src,
            vec![
                Ok(ok_batch(&[(1, "Hello my good friend")])), // partial: 2, 3 requeued
                Ok(ok_batch(&[(2, "Goodbye then friend"), (3, "Off we go now")])),
                Ok(r#"{"issues":[]}"#.into()),
            ],
        )
        .await;
        assert_eq!(
            board_events(&events),
            vec![
                "lines 3".to_string(),
                "sending [1, 2, 3] Batch { batch: 1, total_batches: 1 }".into(),
                r#"landed [(1, "Hello my good friend")]"#.into(),
                "idle".into(),
                "sending [2, 3] Batch { batch: 2, total_batches: 2 }".into(),
                r#"landed [(2, "Goodbye then friend"), (3, "Off we go now")]"#.into(),
                "idle".into(),
            ]
        );
    }

    #[tokio::test]
    async fn failed_batch_goes_idle_without_landing_then_resends() {
        let src = ass_source(&["你好", "再见"]);
        let (result, events, _, _dir) = run_pipeline(
            &src,
            vec![
                Ok("not json".into()), // Failure: halve and requeue both lines
                Ok(ok_batch(&[(1, "Hello there friend"), (2, "Goodbye for now")])),
                Ok(r#"{"issues":[]}"#.into()),
            ],
        )
        .await;
        assert!(result.success);
        assert_eq!(
            board_events(&events),
            vec![
                "lines 2".to_string(),
                "sending [1, 2] Batch { batch: 1, total_batches: 1 }".into(),
                "idle".into(),
                "sending [1, 2] Batch { batch: 2, total_batches: 2 }".into(),
                r#"landed [(1, "Hello there friend"), (2, "Goodbye for now")]"#.into(),
                "idle".into(),
            ]
        );
    }

    #[tokio::test]
    async fn fatal_cleanup_leaves_the_board_to_the_stopped_state() {
        let src = ass_source(&["你好"]);
        let (_, events, _, _dir) = run_pipeline(
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
        // No Idle after the cleanup request: the Stopped state settles the board.
        assert_eq!(
            board_events(&events),
            vec![
                "lines 1".to_string(),
                "sending [1] Batch { batch: 1, total_batches: 1 }".into(),
                r#"landed [(1, "还是中文 leftover")]"#.into(),
                "idle".into(),
                "sending [1] Cleanup".into(),
            ]
        );
        assert!(matches!(events.last(), Some(RunEvent::State { state: FileStateKind::Stopped, .. })));
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
        assert_eq!(
            board_events(&events)[4..],
            [
                "sending [1] Cleanup".to_string(),
                r#"landed [(1, "Fully English now friend")]"#.into(),
                "idle".into(),
            ]
        );
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
        // Only line 6 goes back, tagged with the verify attempt that asked.
        assert_eq!(
            board_events(&events)[4..],
            [
                "sending [6] Retranslate { attempt: 1, max_attempts: 3 }".to_string(),
                r#"landed [(6, "Retranslated line six properly")]"#.into(),
                "idle".into(),
            ]
        );
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
        let stop = RunStop::default();
        let svc = LlmService::new(translator.clone(), 2, stop.token().clone(), tx.clone());
        let verify_svc = LlmService::new(verifier.clone(), 2, stop.token().clone(), tx.clone());
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
            stop,
            tx,
            seed: None,
        })
        .await
        .result;
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
        let (result, events, driver, _dir) = run_pipeline(
            &src,
            vec![Err(crate::llm::error::LlmError::Http { status: 401, body: "no".into(), retry_after: None })],
        )
        .await;
        assert!(!result.success);
        assert_eq!(driver.call_count(), 1);
        // No Idle after Fatal: the Error event settles the board.
        assert_eq!(
            board_events(&events).last().unwrap(),
            "sending [1, 2] Batch { batch: 1, total_batches: 1 }"
        );
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
    async fn dead_key_during_verify_stops_the_run_not_silent_clean() {
        let src = ass_source(&["你好", "再见"]);
        let ran = run_with(
            &src,
            vec![
                Ok(ok_batch(&[(1, "Hello there friend"), (2, "Goodbye for now")])),
                Err(crate::llm::error::LlmError::Http {
                    status: 401,
                    body: "dead key".into(),
                    retry_after: None,
                }),
            ],
            None,
        )
        .await;
        let result = &ran.outcome.result;
        assert!(!result.success);
        assert!(result.output_path.is_none());
        assert_eq!(ran.outcome.end, FileEnd::Stopped);
        // Both lines were translated: kept for Resume.
        assert_eq!(ran.outcome.translations.len(), 2);
        assert_eq!(result.translated_lines, 2);
        let info = ran.stop.info().expect("the run stopped");
        assert_eq!(info.message, "Anthropic rejected the API key used for Verify.");
        assert_eq!((info.kind, info.step), (Some(ErrorKind::Auth), Some(Step::Verify)));
        // Not a failure of this file, and the raw error reaches the log.
        assert!(!ran.events.iter().any(|e| matches!(e, RunEvent::Error { .. })));
        assert!(ran.events.iter().any(|e| matches!(
            e,
            RunEvent::Log { level: LogLevel::Error, message, .. } if message.contains("dead key")
        )));
        assert!(!ran.dir.path().join("ep01.eng.ass").exists());
    }

    #[tokio::test]
    async fn dead_key_during_cleanup_stops_the_run() {
        let src = ass_source(&["你好"]);
        // The translated text still contains source-language characters, so the
        // cleanup pass fires; its single request dies with 401.
        let ran = run_with(
            &src,
            vec![
                Ok(ok_batch(&[(1, "还是中文 leftover")])),
                Err(crate::llm::error::LlmError::Http {
                    status: 401,
                    body: "dead".into(),
                    retry_after: None,
                }),
            ],
            None,
        )
        .await;
        assert!(!ran.outcome.result.success);
        assert_eq!(ran.outcome.end, FileEnd::Stopped);
        let cause = ran.stop.cause().expect("a cause");
        assert_eq!((cause.kind, cause.step), (ErrorKind::Auth, Step::Translate));
        assert!(ran.events.iter().any(|e| matches!(
            e,
            RunEvent::Log { level: LogLevel::Error, message, .. } if message.contains("cleanup")
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
        let (tx, mut rx) = mpsc::channel(256);
        let stop = RunStop::default();
        let svc = LlmService::new(driver, 2, stop.token().clone(), tx.clone());
        stop.by_user();
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("ep01.ass");
        std::fs::write(&input, &src).unwrap();
        let prompts = crate::prompts::TranslationPrompts::defaults(
            &LanguagePair::from_codes("zh", "en").unwrap(),
            crate::config::projects::Tone::Standard,
        );
        let outcome = translate_file(FileJob {
            input,
            file_name: "ep01.ass".into(),
            svc: &svc,
            verify_svc: &svc,
            glossary: &Glossary::default(),
            pair: LanguagePair::from_codes("zh", "en").unwrap(),
            prompts: &prompts,
            batch_limit: Some(100),
            stop,
            tx,
            seed: None,
        })
        .await;
        assert!(!outcome.result.success);
        assert!(outcome.result.output_path.is_none());
        // Stopped, not failed: no Error, a Stopped state.
        assert_eq!(outcome.end, FileEnd::Stopped);
        drop(svc);
        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        assert!(!events.iter().any(|e| matches!(e, RunEvent::Error { .. })));
        assert!(matches!(events.last(), Some(RunEvent::State { state: FileStateKind::Stopped, .. })));
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
        assert_eq!(done_issues[0].issue_type, "meaning");
    }
    /// Stops the run (as the user's Stop would) on its `at`-th request.
    struct StopOnCall {
        inner: std::sync::Arc<ScriptedDriver>,
        at: u32,
        calls: std::sync::atomic::AtomicU32,
        stop: RunStop,
    }

    #[async_trait::async_trait]
    impl crate::llm::LlmDriver for StopOnCall {
        async fn complete(&self, _s: &str, _u: &str) -> Result<String, crate::llm::error::LlmError> {
            unreachable!()
        }
        async fn stream(
            &self,
            req: &crate::llm::LlmRequest,
        ) -> Result<crate::llm::LlmResponse, crate::llm::error::LlmError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            if n == self.at {
                self.stop.by_user();
                return Err(crate::llm::error::LlmError::Transport(
                    crate::llm::error::CANCELLED_MSG.into(),
                ));
            }
            self.inner.stream(req).await
        }
        async fn list_models(&self) -> Result<Vec<String>, crate::llm::error::LlmError> {
            Ok(vec![])
        }
        fn model(&self) -> &str {
            "scripted"
        }
    }

    fn twelve() -> String {
        ass_source(&["你好", "再见", "走吧", "好的", "不行", "可以", "什么", "哪里", "怎么", "为何", "真的", "假的"])
    }

    #[tokio::test]
    async fn a_stop_mid_run_keeps_the_finished_batch_and_says_stopped() {
        let src = twelve();
        let first: Vec<(u32, &str)> = (1..=10).map(|i| (i, "A clean English line here")).collect();
        let stop = RunStop::default();
        let driver = std::sync::Arc::new(StopOnCall {
            inner: ScriptedDriver::new(vec![Ok(ok_batch(&first))]),
            at: 2,
            calls: Default::default(),
            stop: stop.clone(),
        });
        let (tx, mut rx) = mpsc::channel(256);
        let svc = LlmService::new(driver, 2, stop.token().clone(), tx.clone());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ep01.ass"), &src).unwrap();
        let pair = LanguagePair::from_codes("zh", "en").unwrap();
        let prompts =
            crate::prompts::TranslationPrompts::defaults(&pair, crate::config::projects::Tone::Standard);
        let outcome = translate_file(FileJob {
            input: dir.path().join("ep01.ass"),
            file_name: "ep01.ass".into(),
            svc: &svc,
            verify_svc: &svc,
            glossary: &Glossary::default(),
            pair,
            prompts: &prompts,
            batch_limit: Some(10),
            stop: stop.clone(),
            tx,
            seed: None,
        })
        .await;
        drop(svc);
        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        assert_eq!(outcome.end, FileEnd::Stopped);
        assert_eq!(outcome.translations.len(), 10, "the finished batch is kept");
        assert_eq!((outcome.result.translated_lines, outcome.result.total_lines), (10, 12));
        assert!(outcome.source_hash.is_some());
        assert!(outcome.output_name.is_none());
        assert!(!events.iter().any(|e| matches!(e, RunEvent::Error { .. })), "not a failure");
        assert!(!events.iter().any(|e| matches!(
            e,
            RunEvent::Log { message, .. } if message.contains("cancelled")
        )));
        assert!(matches!(events.last(), Some(RunEvent::State { state: FileStateKind::Stopped, .. })));
        assert!(stop.info().unwrap().by_user);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no output written");
    }

    #[tokio::test]
    async fn resume_sends_only_the_missing_lines() {
        let src = ass_source(&["你好", "再见", "走吧"]);
        let seed = Seed {
            source_hash: fingerprint(&src),
            translations: [(1, "Hello there friend".to_string()), (2, "Goodbye for now".to_string())]
                .into(),
        };
        let ran = run_with(
            &src,
            vec![Ok(ok_batch(&[(3, "Off we go now")])), Ok(r#"{"issues":[]}"#.into())],
            Some(seed),
        )
        .await;
        assert_eq!(ran.outcome.end, FileEnd::Done);
        assert_eq!(ran.outcome.result.translated_lines, 3);
        assert_eq!(ran.driver.call_count(), 2);
        let written =
            std::fs::read_to_string(ran.dir.path().join("ep01.eng.ass")).unwrap();
        assert!(written.contains("Hello there friend") && written.contains("Off we go now"));
        // The window sees the kept lines at once, then only line 3 goes out.
        assert_eq!(
            board_events(&ran.events)[..3],
            [
                "lines 3".to_string(),
                r#"landed [(1, "Hello there friend"), (2, "Goodbye for now")]"#.into(),
                "sending [3] Batch { batch: 1, total_batches: 1 }".into(),
            ]
        );
    }

    #[tokio::test]
    async fn a_seed_from_another_version_of_the_file_is_ignored() {
        let src = ass_source(&["你好", "再见"]);
        let seed = Seed {
            source_hash: fingerprint("an older file"),
            translations: [(1, "Stale".to_string())].into(),
        };
        let ran = run_with(
            &src,
            vec![
                Ok(ok_batch(&[(1, "Hello there friend"), (2, "Goodbye for now")])),
                Ok(r#"{"issues":[]}"#.into()),
            ],
            Some(seed),
        )
        .await;
        assert_eq!(ran.outcome.translations[&1], "Hello there friend");
        assert!(ran.driver.last_request().is_some());
    }

    #[tokio::test]
    async fn retries_say_why() {
        let src = ass_source(&["你好", "再见", "走吧"]);
        let (_, events, _, _dir) = run_pipeline(
            &src,
            vec![
                Ok(ok_batch(&[(1, "Hello my good friend")])), // 2 and 3 missing
                Ok(ok_batch(&[(2, "Goodbye then friend"), (3, "Off we go now")])),
                Ok(r#"{"issues":[]}"#.into()),
            ],
        )
        .await;
        let reasons: Vec<Option<String>> = events
            .iter()
            .filter_map(|e| match e {
                RunEvent::Progress { retries, retry_reason, .. } if *retries > 0 => Some(retry_reason.clone()),
                _ => None,
            })
            .collect();
        assert!(!reasons.is_empty());
        assert!(reasons.iter().all(|r| r.as_deref() == Some(crate::translation::batch::REASON_INCOMPLETE)));
    }

    #[tokio::test]
    async fn nothing_translated_is_a_failure_with_a_reason() {
        let src = ass_source(&["你好", "再见"]);
        let responses: Vec<_> = (0..30).map(|_| Ok("not json at all".to_string())).collect();
        let ran = run_with(&src, responses, None).await;
        match &ran.outcome.end {
            FileEnd::Failed { reason, kind } => {
                assert_eq!(*kind, ErrorKind::Other);
                assert!(reason.contains("no line got translated"), "{reason}");
            }
            other => panic!("expected failed, got {other:?}"),
        }
        assert!(ran.events.iter().any(|e| matches!(
            e,
            RunEvent::Error { message, error_kind: ErrorKind::Other, .. } if message.contains("no line got translated")
        )));
        assert!(ran.stop.info().is_none(), "one file failing doesn't stop the run");
    }

    #[tokio::test]
    async fn an_unreadable_file_fails_on_its_own() {
        let driver = ScriptedDriver::new(vec![]);
        let (tx, mut rx) = mpsc::channel(64);
        let stop = RunStop::default();
        let svc = LlmService::new(driver, 2, stop.token().clone(), tx.clone());
        let dir = tempfile::tempdir().unwrap();
        let pair = LanguagePair::from_codes("zh", "en").unwrap();
        let prompts =
            crate::prompts::TranslationPrompts::defaults(&pair, crate::config::projects::Tone::Standard);
        let outcome = translate_file(FileJob {
            input: dir.path().join("gone.ass"),
            file_name: "gone.ass".into(),
            svc: &svc,
            verify_svc: &svc,
            glossary: &Glossary::default(),
            pair,
            prompts: &prompts,
            batch_limit: None,
            stop,
            tx,
            seed: None,
        })
        .await;
        drop(svc);
        assert!(matches!(&outcome.end, FileEnd::Failed { reason, .. } if reason.starts_with("Couldn't read this file")));
        assert!(outcome.source_hash.is_none());
        let mut kinds = Vec::new();
        while let Some(e) = rx.recv().await {
            if let RunEvent::Error { error_kind, .. } = e {
                kinds.push(error_kind);
            }
        }
        assert_eq!(kinds, [ErrorKind::Other]);
    }

    #[tokio::test]
    async fn requests_tell_the_board_when_they_really_go_out() {
        let src = ass_source(&["你好", "再见"]);
        let (_, events, _, _dir) = run_pipeline(
            &src,
            vec![
                Ok(ok_batch(&[(1, "Hello there friend"), (2, "Goodbye for now")])),
                Ok(r#"{"issues":[]}"#.into()),
            ],
        )
        .await;
        let first_sending = events.iter().position(|e| matches!(e, RunEvent::Sending { .. })).unwrap();
        let first_sent = events.iter().position(|e| matches!(e, RunEvent::RequestSent { .. })).unwrap();
        let first_landed = events.iter().position(|e| matches!(e, RunEvent::Landed { .. })).unwrap();
        assert!(first_sending < first_sent && first_sent < first_landed);
    }

    #[test]
    fn review_scopes_group_adjacent_lines() {
        let scopes = review_scopes(&[9, 3, 4, 4, 12]);
        let spans: Vec<(u32, u32, u32)> =
            scopes.iter().map(|s| (s.start_line, s.end_line, s.context_start)).collect();
        assert_eq!(spans, [(3, 4, 3), (9, 9, 9), (12, 12, 12)]);
    }

    async fn review(
        src: &str,
        stored: &[(u32, &str)],
        hash: String,
        ids: Vec<u32>,
        responses: Vec<Result<String, crate::llm::error::LlmError>>,
    ) -> (ReviewOutcome, Vec<RunEvent>, std::sync::Arc<ScriptedDriver>, tempfile::TempDir) {
        let driver = ScriptedDriver::new(responses);
        let (tx, mut rx) = mpsc::channel(256);
        let stop = RunStop::default();
        let svc = LlmService::new(driver.clone(), 2, stop.token().clone(), tx.clone());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ep01.ass"), src).unwrap();
        let pair = LanguagePair::from_codes("zh", "en").unwrap();
        let prompts =
            crate::prompts::TranslationPrompts::defaults(&pair, crate::config::projects::Tone::Standard);
        let issue = |line_id: u32, issue_type: &str| VerifyIssue {
            line_id,
            source: String::new(),
            translation: String::new(),
            issue_type: issue_type.into(),
            description: "d".into(),
            severity: "high".into(),
            term: None,
            expected: None,
        };
        let outcome = retranslate_episode_lines(ReviewJob {
            job: FileJob {
                input: dir.path().join("ep01.ass"),
                file_name: "ep01.ass".into(),
                svc: &svc,
                verify_svc: &svc,
                glossary: &Glossary::default(),
                pair,
                prompts: &prompts,
                batch_limit: Some(100),
                stop,
                tx,
                seed: Some(Seed {
                    source_hash: hash,
                    translations: stored.iter().map(|(id, t)| (*id, t.to_string())).collect(),
                }),
            },
            line_ids: ids,
            output_name: "ep01.warning.eng.ass".into(),
            issues: vec![issue(2, "name"), issue(4, "untranslated"), issue(1, "meaning")],
            needs_look: true,
        })
        .await;
        drop(svc);
        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        (outcome, events, driver, dir)
    }

    #[tokio::test]
    async fn review_retranslates_only_the_chosen_lines_and_clears_their_issues() {
        let src = ass_source(&["{\\an8}你好", "叶长生不会走的", "走吧", "你算什么东西"]);
        let stored = [(1, "{\\an8}Hello"), (2, "Ye Changshen won't leave."), (3, "Let's go.")];
        let (outcome, events, driver, dir) = review(
            &src,
            &stored,
            fingerprint(&src),
            vec![2, 4],
            vec![Ok(ok_batch(&[(2, "Ye Changsheng won't leave.")])), Ok(ok_batch(&[(4, "Who do you think you are?")]))],
        )
        .await;
        assert!(outcome.failed.is_none());
        assert_eq!(outcome.retranslated, [2, 4]);
        // Only line 1's issue is left.
        assert_eq!(outcome.result.issues.iter().map(|i| i.line_id).collect::<Vec<_>>(), [1]);
        assert_eq!(driver.call_count(), 2);
        // Line 4's request carries the lines before it as context.
        assert!(driver.last_request().unwrap().user.contains("Let's go."));
        let written = std::fs::read_to_string(dir.path().join("ep01.warning.eng.ass")).unwrap();
        assert!(written.contains("{\\an8}Hello"), "untouched lines keep their tags");
        assert!(written.contains("Ye Changsheng won't leave."));
        assert!(written.contains("Who do you think you are?"));
        assert!(!written.contains("Changshen won't"));
        // Ends where it started: needs a look, with the remaining issues.
        assert!(events.iter().any(|e| matches!(e, RunEvent::FileDone { issues, .. } if issues.len() == 1)));
        assert!(matches!(events.last(), Some(RunEvent::State { state: FileStateKind::Warning, .. })));
        assert!(events.iter().any(|e| matches!(
            e,
            RunEvent::State { state: FileStateKind::Retranslating, detail: Some(d), .. } if d == "2 lines"
        )));
    }

    #[tokio::test]
    async fn review_refuses_a_changed_source() {
        let src = ass_source(&["你好"]);
        let (outcome, events, driver, dir) =
            review(&src, &[(1, "Hello")], fingerprint("old"), vec![1], vec![]).await;
        assert!(outcome.failed.unwrap().contains("changed since it was translated"));
        assert_eq!(driver.call_count(), 0);
        assert!(!dir.path().join("ep01.warning.eng.ass").exists());
        assert!(events.iter().any(|e| matches!(e, RunEvent::Error { .. })));
    }
}
