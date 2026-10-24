//! Stored results: each episode's last outcome, issues and translations,
//! kept in the app's data folder (keyed by subtitle folder) so the episode
//! list, Review and Resume survive a restart. Nothing is written into the
//! user's subtitle folder except the translated files themselves.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::config::projects::Tone;
use crate::error::{AppError, AppResult};
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

/// What an episode's translation is made of: enough to rebuild its output
/// (Review) and to carry on where a stopped run left it (Resume). Stays in
/// the app's data folder; never sent to the webview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredLines {
    pub file: String,
    pub source_lang: String,
    pub target_lang: String,
    #[serde(default)]
    pub tone: Tone,
    /// [`fingerprint`] of the decoded source these lines belong to. Lines
    /// from an older version of the file are never applied to a newer one.
    pub source_hash: String,
    pub total_lines: u32,
    /// Line id (the pipeline's, 1-based) → the translation as written to the
    /// file, override tags in.
    pub translations: BTreeMap<u32, String>,
    /// [`fingerprint_bytes`] of the output as Polygluttony last wrote it, so
    /// Review won't overwrite changes made since in an editor. `None` for
    /// results stored before this was kept, and while there's no output.
    #[serde(default)]
    pub output_hash: Option<String>,
}

/// Stable 64-bit FNV-1a, hex. Unlike `DefaultHasher` it never changes
/// between Rust releases, so stored keys keep working.
fn fnv1a(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Fingerprint of a decoded subtitle file.
pub fn fingerprint(text: &str) -> String {
    fnv1a(text.as_bytes())
}

/// Fingerprint of a file's bytes as they are on disk.
pub fn fingerprint_bytes(bytes: &[u8]) -> String {
    fnv1a(bytes)
}

/// Epoch ms, for `finished_ms`.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// One process-wide lock over every read-modify-write: the run's file tasks
/// and the review commands all write here.
static LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// The folder results on disk: `{folder, episodes}`. `folder` is only there
/// for a human reading the file.
#[derive(Serialize)]
struct OnDisk<'a> {
    folder: String,
    episodes: &'a BTreeMap<String, EpisodeRecord>,
}

/// The stored results: `<dir>/<folder key>.json` holds a folder's
/// [`FolderResults`], `<dir>/<folder key>/<file key>.json` each episode's
/// [`StoredLines`]. Keys are stable hashes of the canonical folder path and of
/// the file name. A missing or damaged file reads as empty, never an error.
#[derive(Debug, Clone)]
pub struct ResultsStore {
    dir: PathBuf,
}

impl ResultsStore {
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// `<app data>/results`.
    pub fn for_app(app: &tauri::AppHandle) -> AppResult<Self> {
        use tauri::Manager;
        let base = app
            .path()
            .app_data_dir()
            .map_err(|e| AppError::Other(format!("no app-data dir: {e}")))?;
        Ok(Self::at(base.join("results")))
    }

    fn folder_key(folder: &Path) -> String {
        let canonical = std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
        let text = canonical.to_string_lossy();
        let trimmed = text.trim_end_matches(['/', '\\']);
        fnv1a(if trimmed.is_empty() {
            text.as_bytes()
        } else {
            trimmed.as_bytes()
        })
    }

    fn results_path(&self, folder: &Path) -> PathBuf {
        self.dir.join(format!("{}.json", Self::folder_key(folder)))
    }

    fn lines_path(&self, folder: &Path, file: &str) -> PathBuf {
        self.dir
            .join(Self::folder_key(folder))
            .join(format!("{}.json", fnv1a(file.as_bytes())))
    }

