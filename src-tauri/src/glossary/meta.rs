//! Sidecar metadata for a folder's glossary, plus the IPC shapes around it.
//!
//! `glossary.json` stays plain and hand-editable; everything the app knows
//! *about* the terms lives next to it in `<folder>/.polygluttony/`:
//! - `glossary-meta.json` — where each term came from, which terms a scan
//!   added that nobody has looked at yet, the last bulk change, the files
//!   already scanned, and the show name for Look up names online.
//! - `glossary-history/` — snapshots taken before every bulk change, for
//!   "Restore an earlier version…".

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::diff::GlossaryDiff;
use super::model::GlossaryDoc;

/// Who a term's current translation came from. A term missing from
/// `GlossaryMeta::origins` counts as `Ai`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum TermOrigin {
    /// Found or changed by a scan, Tidy up or Look up.
    Ai,
    /// Typed, edited, imported or accepted by the user. AI steps only ever
    /// *propose* changes to these, unticked.
    Yours,
}

/// A bulk change to the glossary, and the reason a snapshot was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum GlossaryChangeKind {
    /// Add new episodes (append build).
    Scan,
    /// Rebuild from scratch.
    Rebuild,
    TidyUp,
    LookUp,
    Import,
    Restore,
    /// Replace in translations…
    Replace,
    /// A multi-row edit: move, delete.
    BulkEdit,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct GlossaryChange {
    pub kind: GlossaryChangeKind,
    /// Epoch ms.
    #[ts(type = "number")]
    pub at_ms: i64,
    pub diff: GlossaryDiff,
}

/// Everything the app knows about a folder's glossary beyond the terms.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct GlossaryMeta {
    /// Category key → source term → origin. Missing means `ai`.
    #[serde(default)]
    pub origins: BTreeMap<String, BTreeMap<String, TermOrigin>>,
    /// Category key → source terms a scan added that the user hasn't edited
    /// or reviewed yet (the NEW badge).
    #[serde(default)]
    pub unreviewed: BTreeMap<String, Vec<String>>,
    /// The last bulk change, for "View changes" and the Show filter.
    #[serde(default)]
    pub last_change: Option<GlossaryChange>,
    /// Source file names already scanned into this glossary.
    #[serde(default)]
    pub scanned_files: Vec<String>,
    /// Show name for Look up names online, remembered per folder.
    #[serde(default)]
    pub show_name: Option<String>,
}

/// What `load_glossary` found.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "snake_case")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum GlossaryLoad {
    /// No glossary.json in the folder.
    Missing,
    /// glossary.json exists but isn't valid. The app must not offer to build
    /// over it without saying so.
    Unreadable { message: String, line: Option<u32> },
    Ok { doc: GlossaryDoc, meta: GlossaryMeta },
}

/// One term, by category key and source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct TermRef {
    /// Category key, e.g. "characters".
    pub category: String,
    pub source: String,
}

/// Which terms Tidy up or Look up names online works on.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum GlossaryScope {
    All,
    /// One category, by key ("skills").
    Category { category: String },
    Terms { terms: Vec<TermRef> },
}

/// One entry of the glossary's version history.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct GlossarySnapshot {
    pub id: String,
    /// Epoch ms the snapshot was taken.
    #[ts(type = "number")]
    pub at_ms: i64,
    /// The change that followed it ("before Tidy up").
    pub reason: GlossaryChangeKind,
    pub count: u32,
}
