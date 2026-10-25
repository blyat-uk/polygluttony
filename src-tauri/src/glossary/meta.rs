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
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::diff::GlossaryDiff;
use super::io::write_atomic;
use super::model::{Glossary, GlossaryDoc, CATEGORIES};
use crate::error::AppResult;

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
    /// The glossary also has terms from episodes it has no record of: it was
    /// made before PG kept track, or imported. `None` in a record from before
    /// this field, where it holds when no episodes are on record.
    #[serde(default)]
    pub untracked: Option<bool>,
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

// ---- the sidecar on disk --------------------------------------------------------

/// `<folder>/.polygluttony/` — the app's own files for a work folder.
pub fn app_dir(folder: &Path) -> PathBuf {
    folder.join(".polygluttony")
}

fn meta_path(folder: &Path) -> PathBuf {
    app_dir(folder).join("glossary-meta.json")
}

/// The sidecar as stored, or `None` when there is none or it can't be read.
pub(crate) fn read_meta(folder: &Path) -> Option<GlossaryMeta> {
    let text = std::fs::read_to_string(meta_path(folder)).ok()?;
    serde_json::from_str(&text).ok()
}

/// The folder's glossary metadata. A missing or corrupt sidecar gives the
/// default: metadata is never a reason for a glossary to fail to load.
pub fn load_meta(folder: &Path) -> GlossaryMeta {
    read_meta(folder).unwrap_or_default()
}

/// Atomic write (temp + rename), creating `.polygluttony/` when needed.
pub fn save_meta(folder: &Path, meta: &GlossaryMeta) -> AppResult<()> {
    std::fs::create_dir_all(app_dir(folder))?;
    write_atomic(&meta_path(folder), &serde_json::to_string_pretty(meta)?)
}

/// Epoch milliseconds, for `GlossaryChange::at_ms` and snapshots.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---- bookkeeping after bulk changes -----------------------------------------------

impl GlossaryMeta {
    /// Forget origins and NEW marks of terms that aren't in `g` (deleted,
    /// moved by hand, or under a category key the app doesn't know).
    pub fn retain_terms(&mut self, g: &Glossary) {
        let has = |c: &str, s: &str| CATEGORIES.contains(&c) && g.category(c).contains_key(s);
        self.origins.retain(|c, terms| {
            terms.retain(|s, _| has(c, s));
            !terms.is_empty()
        });
        self.unreviewed.retain(|c, terms| {
            terms.retain(|s| has(c, s));
            !terms.is_empty()
        });
    }

    /// Add new episodes finished: every term of `after` that `before` didn't
    /// have is marked NEW (its origin stays `ai`), `scanned` joins the scanned
    /// files, and `change` becomes the last change. Existing terms keep their
    /// origin and NEW mark.
    pub fn record_scan(
        &mut self,
        before: Option<&Glossary>,
        after: &Glossary,
        scanned: &[String],
        change: GlossaryChange,
    ) {
        self.retain_terms(after);
        for c in CATEGORIES {
            for source in after.category(c).keys() {
                if before.is_some_and(|b| b.has_key(source)) {
                    continue;
                }
                if let Some(origins) = self.origins.get_mut(c) {
                    origins.remove(source);
                }
                self.unreviewed.entry(c.to_string()).or_default().push(source.clone());
            }
        }
        self.origins.retain(|_, terms| !terms.is_empty());
        for terms in self.unreviewed.values_mut() {
            terms.sort();
            terms.dedup();
        }
        // Settled before the first scan goes on record: terms already there with
        // no episodes on record came from episodes nobody wrote down.
        if self.untracked.is_none() {
            self.untracked =
                Some(before.is_some_and(|b| b.count() > 0) && self.scanned_files.is_empty());
        }
        for f in scanned {
            if !self.scanned_files.contains(f) {
                self.scanned_files.push(f.clone());
            }
        }
        self.last_change = Some(change);
    }

    /// Rebuild from scratch finished: every term is the AI's and NEW, and the
    /// scanned files are exactly `scanned`.
    pub fn record_rebuild(&mut self, after: &Glossary, scanned: &[String], change: GlossaryChange) {
        self.origins.clear();
        self.unreviewed = all_sources(after);
        self.scanned_files = scanned.to_vec();
        self.untracked = Some(false);
        self.last_change = Some(change);
    }

    /// An imported glossary is the user's own: every term is `yours` and
    /// nothing is marked NEW.
    pub fn record_import(&mut self, after: &Glossary, change: GlossaryChange) {
        self.origins = all_sources(after)
            .into_iter()
            .map(|(c, sources)| (c, sources.into_iter().map(|s| (s, TermOrigin::Yours)).collect()))
            .collect();
        self.unreviewed.clear();
        self.untracked = Some(true);
        self.last_change = Some(change);
    }
}

