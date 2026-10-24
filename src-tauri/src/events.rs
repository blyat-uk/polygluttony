//! Events emitted from the Rust core to the webview during long-running
//! pipeline operations. The frontend subscribes via `onBackendEvent` (see
//! `src/lib/ipc.ts`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Event channel names. Keep in sync with the frontend listeners.
pub mod names {
    // The Translate page consumes TRANSLATION_EVENT (the tagged-union channel);
    // TRANSLATION_PROGRESS is retained here in case external tooling needs it.
    #[allow(dead_code)]
    pub const TRANSLATION_PROGRESS: &str = "translation://progress";
    // Superseded by GLOSSARY_EVENT (the Glossary view uses the tagged-union
    // channel instead); kept here in case external tooling needs it.
    #[allow(dead_code)]
    pub const GLOSSARY_PROGRESS: &str = "glossary://progress";
    #[allow(dead_code)]
    pub const LOG: &str = "core://log";
}

/// Generic progress payload for a single unit of work (e.g. a file).
// Legacy placeholder from the initial event sketch. Not currently emitted by
// any pipeline (Glossary uses `GLOSSARY_EVENT`; Translate uses `TRANSLATION_EVENT`;
// verification is embedded in the translation pipeline and reported via
// `RunEvent::State { state: FileStateKind::Verifying, .. }` on `TRANSLATION_EVENT`).
// Retained in case external tooling or a future step wants a generic per-file
// progress payload.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct ProgressEvent {
    /// Identifier of the unit of work, typically the file name.
    pub id: String,
    /// Completed steps so far.
    pub completed: u32,
    /// Total steps, when known.
    pub total: Option<u32>,
    /// Optional human-readable status message.
    pub message: Option<String>,
}

/// Channel for all step-3 run events.
pub const TRANSLATION_EVENT: &str = "translation://event";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum FileStateKind {
    Pending,
    Translating,
    Retranslating,
    Cleanup,
    Verifying,
    Done,
    Warning,
    Failed,
    /// The run stopped (by the user or an error) before this file finished.
    /// Finished batches are kept, so Resume carries on from here.
    Stopped,
}

/// What kind of failure an error was, so the UI can say what to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum ErrorKind {
    /// The provider rejected the API key (401/403).
    Auth,
    /// Out of credit or over quota.
    Billing,
    /// Rate-limited and the retries ran out.
    RateLimit,
    /// The provider doesn't know the model.
    Model,
    /// Couldn't reach the provider at all.
    Network,
    /// The provider's servers failed (5xx).
    Server,
    Other,
}

