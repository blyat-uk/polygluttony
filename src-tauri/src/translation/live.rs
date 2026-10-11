//! Live board: what every episode of the current (or last) run looks like line
//! by line, for the episode windows. The run forwarder folds each pipeline
//! event into it ([`LiveBoard::apply`]) and nudges the windows on
//! `EPISODE_CHANGED_EVENT`; a window then pulls a full [`LiveBoard::snapshot`].
//! No replay: the board is the state, a window that opens late sees it all.
//!
//! Plain data behind a `std::sync::Mutex` (never held across an await, never
//! held while reading from disk), so the logic is testable without Tauri. It
//! lives until the next run resets it or the app quits.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ass::decode::decode_file;
use crate::ass::parse::{parse_dialogues, DialogueLine};
use crate::ass::tags::strip_for_text;
use crate::events::{
    EpisodeActivity, EpisodeChanged, EpisodeLine, EpisodeSnapshot, EpisodeSource, EpisodeView,
    FileStateKind, LineStatus, RunEvent, VerifyIssue,
};

/// Managed Tauri state. `None` until the first run of the session starts.
#[derive(Default)]
pub struct LiveBoard(Mutex<Option<Board>>);

/// What one applied event changed, i.e. which windows should refetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Nothing,
    File(String),
    All,
}

impl Change {
    /// The `EPISODE_CHANGED_EVENT` payload for this change, if any.
    pub fn into_event(self) -> Option<EpisodeChanged> {
        match self {
            Change::Nothing => None,
            Change::File(file) => Some(EpisodeChanged { file: Some(file) }),
            Change::All => Some(EpisodeChanged { file: None }),
        }
    }
}

struct Board {
    folder: PathBuf,
    run_active: bool,
    /// The run's episodes, in run order. Linear lookup: a run is tens of files.
    episodes: Vec<Episode>,
}

impl Board {
    /// Apply `f` to the episode, if it is one of this run's.
    fn update(&mut self, file: &str, f: impl FnOnce(&mut Episode)) -> Change {
        match self.episodes.iter_mut().find(|e| e.file == file) {
            Some(ep) => {
                f(ep);
                Change::File(file.to_string())
            }
            None => Change::Nothing,
        }
    }
}

struct Episode {
    file: String,
    /// `None` until the pipeline parses the file (or a snapshot reads it).
    lines: Option<Vec<EpisodeSource>>,
    /// Latest translation per line id, tags stripped. Never cleared within a
    /// run: a full redo re-sends lines that keep their old text until new text
    /// lands, so they read as re-sending rather than blank.
    translations: BTreeMap<u32, String>,
    in_flight: BTreeSet<u32>,
    activity: Option<EpisodeActivity>,
    activity_since_ms: Option<u64>,
    state: FileStateKind,
    detail: Option<String>,
    error: Option<String>,
    issues: Vec<VerifyIssue>,
}

impl Episode {
    fn new(file: String) -> Self {
        Self {
            file,
            lines: None,
            translations: BTreeMap::new(),
            in_flight: BTreeSet::new(),
            activity: None,
            activity_since_ms: None,
            state: FileStateKind::Pending,
            detail: None,
            error: None,
            issues: Vec::new(),
        }
    }

    /// The request is over (or the episode is): nothing is on its way.
    fn settle(&mut self) {
        self.in_flight.clear();
        self.activity = None;
        self.activity_since_ms = None;
    }

    fn finished(&self) -> bool {
        matches!(self.state, FileStateKind::Done | FileStateKind::Warning | FileStateKind::Failed)
    }

    fn snapshot(&self, run_active: bool) -> EpisodeSnapshot {
        let finished = self.finished();
        let lines: Vec<EpisodeLine> = self
            .lines
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|src| {
                let translation = self.translations.get(&src.id).cloned();
                let status = match (self.in_flight.contains(&src.id), translation.is_some()) {
                    (true, true) => LineStatus::Resending,
                    (true, false) => LineStatus::Sending,
                    (false, true) => LineStatus::Translated,
                    (false, false) if finished => LineStatus::Untranslated,
                    (false, false) => LineStatus::Waiting,
                };
                EpisodeLine {
                    id: src.id,
                    start_cs: src.start_cs,
                    end_cs: src.end_cs,
                    source: src.text.clone(),
                    translation,
                    status,
                }
            })
            .collect();
        EpisodeSnapshot {
            file: self.file.clone(),
            state: self.state,
            detail: self.detail.clone(),
            error: self.error.clone(),
            run_active,
            activity: self.activity.clone(),
            activity_since_ms: self.activity_since_ms,
            translated: lines.iter().filter(|l| l.translation.is_some()).count() as u32,
            total: lines.len() as u32,
            lines,
            issues: self.issues.clone(),
        }
    }
}

