//! Version history for a folder's glossary ("Restore an earlier version…").
//!
//! Before every bulk change the current `glossary.json` (plus its sidecar meta,
//! when there is one) is copied into `<folder>/.polygluttony/glossary-history/`
//! as ONE JSON file that also records why and when. The newest [`KEEP`]
//! snapshots are kept.
//!
//! A snapshot's id is its file stem: the epoch ms it was taken, plus `-N` when
//! several land in the same millisecond. Ids are checked to be exactly that
//! shape before they touch the file system, so a restore can't be pointed
//! outside the history folder.
//!
//! The pre-history single undo slot, `glossary.prev.json`, is still offered
//! when a folder has one: as the extra entry [`LEGACY_PREV_ID`] (reason
//! `rebuild`, since Rebuild from scratch was what wrote it). Nothing writes
//! it any more and it never counts towards [`KEEP`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::diff::GlossaryDiff;
use super::io::{load_folder_glossary, write_atomic};
use super::meta::{
    app_dir, load_meta, now_ms, read_meta, save_meta, GlossaryChange, GlossaryChangeKind,
    GlossaryMeta, GlossarySnapshot,
};
use super::model::Glossary;
use crate::error::{AppError, AppResult};

/// How many snapshots a folder keeps.
pub const KEEP: usize = 10;
/// Id of the legacy `glossary.prev.json` entry.
pub const LEGACY_PREV_ID: &str = "prev";
const LEGACY_PREV_FILE: &str = "glossary.prev.json";

/// One history file.
#[derive(Debug, Serialize, Deserialize)]
struct SnapshotFile {
    reason: GlossaryChangeKind,
    at_ms: i64,
    /// `glossary.json` as it was. Null when it wasn't valid JSON; `raw` holds
    /// the text then, so even a broken glossary can be brought back as it was.
    doc: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    raw: Option<String>,
    /// The sidecar as it was; `None` when there was none (or it was corrupt).
    #[serde(default)]
    meta: Option<GlossaryMeta>,
}

impl SnapshotFile {
    /// The text to write back to `glossary.json`.
    fn glossary_text(&self) -> String {
        match &self.raw {
            Some(raw) => raw.clone(),
            None => serde_json::to_string_pretty(&self.doc).expect("a Value always serializes"),
        }
    }
}

fn history_dir(folder: &Path) -> PathBuf {
    app_dir(folder).join("glossary-history")
}

/// `1761300000000` → (1761300000000, 0); `1761300000000-2` → (…, 2). Digits
/// only: anything else (a path, a dot, a sign) is not an id.
fn parse_id(id: &str) -> Option<(i64, u32)> {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let (ms, seq) = id.split_once('-').unwrap_or((id, "0"));
    if !digits(ms) || !digits(seq) {
        return None;
    }
    Some((ms.parse().ok()?, seq.parse().ok()?))
}

/// Snapshot ids on disk, newest first.
fn ids_newest_first(folder: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(history_dir(folder)) else {
        return Vec::new();
    };
    let mut ids: Vec<((i64, u32), String)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            let id = name.strip_suffix(".json")?;
            Some((parse_id(id)?, id.to_string()))
        })
        .collect();
    ids.sort_by(|a, b| b.0.cmp(&a.0));
    ids.into_iter().map(|(_, id)| id).collect()
}

fn read_snapshot(folder: &Path, id: &str) -> Option<SnapshotFile> {
    parse_id(id)?;
    let text = std::fs::read_to_string(history_dir(folder).join(format!("{id}.json"))).ok()?;
    serde_json::from_str(&text).ok()
}

/// Term count of a stored `glossary.json` value.
fn doc_count(doc: &serde_json::Value) -> u32 {
    if doc.is_object() {
        Glossary::from_terms_value(doc, "").count() as u32
    } else {
        0
    }
}

/// Save the current glossary (and meta) to the history, labelled with the
/// change about to happen. `Ok(None)` = there's no glossary.json to save.
pub fn snapshot(folder: &Path, reason: GlossaryChangeKind) -> AppResult<Option<String>> {
    snapshot_at(folder, reason, now_ms())
}

