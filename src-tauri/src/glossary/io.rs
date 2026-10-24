//! Load and save `glossary.json` from/to a work folder (written by the Python
//! tool or by our Glossary step). Missing/invalid file ⇒ `None` on load —
//! glossaries are optional everywhere.

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::diff::GlossaryDiff;
use super::history;
use super::meta::{self, GlossaryChange, GlossaryChangeKind};
use super::model::Glossary;
use crate::error::{AppError, AppResult};

pub fn load_folder_glossary(folder: &Path) -> Option<Glossary> {
    let path = folder.join("glossary.json");
    let text = std::fs::read_to_string(path).ok()?;
    Glossary::from_json(&text)
}

/// Crash-safe write: temp file next to `dest` + rename. rename is atomic
/// against process crashes; we deliberately skip fsync — a power-loss-torn
/// glossary is recoverable by rebuilding (and from the history).
pub(crate) fn write_atomic(dest: &Path, contents: &str) -> AppResult<()> {
    let name = dest
        .file_name()
        .ok_or_else(|| AppError::Other(format!("{} isn't a file path", dest.display())))?;
    let tmp = dest.with_file_name(format!(".{}.tmp", name.to_string_lossy()));
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

/// Atomic, pretty JSON (the file is user-editable via "Open in editor").
pub fn save_folder_glossary(folder: &Path, glossary: &Glossary) -> AppResult<()> {
    write_atomic(&folder.join("glossary.json"), &glossary.to_json_pretty())
}

/// The newest restorable version in the history, as shown by the old
/// destructive confirm dialogs. `modified` is in unix SECONDS (the frontend's
/// `formatRelativeTime` takes seconds).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct GlossaryBackupInfo {
    pub count: u32,
    pub modified: i64,
}

/// The newest earlier version that holds terms — a history snapshot or the
/// legacy `glossary.prev.json`. `None` = nothing to go back to yet.
///
/// A version with zero terms is skipped: restoring it would only empty the
/// glossary, so promising the user something is recoverable from it would be
/// a lie.
pub fn backup_status(folder: &Path) -> Option<GlossaryBackupInfo> {
    history::list(folder)
        .into_iter()
        .find(|s| s.count > 0)
        .map(|s| GlossaryBackupInfo { count: s.count, modified: s.at_ms.div_euclid(1000) })
}