/// Why a run ended early. Sent once, before `RunFinished`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct StopInfo {
    /// True when the user pressed Stop.
    pub by_user: bool,
    /// One plain sentence for the banner, e.g. "Anthropic rejected the API
    /// key used for Translate." Empty when `by_user`.
    pub message: String,
    pub kind: Option<ErrorKind>,
    /// The step whose model failed, when known.
    pub step: Option<crate::config::Step>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum LogPhase {
    Parse,
    Batch,
    Cleanup,
    Verify,
    Llm,
    Error,
    Retranslate,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct VerifyIssue {
    pub line_id: u32,
    pub source: String,
    pub translation: String,
    pub issue_type: String,
    pub description: String,
    pub severity: String,
    /// For `name` issues: the glossary source term the line should use.
    #[serde(default)]
    pub term: Option<String>,
    /// For `name` issues: the translation the glossary expects.
    #[serde(default)]
    pub expected: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct FileResult {
    /// File *name* relative to the run folder, never an absolute path.
    pub file: String,
    pub success: bool,
    pub total_lines: u32,
    pub translated_lines: u32,
    pub has_warnings: bool,
    pub issues: Vec<VerifyIssue>,
    pub output_path: Option<String>,
}

/// Everything the UI hears during a run, on `TRANSLATION_EVENT`.
///
/// The `#[ts(skip)]` variants are internal: the pipeline sends them down the
/// same channel so they stay ordered with the rest, the run forwarder folds
/// them into the live board (`translation::live`) and never emits them to the
/// webview. Episode windows read the board through `episode_snapshot`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum RunEvent {
    /// A run began: always its first event, for translation runs and for
    /// review runs (retranslating lines or a glossary term's lines) alike.
    /// `folder` as the run was given it; `files` in run order.
    RunStarted { folder: String, files: Vec<String>, resume: bool },
    State { file: String, state: FileStateKind, detail: Option<String> },
    Progress {
        file: String,
        translated: u32,
        total: u32,
        batch: u32,
        total_batches: u32,
        retries: u32,
        /// Why the last retry happened, in plain words ("a reply came back incomplete").
        retry_reason: Option<String>,
    },
    Log { file: Option<String>, level: LogLevel, phase: LogPhase, message: String },
    FileDone { file: String, has_warnings: bool, issues: Vec<VerifyIssue> },
    Error { file: String, message: String, error_kind: ErrorKind },
    /// The run is ending early; sent once, before `RunFinished`.
    Stopped { info: StopInfo },
    /// A provider asked us to slow down; requests wait `seconds` first.
    Throttled { seconds: u32, provider: String },
    RunFinished { results: Vec<FileResult> },
    /// The episode's dialogue lines, once parsed.
    #[ts(skip)]
    EpisodeLines { file: String, lines: Vec<EpisodeSource> },
    /// These line ids just went to the model.
    #[ts(skip)]
    Sending { file: String, ids: Vec<u32>, activity: EpisodeActivity },
    /// Translations merged into the episode by any stage, tags stripped.
    #[ts(skip)]
    Landed { file: String, lines: Vec<(u32, String)> },
    /// The request announced by the last `Sending` is over, whatever came back.
    #[ts(skip)]
    Idle { file: String },
    /// The announced request actually went out to the provider (it may wait
    /// for a free slot or a rate-limit pause first). Sent by the LLM service
    /// once per attempt.
    #[ts(skip)]
    RequestSent { file: String },
    /// The announced request is waiting again before a retry.
    #[ts(skip)]
    RequestWaiting { file: String },
}

impl RunEvent {
    /// True for the live-board variants the forwarder keeps from the webview.
    /// No wildcard arm: a new variant has to decide which side it's on.
    pub fn is_internal(&self) -> bool {
        match self {
            RunEvent::EpisodeLines { .. }
            | RunEvent::Sending { .. }
            | RunEvent::Landed { .. }
            | RunEvent::Idle { .. }
            | RunEvent::RequestSent { .. }
            | RunEvent::RequestWaiting { .. } => true,
            RunEvent::RunStarted { .. }
            | RunEvent::State { .. }
            | RunEvent::Progress { .. }
            | RunEvent::Log { .. }
            | RunEvent::FileDone { .. }
            | RunEvent::Error { .. }
            | RunEvent::Stopped { .. }
            | RunEvent::Throttled { .. }
            | RunEvent::RunFinished { .. } => false,
        }
    }
}

/// Channel that tells episode windows to fetch a fresh snapshot.
pub const EPISODE_CHANGED_EVENT: &str = "episode://changed";

/// Payload of `EPISODE_CHANGED_EVENT`. `file: None` means every episode: a run
/// started or finished.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct EpisodeChanged {
    pub file: Option<String>,
}

/// One dialogue line as the live board first learns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct EpisodeSource {
    /// 1-based, the pipeline's line id.
    pub id: u32,
    /// Centiseconds.
    pub start_cs: u32,
    pub end_cs: u32,
    /// ASS override tags stripped; `\N` and `\n` kept for the view to break on.
    pub text: String,
}

/// What the model is doing for an episode right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum EpisodeActivity {
    /// A first-pass (or full-redo) batch, 1-based.
    Batch { batch: u32, total_batches: u32 },
    /// The cleanup pass re-sending lines that still carry source text.
    Cleanup,
    /// Verify asked for these lines again.
    Retranslate { attempt: u32, max_attempts: u32 },
}

/// What a line's Translation cell shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum LineStatus {
    /// Not sent yet.
    Waiting,
    /// Sent, no translation yet.
    Sending,
    Translated,
    /// Sent again (cleanup or verify) while an older translation stands.
    Resending,
    /// The episode finished without translating it.
    Untranslated,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct EpisodeLine {
    pub id: u32,
    pub start_cs: u32,
    pub end_cs: u32,
    pub source: String,
    /// Latest translation, tags stripped.
    pub translation: Option<String>,
    pub status: LineStatus,
}

/// One episode as its live window shows it.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct EpisodeSnapshot {
    pub file: String,
    pub state: FileStateKind,
    pub detail: Option<String>,
    pub error: Option<String>,
    /// False once the run has finished or been cancelled.
    pub run_active: bool,
    pub activity: Option<EpisodeActivity>,
    /// Epoch ms the current activity's request went out.
    #[ts(type = "number | null")]
    pub activity_since_ms: Option<u64>,
    /// Lines with any translation.
    pub translated: u32,
    pub total: u32,
    pub lines: Vec<EpisodeLine>,
    /// Final verify issues; empty until the episode finishes.
    pub issues: Vec<VerifyIssue>,
    /// Set while this episode's next request is held back because a
    /// provider asked us to slow down.
    #[serde(default)]
    pub throttle: Option<EpisodeThrottle>,
}