pub(crate) fn snapshot_at(
    folder: &Path,
    reason: GlossaryChangeKind,
    at_ms: i64,
) -> AppResult<Option<String>> {
    let text = match std::fs::read_to_string(folder.join("glossary.json")) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let (doc, raw) = match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => (v, None),
        Err(_) => (serde_json::Value::Null, Some(text)),
    };
    let file = SnapshotFile { reason, at_ms, doc, raw, meta: read_meta(folder) };

    let dir = history_dir(folder);
    std::fs::create_dir_all(&dir)?;
    let at = at_ms.max(0);
    let mut id = at.to_string();
    let mut seq = 0u32;
    while dir.join(format!("{id}.json")).exists() {
        seq += 1;
        id = format!("{at}-{seq}");
    }
    write_atomic(&dir.join(format!("{id}.json")), &serde_json::to_string_pretty(&file)?)?;
    rotate(folder);
    Ok(Some(id))
}

/// Delete all but the newest [`KEEP`] snapshots. Best effort: a file that
/// won't go away costs a slot, never the snapshot that was just taken.
fn rotate(folder: &Path) {
    let dir = history_dir(folder);
    for id in ids_newest_first(folder).into_iter().skip(KEEP) {
        let _ = std::fs::remove_file(dir.join(format!("{id}.json")));
    }
}

/// The legacy `glossary.prev.json`, when it holds terms.
fn legacy_prev(folder: &Path) -> Option<GlossarySnapshot> {
    let path = folder.join(LEGACY_PREV_FILE);
    let text = std::fs::read_to_string(&path).ok()?;
    let count = Glossary::from_json(&text)?.count() as u32;
    if count == 0 {
        return None;
    }
    // An unreadable mtime only misplaces the entry in the list; 0 sorts it last.
    let at_ms = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Some(GlossarySnapshot {
        id: LEGACY_PREV_ID.into(),
        at_ms,
        reason: GlossaryChangeKind::Rebuild,
        count,
    })
}

/// Every restorable version, newest first. Unreadable snapshot files are left
/// out.
pub fn list(folder: &Path) -> Vec<GlossarySnapshot> {
    let mut out: Vec<GlossarySnapshot> = ids_newest_first(folder)
        .into_iter()
        .filter_map(|id| {
            let file = read_snapshot(folder, &id)?;
            Some(GlossarySnapshot {
                at_ms: file.at_ms,
                reason: file.reason,
                count: doc_count(&file.doc),
                id,
            })
        })
        .collect();
    out.extend(legacy_prev(folder));
    // Stable: snapshots taken in the same millisecond keep their id order.
    out.sort_by_key(|s| std::cmp::Reverse(s.at_ms));
    out
}

fn not_found() -> AppError {
    AppError::Other("That earlier version isn't there any more.".into())
}