/// Category key → sorted sources, non-empty categories only.
fn all_sources(g: &Glossary) -> BTreeMap<String, Vec<String>> {
    CATEGORIES
        .iter()
        .filter(|c| !g.category(c).is_empty())
        .map(|c| (c.to_string(), g.category(c).keys().cloned().collect()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(kind: GlossaryChangeKind, before: Option<&Glossary>, after: &Glossary) -> GlossaryChange {
        GlossaryChange { kind, at_ms: 42, diff: GlossaryDiff::compute(before, after) }
    }

    fn glossary(terms: &[(&str, &str, &str)]) -> Glossary {
        let mut g = Glossary::new("xianxia");
        for (c, s, t) in terms {
            g.category_mut(c).insert(s.to_string(), t.to_string());
        }
        g
    }

    #[test]
    fn missing_meta_loads_as_default() {
        let dir = tempfile::tempdir().unwrap();
        let m = load_meta(dir.path());
        assert!(m.origins.is_empty() && m.unreviewed.is_empty());
        assert!(m.last_change.is_none() && m.show_name.is_none());
        assert!(read_meta(dir.path()).is_none());
    }

    #[test]
    fn corrupt_meta_loads_as_default() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(app_dir(dir.path())).unwrap();
        for bad in ["not json", r#"{"origins":{"characters":{"林动":"robot"}}}"#, "[1,2]"] {
            std::fs::write(meta_path(dir.path()), bad).unwrap();
            let m = load_meta(dir.path());
            assert!(m.origins.is_empty(), "{bad} must give the default");
            assert!(read_meta(dir.path()).is_none());
        }
    }

    #[test]
    fn meta_round_trips_through_the_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let after = glossary(&[("characters", "林动", "Lin Dong")]);
        let mut m = GlossaryMeta {
            scanned_files: vec!["e1.ass".into()],
            show_name: Some("Martial Universe".into()),
            ..Default::default()
        };
        m.origins.entry("characters".into()).or_default().insert("林动".into(), TermOrigin::Yours);
        m.unreviewed.insert("items".into(), vec!["祖符".into()]);
        m.last_change = Some(change(GlossaryChangeKind::TidyUp, None, &after));
        save_meta(dir.path(), &m).unwrap();

        // Stored where the contract says, with no temp file left behind.
        let stored = dir.path().join(".polygluttony").join("glossary-meta.json");
        assert!(stored.is_file());
        assert!(!dir.path().join(".polygluttony").join(".glossary-meta.json.tmp").exists());

        let back = load_meta(dir.path());
        assert_eq!(back.origins["characters"]["林动"], TermOrigin::Yours);
        assert_eq!(back.unreviewed["items"], vec!["祖符".to_string()]);
        assert_eq!(back.scanned_files, vec!["e1.ass".to_string()]);
        assert_eq!(back.show_name.as_deref(), Some("Martial Universe"));
        let c = back.last_change.unwrap();
        assert_eq!(c.kind, GlossaryChangeKind::TidyUp);
        assert_eq!(c.at_ms, 42);
        assert_eq!(c.diff.total_added, 1);
    }

    /// Fields default one by one, so an older or hand-trimmed sidecar loads.
    #[test]
    fn partial_meta_fills_in_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(app_dir(dir.path())).unwrap();
        std::fs::write(meta_path(dir.path()), r#"{"show_name":"Veil of Shadow"}"#).unwrap();
        let m = load_meta(dir.path());
        assert_eq!(m.show_name.as_deref(), Some("Veil of Shadow"));
        assert!(m.scanned_files.is_empty());
    }

    #[test]
    fn retain_terms_drops_entries_for_missing_terms() {
        let g = glossary(&[("characters", "林动", "Lin Dong")]);
        let mut m = GlossaryMeta::default();
        let o = m.origins.entry("characters".into()).or_default();
        o.insert("林动".into(), TermOrigin::Yours);
        o.insert("gone".into(), TermOrigin::Yours);
        m.origins.entry("bogus".into()).or_default().insert("x".into(), TermOrigin::Ai);
        m.unreviewed.insert("characters".into(), vec!["林动".into(), "gone".into()]);
        m.unreviewed.insert("items".into(), vec!["林动".into()]); // wrong category
        m.retain_terms(&g);
        assert_eq!(m.origins.len(), 1);
        assert_eq!(m.origins["characters"].len(), 1);
        assert_eq!(m.unreviewed.len(), 1);
        assert_eq!(m.unreviewed["characters"], vec!["林动".to_string()]);
    }

    #[test]
    fn record_scan_marks_only_new_terms_and_keeps_the_rest() {
        let before = glossary(&[("characters", "林动", "Lin Dong"), ("items", "祖符", "Symbol")]);
        let after = glossary(&[
            ("characters", "林动", "Lin Dong"),
            ("characters", "应欢欢", "Ying Huanhuan"),
            ("items", "祖符", "Symbol"),
            ("locations", "青阳镇", "Qingyang Town"),
        ]);
        let mut m = GlossaryMeta {
            show_name: Some("Martial Universe".into()),
            scanned_files: vec!["e1.ass".into()],
            ..Default::default()
        };
        m.origins.entry("characters".into()).or_default().insert("林动".into(), TermOrigin::Yours);
        m.unreviewed.insert("items".into(), vec!["祖符".into()]); // still NEW from an earlier scan

        let c = change(GlossaryChangeKind::Scan, Some(&before), &after);
        m.record_scan(Some(&before), &after, &["e1.ass".into(), "e2.ass".into()], c);

        assert_eq!(m.origins["characters"]["林动"], TermOrigin::Yours, "existing origin kept");
        assert_eq!(m.origins.len(), 1, "new terms get no origin entry (= ai)");
        assert_eq!(m.unreviewed["characters"], vec!["应欢欢".to_string()]);
        assert_eq!(m.unreviewed["locations"], vec!["青阳镇".to_string()]);
        assert_eq!(m.unreviewed["items"], vec!["祖符".to_string()], "earlier NEW marks stay");
        assert_eq!(m.scanned_files, vec!["e1.ass".to_string(), "e2.ass".to_string()]);
        assert_eq!(m.untracked, Some(false), "every term came from episodes on record");
        assert_eq!(m.show_name.as_deref(), Some("Martial Universe"));
        let lc = m.last_change.unwrap();
        assert_eq!(lc.kind, GlossaryChangeKind::Scan);
        assert_eq!(lc.diff.total_added, 2);
    }

    #[test]
    fn record_scan_on_a_first_build_marks_everything_new() {
        let after = glossary(&[("characters", "林动", "Lin Dong")]);
        let mut m = GlossaryMeta::default();
        m.record_scan(None, &after, &["e1.ass".into()], change(GlossaryChangeKind::Scan, None, &after));
        assert_eq!(m.unreviewed["characters"], vec!["林动".to_string()]);
        assert_eq!(m.scanned_files, vec!["e1.ass".to_string()]);
        assert_eq!(m.untracked, Some(false));
    }

    /// A glossary from before PG kept track has terms but no episodes on
    /// record: its first scan says so, and it stays said after more scans.
    #[test]
    fn record_scan_on_an_untracked_glossary_remembers_it() {
        let before = glossary(&[("characters", "林动", "Lin Dong")]);
        let after = glossary(&[("characters", "林动", "Lin Dong"), ("items", "祖符", "Symbol")]);
        let mut m = GlossaryMeta::default();
        let c = change(GlossaryChangeKind::Scan, Some(&before), &after);
        m.record_scan(Some(&before), &after, &["e7.ass".into()], c);
        assert_eq!(m.untracked, Some(true));
        assert_eq!(m.scanned_files, vec!["e7.ass".to_string()]);
        let c = change(GlossaryChangeKind::Scan, Some(&after), &after);
        m.record_scan(Some(&after), &after, &["e8.ass".into()], c);
        assert_eq!(m.untracked, Some(true));
    }

    /// A term that was deleted and found again is the AI's again, whatever a
    /// stale origin entry says.
    #[test]
    fn record_scan_resets_a_stale_origin_of_a_new_term() {
        let before = Glossary::new("xianxia");
        let after = glossary(&[("characters", "林动", "Lin Dong")]);
        let mut m = GlossaryMeta::default();
        m.origins.entry("characters".into()).or_default().insert("林动".into(), TermOrigin::Yours);
        m.record_scan(Some(&before), &after, &[], change(GlossaryChangeKind::Scan, Some(&before), &after));
        assert!(m.origins.is_empty());
        assert!(m.scanned_files.is_empty());
    }

    #[test]
    fn record_rebuild_resets_origins_and_marks_all_new() {
        let after = glossary(&[("characters", "林动", "Lin Dong"), ("items", "祖符", "Symbol")]);
        let mut m = GlossaryMeta {
            show_name: Some("Martial Universe".into()),
            scanned_files: vec!["old.ass".into()],
            ..Default::default()
        };
        m.origins.entry("characters".into()).or_default().insert("林动".into(), TermOrigin::Yours);
        m.record_rebuild(&after, &["e1.ass".into()], change(GlossaryChangeKind::Rebuild, None, &after));
        assert!(m.origins.is_empty());
        assert_eq!(m.unreviewed["characters"], vec!["林动".to_string()]);
        assert_eq!(m.unreviewed["items"], vec!["祖符".to_string()]);
        assert_eq!(m.scanned_files, vec!["e1.ass".to_string()]);
        assert_eq!(m.untracked, Some(false));
        assert_eq!(m.show_name.as_deref(), Some("Martial Universe"));
        assert_eq!(m.last_change.unwrap().kind, GlossaryChangeKind::Rebuild);
    }

    #[test]
    fn record_import_marks_all_yours() {
        let after = glossary(&[("characters", "林动", "Lin Dong"), ("items", "祖符", "Symbol")]);
        let mut m = GlossaryMeta::default();
        m.unreviewed.insert("characters".into(), vec!["林动".into()]);
        m.record_import(&after, change(GlossaryChangeKind::Import, None, &after));
        assert_eq!(m.origins["characters"]["林动"], TermOrigin::Yours);
        assert_eq!(m.origins["items"]["祖符"], TermOrigin::Yours);
        assert!(m.unreviewed.is_empty());
        assert_eq!(m.untracked, Some(true));
        assert_eq!(m.last_change.unwrap().kind, GlossaryChangeKind::Import);
    }
}
