//! Stored results: each episode's last outcome, issues and translations,
//! kept in the app's data folder (keyed by subtitle folder) so the episode
//! list, Review and Resume survive a restart. Nothing is written into the
//! user's subtitle folder except the translated files themselves.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::events::VerifyIssue;

/// How an episode's last run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum EpisodeOutcome {
    Done,
    NeedsLook,
    Failed,
    /// Stopped part-way; Resume carries on from the stored lines.
    Stopped,
}

/// One episode's last result.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct EpisodeRecord {
    /// Source file name, relative to the folder.
    pub file: String,
    pub outcome: EpisodeOutcome,
    /// Issues still open (dismissed and fixed ones are gone).
    pub issues: Vec<VerifyIssue>,
    /// Output file name next to the source, when one was written.
    pub output_name: Option<String>,
    /// Epoch ms.
    #[ts(type = "number")]
    pub finished_ms: i64,
    pub total_lines: u32,
    pub translated_lines: u32,
    /// Why it failed or stopped, as one plain sentence.
    pub reason: Option<String>,
}

/// Every stored episode result for one folder, by file name.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct FolderResults {
    pub episodes: BTreeMap<String, EpisodeRecord>,
}

/// Lines of one finished episode whose source contains a glossary term.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct TermLines {
    pub file: String,
    pub line_ids: Vec<u32>,
}