/// Bring back version `id`: the current glossary is snapshotted first
/// (reason `restore`), so the restore can itself be undone.
///
/// The meta written is the snapshot's, except that
/// - `show_name` stays as it is now (it's a per-folder preference, not part
///   of a version); and
/// - `last_change` becomes this restore, so "View changes" shows what it did.
///
/// A version saved without meta (the legacy prev file, or a folder that had
/// none yet) restores with default origins and keeps the current scanned
/// files.
pub fn restore(folder: &Path, id: &str) -> AppResult<()> {
    // Read the target BEFORE the restore snapshot: that snapshot rotates the
    // history and could otherwise delete the oldest version mid-restore.
    let (text, stored_meta) = if id == LEGACY_PREV_ID {
        let text = std::fs::read_to_string(folder.join(LEGACY_PREV_FILE)).map_err(|_| not_found())?;
        (text, None)
    } else {
        let file = read_snapshot(folder, id).ok_or_else(not_found)?;
        (file.glossary_text(), file.meta)
    };
    let current = load_folder_glossary(folder);
    let current_meta = load_meta(folder);

    snapshot(folder, GlossaryChangeKind::Restore)?;
    write_atomic(&folder.join("glossary.json"), &text)?;

    let mut meta = stored_meta.unwrap_or_else(|| GlossaryMeta {
        scanned_files: current_meta.scanned_files.clone(),
        ..Default::default()
    });
    meta.show_name = current_meta.show_name.or(meta.show_name);
    if let Some(restored) = Glossary::from_json(&text) {
        meta.last_change = Some(GlossaryChange {
            kind: GlossaryChangeKind::Restore,
            at_ms: now_ms(),
            diff: GlossaryDiff::compute(current.as_ref(), &restored),
        });
    }
    save_meta(folder, &meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glossary::io::save_folder_glossary;
    use crate::glossary::meta::TermOrigin;

    fn write_glossary(folder: &Path, terms: &[(&str, &str)]) {
        let mut g = Glossary::new("xianxia");
        for (s, t) in terms {
            g.characters.insert(s.to_string(), t.to_string());
        }
        save_folder_glossary(folder, &g).unwrap();
    }

    fn current(folder: &Path) -> Glossary {
        load_folder_glossary(folder).unwrap()
    }

    #[test]
    fn ids_must_be_digits() {
        assert_eq!(parse_id("1761300000000"), Some((1761300000000, 0)));
        assert_eq!(parse_id("1761300000000-2"), Some((1761300000000, 2)));
        for bad in ["", "prev", "../x", "12-", "-3", "1+2", "12-a", "1.5", "12/34", "+12"] {
            assert_eq!(parse_id(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn no_glossary_means_no_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(snapshot(dir.path(), GlossaryChangeKind::TidyUp).unwrap(), None);
        assert!(list(dir.path()).is_empty());
        assert!(!history_dir(dir.path()).exists());
    }

    #[test]
    fn snapshot_records_doc_meta_reason_and_time() {
        let dir = tempfile::tempdir().unwrap();
        write_glossary(dir.path(), &[("林动", "Lin Dong"), ("应欢欢", "Ying Huanhuan")]);
        let mut m = GlossaryMeta { show_name: Some("Martial Universe".into()), ..Default::default() };
        m.origins.entry("characters".into()).or_default().insert("林动".into(), TermOrigin::Yours);
        save_meta(dir.path(), &m).unwrap();

        let id = snapshot_at(dir.path(), GlossaryChangeKind::TidyUp, 1_000).unwrap().unwrap();
        assert_eq!(id, "1000");
        let file = read_snapshot(dir.path(), &id).unwrap();
        assert_eq!(file.reason, GlossaryChangeKind::TidyUp);
        assert_eq!(file.at_ms, 1_000);
        assert_eq!(doc_count(&file.doc), 2);
        assert_eq!(file.meta.unwrap().origins["characters"]["林动"], TermOrigin::Yours);

        let entries = list(dir.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "1000");
        assert_eq!(entries[0].count, 2);
        assert_eq!(entries[0].reason, GlossaryChangeKind::TidyUp);
    }

    #[test]
    fn snapshot_without_meta_stores_none() {
        let dir = tempfile::tempdir().unwrap();
        write_glossary(dir.path(), &[("林动", "Lin Dong")]);
        let id = snapshot(dir.path(), GlossaryChangeKind::Scan).unwrap().unwrap();
        assert!(read_snapshot(dir.path(), &id).unwrap().meta.is_none());
    }

    #[test]
    fn same_millisecond_snapshots_get_distinct_ids_in_order() {
        let dir = tempfile::tempdir().unwrap();
        write_glossary(dir.path(), &[("a", "A")]);
        let a = snapshot_at(dir.path(), GlossaryChangeKind::Scan, 5).unwrap().unwrap();
        write_glossary(dir.path(), &[("a", "A"), ("b", "B")]);
        let b = snapshot_at(dir.path(), GlossaryChangeKind::TidyUp, 5).unwrap().unwrap();
        assert_eq!((a.as_str(), b.as_str()), ("5", "5-1"));
        let ids: Vec<String> = list(dir.path()).into_iter().map(|s| s.id).collect();
        assert_eq!(ids, vec!["5-1".to_string(), "5".to_string()], "newest first");
    }

    #[test]
    fn keeps_the_newest_ten_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..13i64 {
            write_glossary(dir.path(), &[("a", &format!("v{i}"))]);
            // Out-of-order times would expose a sort by file system order.
            snapshot_at(dir.path(), GlossaryChangeKind::BulkEdit, 1_000 + i * 10).unwrap();
        }
        let entries = list(dir.path());
        assert_eq!(entries.len(), KEEP);
        let times: Vec<i64> = entries.iter().map(|s| s.at_ms).collect();
        let expected: Vec<i64> = (3..13).rev().map(|i| 1_000 + i * 10).collect();
        assert_eq!(times, expected);
        // Rotation deleted the files, not just hid them.
        let files = std::fs::read_dir(history_dir(dir.path())).unwrap().count();
        assert_eq!(files, KEEP);
    }

    #[test]
    fn unreadable_snapshot_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        write_glossary(dir.path(), &[("a", "A")]);
        snapshot_at(dir.path(), GlossaryChangeKind::Scan, 10).unwrap();
        std::fs::write(history_dir(dir.path()).join("20.json"), "garbage").unwrap();
        std::fs::write(history_dir(dir.path()).join("notes.json"), "{}").unwrap();
        let ids: Vec<String> = list(dir.path()).into_iter().map(|s| s.id).collect();
        assert_eq!(ids, vec!["10".to_string()]);
        assert!(restore(dir.path(), "20").is_err());
    }

    #[test]
    fn restore_snapshots_the_current_version_first() {
        let dir = tempfile::tempdir().unwrap();
        write_glossary(dir.path(), &[("林动", "Lin Dong")]);
        save_meta(
            dir.path(),
            &GlossaryMeta { show_name: Some("Old name".into()), ..Default::default() },
        )
        .unwrap();
        let id = snapshot_at(dir.path(), GlossaryChangeKind::TidyUp, 1_000).unwrap().unwrap();

        // The change the snapshot guarded against.
        write_glossary(dir.path(), &[("林动", "Lin Tung"), ("应欢欢", "Ying Huanhuan")]);
        let mut now_meta = GlossaryMeta { show_name: Some("Martial Universe".into()), ..Default::default() };
        now_meta.unreviewed.insert("characters".into(), vec!["应欢欢".into()]);
        save_meta(dir.path(), &now_meta).unwrap();

        restore(dir.path(), &id).unwrap();

        let g = current(dir.path());
        assert_eq!(g.count(), 1);
        assert_eq!(g.characters["林动"], "Lin Dong");

        // The version we left is the newest snapshot, labelled `restore`.
        let entries = list(dir.path());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].reason, GlossaryChangeKind::Restore);
        assert_eq!(entries[0].count, 2);
        let undo = read_snapshot(dir.path(), &entries[0].id).unwrap();
        assert_eq!(undo.meta.unwrap().unreviewed["characters"], vec!["应欢欢".to_string()]);

        // Meta: the snapshot's, the current show name, and the restore as last change.
        let m = load_meta(dir.path());
        assert!(m.unreviewed.is_empty());
        assert_eq!(m.show_name.as_deref(), Some("Martial Universe"));
        let lc = m.last_change.unwrap();
        assert_eq!(lc.kind, GlossaryChangeKind::Restore);
        assert_eq!(lc.diff.total_removed, 1);
        assert_eq!(lc.diff.total_modified, 1);

        // …and the restore itself can be undone.
        restore(dir.path(), &entries[0].id).unwrap();
        assert_eq!(current(dir.path()).count(), 2);
    }

    /// With a full history the restore snapshot rotates the oldest file away —
    /// restoring that oldest version must still work.
    #[test]
    fn restoring_the_oldest_of_a_full_history_works() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..KEEP as i64 {
            write_glossary(dir.path(), &[("a", &format!("v{i}"))]);
            snapshot_at(dir.path(), GlossaryChangeKind::BulkEdit, 100 + i).unwrap();
        }
        write_glossary(dir.path(), &[("a", "now")]);
        let oldest = list(dir.path()).last().unwrap().id.clone();
        assert_eq!(oldest, "100");
        restore(dir.path(), &oldest).unwrap();
        assert_eq!(current(dir.path()).characters["a"], "v0");
        assert_eq!(list(dir.path()).len(), KEEP);
    }

    #[test]
    fn restore_rejects_unknown_and_malformed_ids() {
        let dir = tempfile::tempdir().unwrap();
        write_glossary(dir.path(), &[("a", "A")]);
        for id in ["999", "../glossary", "prev", ""] {
            let err = restore(dir.path(), id).unwrap_err().to_string();
            assert!(err.contains("isn't there any more"), "{id:?}: {err}");
        }
        // A failed restore changes nothing and takes no snapshot.
        assert_eq!(current(dir.path()).characters["a"], "A");
        assert!(list(dir.path()).is_empty());
    }

    #[test]
    fn legacy_prev_is_listed_and_restorable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(LEGACY_PREV_FILE),
            r#"{"world_type":"xianxia","terms":{"characters":{"林动":"Lin Dong"}}}"#,
        )
        .unwrap();
        write_glossary(dir.path(), &[("应欢欢", "Ying Huanhuan"), ("b", "B")]);
        save_meta(
            dir.path(),
            &GlossaryMeta {
                show_name: Some("Martial Universe".into()),
                scanned_files: vec!["e1.ass".into()],
                ..Default::default()
            },
        )
        .unwrap();
        // A real snapshot taken "later" than the prev file.
        snapshot(dir.path(), GlossaryChangeKind::TidyUp).unwrap();

        let entries = list(dir.path());
        assert_eq!(entries.len(), 2);
        let prev = entries.iter().find(|s| s.id == LEGACY_PREV_ID).unwrap();
        assert_eq!(prev.reason, GlossaryChangeKind::Rebuild);
        assert_eq!(prev.count, 1);
        assert!(prev.at_ms > 0);
        assert!(entries[0].at_ms >= entries[1].at_ms, "newest first");

        restore(dir.path(), LEGACY_PREV_ID).unwrap();
        let g = current(dir.path());
        assert_eq!(g.count(), 1);
        assert_eq!(g.characters["林动"], "Lin Dong");
        // No meta in the prev file: default origins, current scanned files and show name.
        let m = load_meta(dir.path());
        assert!(m.origins.is_empty());
        assert_eq!(m.scanned_files, vec!["e1.ass".to_string()]);
        assert_eq!(m.show_name.as_deref(), Some("Martial Universe"));
        // The prev file stays, still restorable; the version we left is in the history.
        assert!(dir.path().join(LEGACY_PREV_FILE).exists());
        let entries = list(dir.path());
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].reason, GlossaryChangeKind::Restore);
        assert_eq!(entries[0].count, 2);
    }

    #[test]
    fn termless_or_broken_legacy_prev_is_not_listed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(LEGACY_PREV_FILE), r#"{"version":3}"#).unwrap();
        assert!(list(dir.path()).is_empty());
        std::fs::write(dir.path().join(LEGACY_PREV_FILE), "not json").unwrap();
        assert!(list(dir.path()).is_empty());
    }

    /// A glossary.json that isn't valid JSON is kept verbatim, so going back to
    /// it gives the user exactly the file they had.
    #[test]
    fn broken_glossary_round_trips_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let broken = "{\"terms\": {\"characters\": {\"林动\": \"Lin Dong\",}}";
        std::fs::write(dir.path().join("glossary.json"), broken).unwrap();
        let id = snapshot(dir.path(), GlossaryChangeKind::Rebuild).unwrap().unwrap();
        assert_eq!(list(dir.path())[0].count, 0);

        write_glossary(dir.path(), &[("a", "A")]);
        restore(dir.path(), &id).unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("glossary.json")).unwrap(), broken);
    }
}