/// A rate-limit wait an episode is sitting out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct EpisodeThrottle {
    /// Who asked us to wait ("Anthropic", or "The server" for a custom URL).
    pub provider: String,
    /// Epoch ms the wait ends.
    #[ts(type = "number")]
    pub until_ms: u64,
}

/// Channel that says a folder's stored results changed (an episode finished,
/// a run stopped, or a review action saved). Payload `ResultsChanged`; refetch
/// `get_folder_results`.
pub const RESULTS_CHANGED_EVENT: &str = "results://changed";

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct ResultsChanged {
    /// The folder as the run or command was given it.
    pub folder: String,
    pub file: String,
}

/// Channel that asks an open episode window to scroll to and select a line.
pub const EPISODE_GO_TO_LINE_EVENT: &str = "episode://go-to-line";

/// Payload of `EPISODE_GO_TO_LINE_EVENT` (typed in `src/lib/ipc.ts`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodeGoToLine {
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum EpisodeView {
    /// No run this session, or the current run doesn't include this episode.
    NotInRun,
    /// In the run, but its source file couldn't be read.
    Unreadable { message: String },
    Live(EpisodeSnapshot),
    /// A finished episode read back from the stored results, outside any run.
    Stored(EpisodeSnapshot),
}

/// Channel for all step-4 glossary events.
pub const GLOSSARY_EVENT: &str = "glossary://event";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum GlossaryPhase {
    Loading,
    Reference,
    Extracting,
    Normalizing,
    Personalizing,
    Saving,
}

/// Terminal payload of a glossary build. `aborted` = mid-build fatal (auth)
/// — partial results were still merged and saved; `cancelled` = user cancel,
/// same partial-save semantics. Partial > none (hard requirement).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct GlossaryBuildSummary {
    pub world_type: String,
    pub files_processed: u32,
    pub batches_processed: u32,
    pub batches_total: u32,
    pub terms_extracted: u32,
    pub terms_final: u32,
    pub normalized: bool,
    pub personalized: bool,
    pub aborted: bool,
    pub cancelled: bool,
    pub errors: Vec<String>,
    /// Diff vs the glossary as it stood before the build.
    pub diff: crate::glossary::diff::GlossaryDiff,
}

/// One term surfaced by a single extraction batch — fuel for the live build
/// console (terms streaming into their category lanes as batches land).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct TermHit {
    pub category: String,
    pub source: String,
    pub target: String,
}

/// Everything the UI hears on `GLOSSARY_EVENT`. `Error` is terminal-without-
/// result (e.g. final save IO failure); pre-flight failures surface as command
/// errors, not events. `FileChanged` comes from the O15 watcher. `Terms` streams
/// each batch's newly-found terms during extraction (some batches yield none).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum GlossaryEvent {
    Phase { phase: GlossaryPhase, detail: Option<String> },
    Progress { done: u32, total: u32 },
    Terms { batch: u32, hits: Vec<TermHit> },
    Log { level: LogLevel, message: String },
    Done { summary: GlossaryBuildSummary },
    Error { message: String },
    FileChanged,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glossary_event_serializes_with_kind_tag() {
        let ev = GlossaryEvent::Phase { phase: GlossaryPhase::Extracting, detail: None };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["kind"], "phase");
        assert_eq!(v["phase"], "extracting");

        let done = GlossaryEvent::Done {
            summary: GlossaryBuildSummary {
                world_type: "xianxia".into(),
                files_processed: 1,
                batches_processed: 2,
                batches_total: 2,
                terms_extracted: 10,
                terms_final: 8,
                normalized: true,
                personalized: false,
                aborted: false,
                cancelled: false,
                errors: vec![],
                diff: crate::glossary::diff::GlossaryDiff::compute(
                    None,
                    &crate::glossary::model::Glossary::new("xianxia"),
                ),
            },
        };
        let v = serde_json::to_value(&done).unwrap();
        assert_eq!(v["kind"], "done");
        assert_eq!(v["summary"]["terms_final"], 8);

        let v = serde_json::to_value(&GlossaryEvent::FileChanged).unwrap();
        assert_eq!(v, serde_json::json!({"kind": "file_changed"}));
    }
}