    fn read_results(&self, folder: &Path) -> FolderResults {
        std::fs::read_to_string(self.results_path(folder))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn write_results(&self, folder: &Path, results: &FolderResults) -> AppResult<()> {
        let on_disk = OnDisk {
            folder: folder.to_string_lossy().into_owned(),
            episodes: &results.episodes,
        };
        write_atomic(
            &self.results_path(folder),
            &serde_json::to_vec_pretty(&on_disk)?,
        )
    }

    fn read_lines(&self, folder: &Path, file: &str) -> Option<StoredLines> {
        let text = std::fs::read_to_string(self.lines_path(folder, file)).ok()?;
        let lines: StoredLines = serde_json::from_str(&text).ok()?;
        (lines.file == file).then_some(lines)
    }

    fn write_lines(&self, folder: &Path, lines: &StoredLines) -> AppResult<()> {
        write_atomic(
            &self.lines_path(folder, &lines.file),
            &serde_json::to_vec(lines)?,
        )
    }

    fn remove_lines(&self, folder: &Path, file: &str) -> AppResult<()> {
        match std::fs::remove_file(self.lines_path(folder, file)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    /// Every stored episode result for a folder.
    pub fn results(&self, folder: &Path) -> FolderResults {
        let _guard = lock();
        self.read_results(folder)
    }

    pub fn record(&self, folder: &Path, file: &str) -> Option<EpisodeRecord> {
        self.results(folder).episodes.remove(file)
    }

    pub fn lines(&self, folder: &Path, file: &str) -> Option<StoredLines> {
        let _guard = lock();
        self.read_lines(folder, file)
    }

    /// Store an episode's result, and its lines (`None` drops stored lines:
    /// a failed episode has none worth keeping).
    pub fn put_episode(
        &self,
        folder: &Path,
        record: EpisodeRecord,
        lines: Option<&StoredLines>,
    ) -> AppResult<()> {
        let _guard = lock();
        match lines {
            Some(lines) => self.write_lines(folder, lines)?,
            None => self.remove_lines(folder, &record.file)?,
        }
        let mut results = self.read_results(folder);
        results.episodes.insert(record.file.clone(), record);
        self.write_results(folder, &results)
    }

    /// Read-modify-write one episode's record and lines under the lock. `f`
    /// gets the record (when there is one) and the lines (when stored); what
    /// it leaves is written back. Errors from `f` write nothing.
    pub fn edit_episode<T>(
        &self,
        folder: &Path,
        file: &str,
        f: impl FnOnce(&mut EpisodeRecord, &mut Option<StoredLines>) -> AppResult<T>,
    ) -> AppResult<T> {
        let _guard = lock();
        let mut results = self.read_results(folder);
        let Some(record) = results.episodes.get_mut(file) else {
            return Err(AppError::Other(
                "There's no stored result for this episode. Translate it first.".into(),
            ));
        };
        let mut lines = self.read_lines(folder, file);
        let before = lines.clone();
        let out = f(record, &mut lines)?;
        if lines != before {
            match &lines {
                Some(lines) => self.write_lines(folder, lines)?,
                None => self.remove_lines(folder, file)?,
            }
        }
        self.write_results(folder, &results)?;
        Ok(out)
    }
}

/// Write via a temp file in the same folder and a rename, so a crash never
/// leaves half a file. No fsync: losing the newest result to a power cut is
/// acceptable; a torn one is not.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("results");
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    std::fs::write(&tmp, bytes)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(file: &str, outcome: EpisodeOutcome) -> EpisodeRecord {
        EpisodeRecord {
            file: file.into(),
            outcome,
            issues: Vec::new(),
            output_name: Some(format!("{file}.eng.ass")),
            finished_ms: 1,
            total_lines: 2,
            translated_lines: 2,
            reason: None,
        }
    }

    fn lines(file: &str) -> StoredLines {
        StoredLines {
            file: file.into(),
            source_lang: "zh".into(),
            target_lang: "en".into(),
            tone: Tone::Xianxia,
            source_hash: fingerprint("source"),
            total_lines: 2,
            translations: [(1, "{\\an8}One".to_string()), (2, "Two".to_string())].into(),
            output_hash: None,
        }
    }

    #[test]
    fn round_trip() {
        let data = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let store = ResultsStore::at(data.path().join("results"));
        assert!(store.results(folder.path()).episodes.is_empty());

        store
            .put_episode(
                folder.path(),
                record("ep01.ass", EpisodeOutcome::Done),
                Some(&lines("ep01.ass")),
            )
            .unwrap();
        store
            .put_episode(
                folder.path(),
                record("ep02.ass", EpisodeOutcome::Stopped),
                None,
            )
            .unwrap();

        // A fresh store on the same dir reads it all back.
        let again = ResultsStore::at(data.path().join("results"));
        let results = again.results(folder.path());
        assert_eq!(results.episodes.len(), 2);
        assert_eq!(results.episodes["ep01.ass"].outcome, EpisodeOutcome::Done);
        assert_eq!(
            results.episodes["ep02.ass"].outcome,
            EpisodeOutcome::Stopped
        );
        assert_eq!(
            again.lines(folder.path(), "ep01.ass"),
            Some(lines("ep01.ass"))
        );
        assert_eq!(again.lines(folder.path(), "ep02.ass"), None);
        // No temp files left behind.
        let leftovers: Vec<_> = walk(data.path())
            .into_iter()
            .filter(|p| p.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    fn walk(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        out
    }

    #[test]
    fn folders_are_kept_apart_and_spelling_does_not_matter() {
        let data = tempfile::tempdir().unwrap();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let store = ResultsStore::at(data.path());
        store
            .put_episode(a.path(), record("ep01.ass", EpisodeOutcome::Done), None)
            .unwrap();
        assert!(store.results(b.path()).episodes.is_empty());
        let slashed = PathBuf::from(format!("{}/", a.path().display()));
        assert_eq!(store.results(&slashed).episodes.len(), 1);
    }

    #[test]
    fn missing_or_corrupt_files_read_as_empty() {
        let data = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let store = ResultsStore::at(data.path());
        store
            .put_episode(
                folder.path(),
                record("ep01.ass", EpisodeOutcome::Done),
                Some(&lines("ep01.ass")),
            )
            .unwrap();

        std::fs::write(store.results_path(folder.path()), "{ not json").unwrap();
        std::fs::write(store.lines_path(folder.path(), "ep01.ass"), "[1, 2").unwrap();
        assert!(store.results(folder.path()).episodes.is_empty());
        assert_eq!(store.lines(folder.path(), "ep01.ass"), None);

        // Writing over a damaged store starts it afresh.
        store
            .put_episode(
                folder.path(),
                record("ep02.ass", EpisodeOutcome::Failed),
                None,
            )
            .unwrap();
        let results = store.results(folder.path());
        assert_eq!(results.episodes.keys().collect::<Vec<_>>(), ["ep02.ass"]);
    }

    #[test]
    fn lines_of_another_file_are_not_returned() {
        let data = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let store = ResultsStore::at(data.path());
        store
            .put_episode(
                folder.path(),
                record("ep01.ass", EpisodeOutcome::Done),
                Some(&lines("ep01.ass")),
            )
            .unwrap();
        // A file whose name hashes to the same path can't read someone else's lines.
        std::fs::copy(
            store.lines_path(folder.path(), "ep01.ass"),
            store.lines_path(folder.path(), "ep09.ass"),
        )
        .unwrap();
        assert_eq!(store.lines(folder.path(), "ep09.ass"), None);
    }

    #[test]
    fn edit_episode_writes_back_and_errors_write_nothing() {
        let data = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let store = ResultsStore::at(data.path());
        assert!(store
            .edit_episode(folder.path(), "ep01.ass", |_, _| Ok(()))
            .is_err());
        store
            .put_episode(
                folder.path(),
                record("ep01.ass", EpisodeOutcome::NeedsLook),
                Some(&lines("ep01.ass")),
            )
            .unwrap();

        store
            .edit_episode(folder.path(), "ep01.ass", |rec, lines| {
                rec.outcome = EpisodeOutcome::Done;
                lines
                    .as_mut()
                    .unwrap()
                    .translations
                    .insert(2, "Deux".into());
                Ok(())
            })
            .unwrap();
        assert_eq!(
            store.record(folder.path(), "ep01.ass").unwrap().outcome,
            EpisodeOutcome::Done
        );
        assert_eq!(
            store.lines(folder.path(), "ep01.ass").unwrap().translations[&2],
            "Deux"
        );

        let err = store.edit_episode(folder.path(), "ep01.ass", |rec, _| -> AppResult<()> {
            rec.outcome = EpisodeOutcome::Failed;
            Err(AppError::Other("no".into()))
        });
        assert!(err.is_err());
        assert_eq!(
            store.record(folder.path(), "ep01.ass").unwrap().outcome,
            EpisodeOutcome::Done
        );
    }

    #[test]
    fn lines_stored_before_output_fingerprints_still_load() {
        let old = r#"{"file":"ep01.ass","source_lang":"zh","target_lang":"en","tone":"xianxia",
            "source_hash":"abc","total_lines":2,"translations":{"1":"One"}}"#;
        let lines: StoredLines = serde_json::from_str(old).unwrap();
        assert_eq!(lines.output_hash, None);
        assert_eq!(lines.translations[&1], "One");
    }

    #[test]
    fn fingerprints_are_stable() {
        // Pinned: stored keys must survive Rust upgrades.
        assert_eq!(fnv1a(b""), "cbf29ce484222325");
        assert_eq!(fnv1a(b"a"), "af63dc4c8601ec8c");
        assert_ne!(fingerprint("a"), fingerprint("b"));
    }
}