/// Install `src` as the folder's `glossary.json`. The current glossary goes
/// to the history first (reason `import`); every imported term is marked as
/// yours, nothing is left marked NEW. Returns the imported term count.
///
/// The empty-terms check is load-bearing, not defensive: `Glossary::from_json`
/// is lenient by design and drops unknown keys, so ANY valid JSON document
/// parses into an empty glossary. Without this, importing an unrelated `.json`
/// would silently succeed and wipe the folder's terms.
pub fn import_glossary_file(folder: &Path, src: &Path) -> AppResult<u32> {
    let dest = folder.join("glossary.json");
    // canonicalize fails for a non-existent path — fine: a missing dest can't
    // be src. Mirrors the guard in `commands::export_glossary`.
    if let (Ok(s), Ok(d)) = (src.canonicalize(), dest.canonicalize()) {
        if s == d {
            return Err(AppError::Other(
                "That glossary is already this folder's glossary.".into(),
            ));
        }
    }
    let text = std::fs::read_to_string(src)?;
    let glossary = Glossary::from_json(&text)
        .ok_or_else(|| AppError::Other("That file isn't valid JSON.".into()))?;
    if glossary.is_empty() {
        return Err(AppError::Other("That file has no glossary terms.".into()));
    }
    let before = load_folder_glossary(folder);
    history::snapshot(folder, GlossaryChangeKind::Import)?;
    save_folder_glossary(folder, &glossary)?;
    let mut m = meta::load_meta(folder);
    m.record_import(
        &glossary,
        GlossaryChange {
            kind: GlossaryChangeKind::Import,
            at_ms: meta::now_ms(),
            diff: GlossaryDiff::compute(before.as_ref(), &glossary),
        },
    );
    meta::save_meta(folder, &m)?;
    Ok(glossary.count() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_when_present_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_folder_glossary(dir.path()).is_none());
        std::fs::write(
            dir.path().join("glossary.json"),
            r#"{"world_type":"wuxia","terms":{"characters":{"张三":"Zhang San"}}}"#,
        )
        .unwrap();
        let g = load_folder_glossary(dir.path()).unwrap();
        assert_eq!(g.world_type, "wuxia");
        assert_eq!(g.characters.get("张三").unwrap(), "Zhang San");
    }

    #[test]
    fn save_is_atomic_and_pretty() {
        let dir = tempfile::tempdir().unwrap();
        let mut g = Glossary::new("wuxia");
        g.characters.insert("张三".into(), "Zhang San".into());
        save_folder_glossary(dir.path(), &g).unwrap();
        // No temp file left behind.
        assert!(!dir.path().join(".glossary.json.tmp").exists());
        let text = std::fs::read_to_string(dir.path().join("glossary.json")).unwrap();
        assert!(text.contains("\n  ")); // pretty
        let back = load_folder_glossary(dir.path()).unwrap();
        assert_eq!(back.characters.get("张三").unwrap(), "Zhang San");
    }

    #[test]
    fn write_atomic_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("x.json");
        write_atomic(&dest, "{}").unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "{}");
        assert!(!dir.path().join(".x.json.tmp").exists());
    }

    #[test]
    fn backup_status_none_without_history() {
        let dir = tempfile::tempdir().unwrap();
        assert!(backup_status(dir.path()).is_none());
        // A glossary.json alone is not an earlier version.
        let mut g = Glossary::new("xianxia");
        g.characters.insert("林动".into(), "Lin Dong".into());
        save_folder_glossary(dir.path(), &g).unwrap();
        assert!(backup_status(dir.path()).is_none());
    }

    #[test]
    fn backup_status_describes_the_latest_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let mut g = Glossary::new("xianxia");
        g.characters.insert("林动".into(), "Lin Dong".into());
        g.locations.insert("青阳镇".into(), "Qingyang Town".into());
        save_folder_glossary(dir.path(), &g).unwrap();
        history::snapshot(dir.path(), GlossaryChangeKind::TidyUp).unwrap();

        let info = backup_status(dir.path()).expect("a snapshot must be reported");
        assert_eq!(info.count, 2);
        // Unix SECONDS, close to now — not milliseconds.
        let now = meta::now_ms() / 1000;
        assert!((now - info.modified).abs() < 60, "{} vs {now}", info.modified);
    }

    /// `Glossary::from_json` is lenient, so an unrelated JSON document parses
    /// into an EMPTY glossary. Restoring it gives nothing back, so the status
    /// must not claim an earlier version exists.
    #[test]
    fn backup_status_skips_termless_versions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("glossary.prev.json"),
            r#"{"name":"some other config","version":3}"#,
        )
        .unwrap();
        assert!(backup_status(dir.path()).is_none());
    }

    #[test]
    fn import_writes_terms_and_returns_count() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("other.json");
        std::fs::write(
            &src,
            r#"{"world_type":"wuxia","terms":{"characters":{"张三":"Zhang San"},"locations":{"青阳镇":"Qingyang Town"}}}"#,
        )
        .unwrap();

        assert_eq!(import_glossary_file(dir.path(), &src).unwrap(), 2);
        let g = load_folder_glossary(dir.path()).unwrap();
        assert_eq!(g.characters.get("张三").unwrap(), "Zhang San");
        assert_eq!(g.locations.get("青阳镇").unwrap(), "Qingyang Town");
        assert_eq!(g.world_type, "wuxia", "the file's own world_type is preserved");
    }

    #[test]
    fn import_rejects_non_json() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("nope.json");
        std::fs::write(&src, "definitely not json").unwrap();
        let err = import_glossary_file(dir.path(), &src).unwrap_err().to_string();
        assert!(err.contains("isn't valid JSON"), "got: {err}");
        assert!(load_folder_glossary(dir.path()).is_none());
    }

    /// `Glossary::from_json` is deliberately lenient, so a valid-but-unrelated
    /// JSON document parses into an EMPTY glossary. Without this check an
    /// import of nothing would look like a success.
    #[test]
    fn import_rejects_empty_terms() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("unrelated.json");
        std::fs::write(&src, r#"{"name":"some other config","version":3}"#).unwrap();
        let err = import_glossary_file(dir.path(), &src).unwrap_err().to_string();
        assert!(err.contains("no glossary terms"), "got: {err}");
        assert!(load_folder_glossary(dir.path()).is_none());
    }

    #[test]
    fn import_rejects_self() {
        let dir = tempfile::tempdir().unwrap();
        let mut g = Glossary::new("xianxia");
        g.characters.insert("林动".into(), "Lin Dong".into());
        save_folder_glossary(dir.path(), &g).unwrap();

        let src = dir.path().join("glossary.json");
        let err = import_glossary_file(dir.path(), &src).unwrap_err().to_string();
        assert!(err.contains("already this folder"), "got: {err}");
        // Still intact.
        assert_eq!(load_folder_glossary(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn import_snapshots_the_existing_glossary_to_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut old = Glossary::new("xianxia");
        old.characters.insert("应欢欢".into(), "Ying Huanhuan".into());
        save_folder_glossary(dir.path(), &old).unwrap();

        let src = dir.path().join("new.json");
        std::fs::write(&src, r#"{"world_type":"xianxia","terms":{"characters":{"林动":"Lin Dong"}}}"#)
            .unwrap();
        assert_eq!(import_glossary_file(dir.path(), &src).unwrap(), 1);

        let g = load_folder_glossary(dir.path()).unwrap();
        assert_eq!(g.count(), 1);
        assert!(g.characters.contains_key("林动"));

        // The old glossary went to the history, not to glossary.prev.json.
        assert!(!dir.path().join("glossary.prev.json").exists());
        let snaps = history::list(dir.path());
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].reason, GlossaryChangeKind::Import);
        assert_eq!(snaps[0].count, 1);
        history::restore(dir.path(), &snaps[0].id).unwrap();
        let back = load_folder_glossary(dir.path()).unwrap();
        assert_eq!(back.characters.get("应欢欢").unwrap(), "Ying Huanhuan");
    }

    #[test]
    fn import_marks_every_term_yours_and_clears_new_marks() {
        use crate::glossary::meta::TermOrigin;
        let dir = tempfile::tempdir().unwrap();
        let mut old = Glossary::new("xianxia");
        old.characters.insert("应欢欢".into(), "Ying Huanhuan".into());
        save_folder_glossary(dir.path(), &old).unwrap();
        let mut m = meta::GlossaryMeta {
            show_name: Some("Martial Universe".into()),
            scanned_files: vec!["e1.ass".into()],
            ..Default::default()
        };
        m.unreviewed.insert("characters".into(), vec!["应欢欢".into()]);
        meta::save_meta(dir.path(), &m).unwrap();

        let src = dir.path().join("new.json");
        std::fs::write(
            &src,
            r#"{"world_type":"xianxia","terms":{"characters":{"林动":"Lin Dong"},"items":{"祖符":"Ancestral Symbol"}}}"#,
        )
        .unwrap();
        import_glossary_file(dir.path(), &src).unwrap();

        let m = meta::load_meta(dir.path());
        assert_eq!(m.origins["characters"]["林动"], TermOrigin::Yours);
        assert_eq!(m.origins["items"]["祖符"], TermOrigin::Yours);
        assert!(!m.origins.get("characters").is_some_and(|c| c.contains_key("应欢欢")));
        assert!(m.unreviewed.values().all(|v| v.is_empty()), "{:?}", m.unreviewed);
        let change = m.last_change.expect("import records its change");
        assert_eq!(change.kind, GlossaryChangeKind::Import);
        assert_eq!(change.diff.total_added, 2);
        assert_eq!(change.diff.total_removed, 1);
        // Untouched by an import.
        assert_eq!(m.show_name.as_deref(), Some("Martial Universe"));
        assert_eq!(m.scanned_files, vec!["e1.ass".to_string()]);
    }

    #[test]
    fn rejected_import_takes_no_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let mut old = Glossary::new("xianxia");
        old.characters.insert("应欢欢".into(), "Ying Huanhuan".into());
        save_folder_glossary(dir.path(), &old).unwrap();
        let src = dir.path().join("unrelated.json");
        std::fs::write(&src, r#"{"version":3}"#).unwrap();
        assert!(import_glossary_file(dir.path(), &src).is_err());
        assert!(history::list(dir.path()).is_empty());
    }
}