impl LiveBoard {
    /// A poisoned lock still holds plain, consistent-enough data: a panic
    /// mid-update leaves at worst one stale field, better than no window.
    fn lock(&self) -> MutexGuard<'_, Option<Board>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Start a fresh board for a new run: every episode pending, nothing known.
    pub fn reset(&self, folder: &Path, files: &[String]) {
        *self.lock() = Some(Board {
            folder: folder.to_path_buf(),
            run_active: true,
            episodes: files.iter().map(|f| Episode::new(f.clone())).collect(),
        });
    }

    /// Fold one run event into the board.
    pub fn apply(&self, event: &RunEvent) -> Change {
        let mut guard = self.lock();
        let Some(board) = guard.as_mut() else { return Change::Nothing };
        match event {
            RunEvent::Progress { .. } | RunEvent::Log { .. } => Change::Nothing,
            RunEvent::RunFinished { .. } => {
                board.run_active = false;
                board.episodes.iter_mut().for_each(Episode::settle);
                Change::All
            }
            RunEvent::State { file, state, detail } => board.update(file, |ep| {
                ep.state = *state;
                ep.detail = detail.clone();
                if ep.finished() {
                    ep.settle();
                }
            }),
            RunEvent::FileDone { file, issues, .. } => {
                board.update(file, |ep| ep.issues = issues.clone())
            }
            RunEvent::Error { file, message } => board.update(file, |ep| {
                ep.error = Some(message.clone());
                ep.state = FileStateKind::Failed;
                ep.settle();
            }),
            RunEvent::EpisodeLines { file, lines } => {
                board.update(file, |ep| ep.lines = Some(lines.clone()))
            }
            RunEvent::Sending { file, ids, activity } => board.update(file, |ep| {
                ep.in_flight = ids.iter().copied().collect();
                ep.activity = Some(activity.clone());
                ep.activity_since_ms = Some(now_ms());
            }),
            RunEvent::Landed { file, lines } => board.update(file, |ep| {
                for (id, text) in lines {
                    ep.in_flight.remove(id);
                    ep.translations.insert(*id, text.clone());
                }
            }),
            RunEvent::Idle { file } => board.update(file, Episode::settle),
        }
    }

    /// One episode as its window shows it. An episode still waiting has no
    /// lines on the board yet, so its source is read from disk (outside the
    /// lock) and kept for later snapshots.
    pub fn snapshot(&self, file: &str) -> EpisodeView {
        let folder = {
            let guard = self.lock();
            let Some(board) = guard.as_ref() else { return EpisodeView::NotInRun };
            let Some(ep) = board.episodes.iter().find(|e| e.file == file) else {
                return EpisodeView::NotInRun;
            };
            if ep.lines.is_some() {
                return EpisodeView::Live(ep.snapshot(board.run_active));
            }
            board.folder.clone()
        };

        let lines = match read_sources(&folder.join(file)) {
            Ok(lines) => lines,
            Err(message) => return EpisodeView::Unreadable { message },
        };

        let mut guard = self.lock();
        let Some(board) = guard.as_mut() else { return EpisodeView::NotInRun };
        if board.folder != folder {
            // A new run on another folder started while we read: these lines
            // belong to a file that may not be this run's. Start over.
            drop(guard);
            return self.snapshot(file);
        }
        let run_active = board.run_active;
        let Some(ep) = board.episodes.iter_mut().find(|e| e.file == file) else {
            return EpisodeView::NotInRun;
        };
        // The pipeline may have parsed it meanwhile; its lines win.
        ep.lines.get_or_insert(lines);
        EpisodeView::Live(ep.snapshot(run_active))
    }
}

/// The board's view of parsed dialogue: id = index + 1 (the pipeline's line
/// ids), times clamped into `u32`, override tags stripped.
pub fn episode_sources(lines: &[DialogueLine]) -> Vec<EpisodeSource> {
    let cs = |v: i64| u32::try_from(v.max(0)).unwrap_or(u32::MAX);
    lines
        .iter()
        .enumerate()
        .map(|(idx, d)| EpisodeSource {
            id: idx as u32 + 1,
            start_cs: cs(d.start_cs),
            end_cs: cs(d.end_cs),
            text: strip_for_text(&d.text),
        })
        .collect()
}

/// Decode and parse exactly as the pipeline does, so the lines match the ids
/// it will send later.
fn read_sources(path: &Path) -> Result<Vec<EpisodeSource>, String> {
    let text = decode_file(path).map_err(|e| e.to_string())?;
    Ok(episode_sources(&parse_dialogues(&text)))
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{FileResult, LogLevel, LogPhase};

    fn src(id: u32, text: &str) -> EpisodeSource {
        EpisodeSource { id, start_cs: id * 100, end_cs: id * 100 + 50, text: text.into() }
    }

    fn board_with_lines(n: u32) -> LiveBoard {
        let board = LiveBoard::default();
        board.reset(Path::new("/nowhere"), &["ep01.ass".into(), "ep02.ass".into()]);
        board.apply(&RunEvent::EpisodeLines {
            file: "ep01.ass".into(),
            lines: (1..=n).map(|i| src(i, &format!("line {i}"))).collect(),
        });
        board
    }

    fn live(view: EpisodeView) -> EpisodeSnapshot {
        match view {
            EpisodeView::Live(s) => s,
            other => panic!("expected a live snapshot, got {other:?}"),
        }
    }

    fn statuses(board: &LiveBoard) -> Vec<LineStatus> {
        live(board.snapshot("ep01.ass")).lines.iter().map(|l| l.status).collect()
    }

    fn state(file: &str, state: FileStateKind) -> RunEvent {
        RunEvent::State { file: file.into(), state, detail: None }
    }

    fn sending(ids: &[u32]) -> RunEvent {
        RunEvent::Sending {
            file: "ep01.ass".into(),
            ids: ids.to_vec(),
            activity: EpisodeActivity::Batch { batch: 1, total_batches: 1 },
        }
    }

    fn landed(lines: &[(u32, &str)]) -> RunEvent {
        RunEvent::Landed {
            file: "ep01.ass".into(),
            lines: lines.iter().map(|(id, t)| (*id, t.to_string())).collect(),
        }
    }

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

    #[test]
    fn line_status_follows_in_flight_and_translations() {
        let board = board_with_lines(4);
        assert_eq!(statuses(&board), vec![LineStatus::Waiting; 4]);

        board.apply(&sending(&[1, 2]));
        assert_eq!(
            statuses(&board),
            vec![
                LineStatus::Sending,
                LineStatus::Sending,
                LineStatus::Waiting,
                LineStatus::Waiting
            ]
        );

        // Line 2 is still on its way after a partial landing.
        board.apply(&landed(&[(1, "One")]));
        assert_eq!(
            statuses(&board),
            vec![
                LineStatus::Translated,
                LineStatus::Sending,
                LineStatus::Waiting,
                LineStatus::Waiting
            ]
        );

        board.apply(&RunEvent::Idle { file: "ep01.ass".into() });
        board.apply(&sending(&[1, 3]));
        let snap = live(board.snapshot("ep01.ass"));
        assert_eq!(
            snap.lines.iter().map(|l| l.status).collect::<Vec<_>>(),
            vec![
                LineStatus::Resending,
                LineStatus::Waiting,
                LineStatus::Sending,
                LineStatus::Waiting
            ]
        );
        assert_eq!(snap.lines[0].translation.as_deref(), Some("One"));
        assert_eq!((snap.translated, snap.total), (1, 4));
        assert!(snap.activity_since_ms.is_some());
        assert_eq!(snap.activity, Some(EpisodeActivity::Batch { batch: 1, total_batches: 1 }));

        board.apply(&landed(&[(1, "One again"), (3, "Three")]));
        board.apply(&RunEvent::Idle { file: "ep01.ass".into() });
        board.apply(&state("ep01.ass", FileStateKind::Warning));
        let snap = live(board.snapshot("ep01.ass"));
        assert_eq!(
            snap.lines.iter().map(|l| l.status).collect::<Vec<_>>(),
            vec![
                LineStatus::Translated,
                LineStatus::Untranslated,
                LineStatus::Translated,
                LineStatus::Untranslated
            ]
        );
        assert_eq!(snap.lines[0].translation.as_deref(), Some("One again"));
        assert_eq!(snap.activity, None);
        assert_eq!(snap.activity_since_ms, None);
    }

    #[test]
    fn idle_clears_in_flight_and_activity() {
        let board = board_with_lines(2);
        board.apply(&sending(&[1, 2]));
        assert_eq!(
            board.apply(&RunEvent::Idle { file: "ep01.ass".into() }),
            Change::File("ep01.ass".into())
        );
        let snap = live(board.snapshot("ep01.ass"));
        assert_eq!(
            snap.lines.iter().map(|l| l.status).collect::<Vec<_>>(),
            vec![LineStatus::Waiting; 2]
        );
        assert_eq!((snap.activity, snap.activity_since_ms), (None, None));
    }

    #[test]
    fn terminal_state_clears_in_flight() {
        for terminal in [FileStateKind::Done, FileStateKind::Warning, FileStateKind::Failed] {
            let board = board_with_lines(2);
            board.apply(&sending(&[1, 2]));
            board.apply(&state("ep01.ass", FileStateKind::Verifying));
            assert_eq!(
                statuses(&board),
                vec![LineStatus::Sending; 2],
                "non-terminal keeps in-flight"
            );
            board.apply(&state("ep01.ass", terminal));
            let snap = live(board.snapshot("ep01.ass"));
            assert_eq!(snap.state, terminal);
            assert_eq!(
                snap.lines.iter().map(|l| l.status).collect::<Vec<_>>(),
                vec![LineStatus::Untranslated; 2]
            );
            assert_eq!(snap.activity, None);
        }
    }

    #[test]
    fn error_fails_the_episode_and_clears_in_flight() {
        let board = board_with_lines(2);
        board.apply(&sending(&[1]));
        board.apply(&landed(&[(2, "Two")]));
        board.apply(&RunEvent::Error { file: "ep01.ass".into(), message: "dead key".into() });
        let snap = live(board.snapshot("ep01.ass"));
        assert_eq!(snap.state, FileStateKind::Failed);
        assert_eq!(snap.error.as_deref(), Some("dead key"));
        assert_eq!(
            snap.lines.iter().map(|l| l.status).collect::<Vec<_>>(),
            vec![LineStatus::Untranslated, LineStatus::Translated]
        );
    }

    #[test]
    fn state_detail_and_issues_are_kept() {
        let board = board_with_lines(1);
        board.apply(&RunEvent::State {
            file: "ep01.ass".into(),
            state: FileStateKind::Retranslating,
            detail: Some("attempt 1/3".into()),
        });
        let snap = live(board.snapshot("ep01.ass"));
        assert_eq!(
            (snap.state, snap.detail.as_deref()),
            (FileStateKind::Retranslating, Some("attempt 1/3"))
        );
        assert!(snap.issues.is_empty());

        let issue = VerifyIssue {
            line_id: 1,
            source: "line 1".into(),
            translation: "x".into(),
            issue_type: "drift".into(),
            description: "d".into(),
            severity: "warning".into(),
        };
        board.apply(&RunEvent::FileDone {
            file: "ep01.ass".into(),
            has_warnings: true,
            issues: vec![issue],
        });
        assert_eq!(live(board.snapshot("ep01.ass")).issues.len(), 1);
    }

    #[test]
    fn progress_and_log_change_nothing() {
        let board = board_with_lines(1);
        let progress = RunEvent::Progress {
            file: "ep01.ass".into(),
            translated: 0,
            total: 1,
            batch: 1,
            total_batches: 1,
            retries: 0,
        };
        let log = RunEvent::Log {
            file: Some("ep01.ass".into()),
            level: LogLevel::Info,
            phase: LogPhase::Batch,
            message: "m".into(),
        };
        assert_eq!(board.apply(&progress), Change::Nothing);
        assert_eq!(board.apply(&log), Change::Nothing);
        assert_eq!(board.apply(&sending(&[1])), Change::File("ep01.ass".into()));
    }

    #[test]
    fn events_for_files_outside_the_run_are_ignored() {
        let board = board_with_lines(1);
        let stranger = RunEvent::Sending {
            file: "other.ass".into(),
            ids: vec![1],
            activity: EpisodeActivity::Cleanup,
        };
        assert_eq!(board.apply(&stranger), Change::Nothing);
        assert_eq!(board.apply(&state("other.ass", FileStateKind::Done)), Change::Nothing);
        assert!(matches!(board.snapshot("other.ass"), EpisodeView::NotInRun));
        assert_eq!(statuses(&board), vec![LineStatus::Waiting]);
    }

    #[test]
    fn no_run_yet_is_not_in_run_and_ignores_events() {
        let board = LiveBoard::default();
        assert!(matches!(board.snapshot("ep01.ass"), EpisodeView::NotInRun));
        assert_eq!(board.apply(&state("ep01.ass", FileStateKind::Translating)), Change::Nothing);
        assert_eq!(board.apply(&RunEvent::RunFinished { results: vec![] }), Change::Nothing);
    }

    #[test]
    fn reset_replaces_the_previous_run() {
        let board = board_with_lines(2);
        board.apply(&landed(&[(1, "One")]));
        board.apply(&state("ep01.ass", FileStateKind::Done));
        board.apply(&RunEvent::RunFinished { results: vec![] });

        board.reset(Path::new("/nowhere"), &["ep01.ass".into(), "ep03.ass".into()]);
        assert!(matches!(board.snapshot("ep02.ass"), EpisodeView::NotInRun));
        board.apply(&RunEvent::EpisodeLines {
            file: "ep01.ass".into(),
            lines: vec![src(1, "line 1")],
        });
        let snap = live(board.snapshot("ep01.ass"));
        assert!(snap.run_active);
        assert_eq!(snap.state, FileStateKind::Pending);
        assert_eq!(snap.lines[0].translation, None);
        assert_eq!(snap.lines[0].status, LineStatus::Waiting);
    }

    #[test]
    fn run_finished_settles_every_episode() {
        let board = board_with_lines(2);
        board.apply(&RunEvent::EpisodeLines { file: "ep02.ass".into(), lines: vec![src(1, "a")] });
        board.apply(&sending(&[1, 2]));
        board.apply(&RunEvent::Sending {
            file: "ep02.ass".into(),
            ids: vec![1],
            activity: EpisodeActivity::Cleanup,
        });
        let results: Vec<FileResult> = vec![];
        assert_eq!(board.apply(&RunEvent::RunFinished { results }), Change::All);
        for file in ["ep01.ass", "ep02.ass"] {
            let snap = live(board.snapshot(file));
            assert!(!snap.run_active);
            assert_eq!(snap.activity, None);
            assert!(snap.lines.iter().all(|l| l.status == LineStatus::Waiting), "{file}");
        }
    }

    #[test]
    fn waiting_episode_reads_its_lines_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ep01.ass"), ass_source(&["{\\an8}你好\\N朋友", "再见"]))
            .unwrap();
        let board = LiveBoard::default();
        board.reset(dir.path(), &["ep01.ass".into()]);

        let snap = live(board.snapshot("ep01.ass"));
        assert_eq!(snap.state, FileStateKind::Pending);
        assert_eq!(snap.total, 2);
        assert_eq!(snap.lines[0].id, 1);
        assert_eq!(snap.lines[0].source, "你好\\N朋友");
        assert_eq!((snap.lines[0].start_cs, snap.lines[0].end_cs), (100, 150));
        assert_eq!(snap.lines[1].source, "再见");
        assert!(snap.lines.iter().all(|l| l.status == LineStatus::Waiting));

        // Kept: the file can go away and the board still knows the lines.
        std::fs::remove_file(dir.path().join("ep01.ass")).unwrap();
        assert_eq!(live(board.snapshot("ep01.ass")).total, 2);
    }

    #[test]
    fn pipeline_lines_replace_the_disk_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ep01.ass"), ass_source(&["你好"])).unwrap();
        let board = LiveBoard::default();
        board.reset(dir.path(), &["ep01.ass".into()]);
        assert_eq!(live(board.snapshot("ep01.ass")).total, 1);
        board.apply(&RunEvent::EpisodeLines {
            file: "ep01.ass".into(),
            lines: vec![src(1, "a"), src(2, "b")],
        });
        assert_eq!(live(board.snapshot("ep01.ass")).total, 2);
    }

    #[test]
    fn missing_source_is_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let board = LiveBoard::default();
        board.reset(dir.path(), &["gone.ass".into()]);
        match board.snapshot("gone.ass") {
            EpisodeView::Unreadable { message } => assert!(!message.is_empty()),
            other => panic!("expected unreadable, got {other:?}"),
        }
    }

    #[test]
    fn sources_clamp_times_and_strip_tags() {
        let line = |start_cs, end_cs, text: &str| DialogueLine {
            layer: 0,
            start_cs,
            end_cs,
            style: "Default".into(),
            name: String::new(),
            margin_l: 0,
            margin_r: 0,
            margin_v: 0,
            effect: String::new(),
            text: text.into(),
        };
        let out =
            episode_sources(&[line(-5, i64::MAX, "{\\i1}Hi{\\i0}\\nthere"), line(10, 20, "b")]);
        assert_eq!(
            out[0],
            EpisodeSource { id: 1, start_cs: 0, end_cs: u32::MAX, text: "Hi\\nthere".into() }
        );
        assert_eq!(out[1].id, 2);
    }

    #[test]
    fn change_maps_to_the_nudge_payload() {
        assert!(Change::Nothing.into_event().is_none());
        assert_eq!(Change::File("a".into()).into_event().unwrap().file.as_deref(), Some("a"));
        assert_eq!(Change::All.into_event().unwrap().file, None);
    }
}
