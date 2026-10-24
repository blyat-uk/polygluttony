//! Review on stored results, outside any run: the stored view of an episode,
//! the user's own line, "Looks fine", "Mark episode done" and finding the
//! lines that use a glossary term. Pure over a [`ResultsStore`] and the
//! subtitle folder, so it's testable without Tauri; the commands add the
//! run-slot checks and the events.

use std::collections::BTreeMap;
use std::path::Path;

use crate::ass::decode::decode_file;
use crate::ass::parse::{parse_dialogues, DialogueLine};
use crate::ass::tags::{reapply, reapply_missing, strip_for_text, strip_positional};
use crate::error::{AppError, AppResult};
use crate::events::{EpisodeLine, EpisodeSnapshot, EpisodeView, FileStateKind, LineStatus};
use crate::models::language_pair::{output_filename, warning_filename, LanguagePair};
use crate::translation::live::episode_sources;
use crate::translation::output;
use crate::translation::results::{
    fingerprint, now_ms, EpisodeOutcome, EpisodeRecord, ResultsStore, StoredLines, TermLines,
};

/// Shown (as the snapshot's `detail`) when the source changed under its
/// stored translation.
pub const SOURCE_CHANGED: &str = "The original file changed since it was translated.";

fn other(message: impl Into<String>) -> AppError {
    AppError::Other(message.into())
}

/// The source, decoded and parsed, with its fingerprint.
struct Source {
    text: String,
    hash: String,
    lines: Vec<DialogueLine>,
}

fn read_source(folder: &Path, file: &str) -> AppResult<Source> {
    let text = decode_file(&folder.join(file))
        .map_err(|e| other(format!("Couldn't read the original file: {e}")))?;
    let hash = fingerprint(&text);
    let lines = parse_dialogues(&text);
    Ok(Source { text, hash, lines })
}

fn state_of(outcome: EpisodeOutcome) -> FileStateKind {
    match outcome {
        EpisodeOutcome::Done => FileStateKind::Done,
        EpisodeOutcome::NeedsLook => FileStateKind::Warning,
        EpisodeOutcome::Failed => FileStateKind::Failed,
        EpisodeOutcome::Stopped => FileStateKind::Stopped,
    }
}

/// A stored episode as its window shows it: the source's lines, the stored
/// translations (when they still belong to that source) and the open
/// issues. `None` when nothing is stored for it.
pub fn stored_snapshot(store: &ResultsStore, folder: &Path, file: &str) -> Option<EpisodeView> {
    let record = store.record(folder, file)?;
    let source = match read_source(folder, file) {
        Ok(s) => s,
        Err(e) => {
            return Some(EpisodeView::Unreadable {
                message: e.to_string(),
            })
        }
    };
    let stored = store.lines(folder, file);
    let changed = stored
        .as_ref()
        .is_some_and(|l| l.source_hash != source.hash);
    let translations: BTreeMap<u32, String> = match stored {
        Some(l) if !changed => l.translations,
        _ => BTreeMap::new(),
    };
    // A stopped episode's missing lines are still to come (Resume).
    let complete = record.outcome != EpisodeOutcome::Stopped;
    let lines: Vec<EpisodeLine> = episode_sources(&source.lines)
        .into_iter()
        .map(|src| {
            let translation = translations.get(&src.id).map(|t| strip_for_text(t));
            let status = match (&translation, complete) {
                (Some(_), _) => LineStatus::Translated,
                (None, true) => LineStatus::Untranslated,
                (None, false) => LineStatus::Waiting,
            };
            EpisodeLine {
                id: src.id,
                start_cs: src.start_cs,
                end_cs: src.end_cs,
                source: src.text,
                translation,
                status,
            }
        })
        .collect();
    Some(EpisodeView::Stored(EpisodeSnapshot {
        file: file.to_string(),
        state: state_of(record.outcome),
        detail: changed.then(|| SOURCE_CHANGED.to_string()),
        error: record.reason,
        run_active: false,
        activity: None,
        activity_since_ms: None,
        translated: lines.iter().filter(|l| l.translation.is_some()).count() as u32,
        total: lines.len() as u32,
        lines,
        issues: if changed { Vec::new() } else { record.issues },
        throttle: None,
    }))
}

/// Rewrite an episode's output from its source and translations, keeping
/// the writer's header and event handling, and remove the other variant
/// (clean or `.warning`). Untranslated lines stay out. Returns the
/// fingerprint of what was written.
fn write_output(
    folder: &Path,
    file: &str,
    pair: &LanguagePair,
    output_name: &str,
    source: &Source,
    translations: &BTreeMap<u32, String>,
) -> AppResult<String> {
    let out: Vec<DialogueLine> = source
        .lines
        .iter()
        .enumerate()
        .filter_map(|(idx, d)| {
            translations.get(&(idx as u32 + 1)).map(|t| DialogueLine {
                text: t.clone(),
                ..d.clone()
            })
        })
        .collect();
    output::write(&folder.join(file), pair, output_name, &source.text, &out)
        .map_err(|e| other(format!("Couldn't write {output_name}: {e}")))
}

/// The output of a finished episode, and its stored lines checked against
/// the source as it is now.
fn finished<'a>(
    record: &EpisodeRecord,
    lines: &'a mut Option<StoredLines>,
    source: &Source,
) -> AppResult<(String, &'a mut StoredLines)> {
    let output_name = match (record.outcome, &record.output_name) {
        (EpisodeOutcome::Done | EpisodeOutcome::NeedsLook, Some(name)) => name.clone(),
        _ => {
            return Err(other(
                "This episode has no finished translation yet. Translate it first.",
            ))
        }
    };
    let lines = lines.as_mut().ok_or_else(|| {
        other("The lines of this episode aren't stored any more. Translate it again to edit it.")
    })?;
    if lines.source_hash != source.hash {
        return Err(other(format!(
            "{SOURCE_CHANGED} Translate it again to edit it."
        )));
    }
    Ok((output_name, lines))
}

/// The user's own translation of one line, override tags kept: the source
/// line's tags go back where they were, as the pipeline does; when the user
/// typed tags of their own, those the typed text doesn't set already. Rewrites
/// the output and clears the line's issues. Refused when the output changed
/// on disk since Polygluttony wrote it.
pub fn save_line(
    store: &ResultsStore,
    folder: &Path,
    file: &str,
    line_id: u32,
    text: &str,
) -> AppResult<()> {
    let text = text.trim().replace("\r\n", "\\N").replace('\n', "\\N");
    if text.is_empty() {
        return Err(other("A line can't be empty. Retranslate it instead."));
    }
    let source = read_source(folder, file)?;
    store.edit_episode(folder, file, |record, lines| {
        let (output_name, lines) = finished(record, lines, &source)?;
        let raw = line_id
            .checked_sub(1)
            .and_then(|i| source.lines.get(i as usize))
            .ok_or_else(|| other(format!("Line {line_id} isn't in this episode.")))?;
        let tagged = if text.contains("{\\") {
            reapply_missing(&strip_positional(&raw.text), &text)
        } else {
            reapply(&strip_positional(&raw.text), &text)
        };
        output::check_unchanged(folder, &output_name, lines.output_hash.as_deref())?;
        let pair = LanguagePair::from_codes(&lines.source_lang, &lines.target_lang)?;
        lines.translations.insert(line_id, tagged);
        let hash =
            write_output(folder, file, &pair, &output_name, &source, &lines.translations)?;
        lines.output_hash = Some(hash);
        record.issues.retain(|i| i.line_id != line_id);
        record.translated_lines = lines.translations.len() as u32;
        Ok(())
    })
}

/// "Looks fine": drop that issue from the line. With `term`, only the issue
/// about that name (a line can have one per name); without, every issue of
/// that type on the line. Already gone is fine.
pub fn dismiss_issue(
    store: &ResultsStore,
    folder: &Path,
    file: &str,
    line_id: u32,
    issue_type: &str,
    term: Option<&str>,
) -> AppResult<()> {
    store.edit_episode(folder, file, |record, _| {
        record.issues.retain(|i| {
            let this = i.line_id == line_id
                && i.issue_type == issue_type
                && term.is_none_or(|t| i.term.as_deref() == Some(t));
            !this
        });
        Ok(())
    })
}

/// "Mark episode done": the `.warning` output takes the clean name (an old
/// clean leftover is replaced) and the episode is done, its issues settled.
/// `fallback` is the folder's language pair, for an episode translated
/// before results were stored.
pub fn mark_episode_done(
    store: &ResultsStore,
    folder: &Path,
    file: &str,
    fallback: Option<LanguagePair>,
) -> AppResult<()> {
    let pair = match store.lines(folder, file) {
        Some(l) => LanguagePair::from_codes(&l.source_lang, &l.target_lang)?,
        None => {
            fallback.ok_or_else(|| other("Polygluttony doesn't know this folder's languages."))?
        }
    };
    let source = folder.join(file);
    let clean_name = output_filename(&source, &pair);
    let clean = folder.join(&clean_name);
    let warning = folder.join(warning_filename(&source, &pair));
    if warning.exists() {
        if clean.exists() {
            std::fs::remove_file(&clean)
                .map_err(|e| other(format!("Couldn't replace the old {clean_name}: {e}")))?;
        }
        std::fs::rename(&warning, &clean)
            .map_err(|e| other(format!("Couldn't rename the file to {clean_name}: {e}")))?;
    } else if !clean.exists() {
        return Err(other(
            "The translated file isn't there any more. Translate the episode again.",
        ));
    }

    if store.record(folder, file).is_some() {
        store.edit_episode(folder, file, |record, _| {
            record.outcome = EpisodeOutcome::Done;
            record.output_name = Some(clean_name.clone());
            record.issues.clear();
            record.reason = None;
            Ok(())
        })
    } else {
        let count = |path: &Path| {
            decode_file(path)
                .map(|t| parse_dialogues(&t).len() as u32)
                .unwrap_or(0)
        };
        let record = EpisodeRecord {
            file: file.to_string(),
            outcome: EpisodeOutcome::Done,
            issues: Vec::new(),
            output_name: Some(clean_name),
            finished_ms: now_ms(),
            total_lines: count(&source),
            translated_lines: count(&clean),
            reason: None,
        };
        store.put_episode(folder, record, None)
    }
}

/// Lines of finished episodes whose source contains `term`, by file. Only
/// lines with a stored translation whose source is unchanged count.
pub fn find_term_lines(store: &ResultsStore, folder: &Path, term: &str) -> Vec<TermLines> {
    let term = term.trim();
    if term.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (file, record) in store.results(folder).episodes {
        if !matches!(
            record.outcome,
            EpisodeOutcome::Done | EpisodeOutcome::NeedsLook
        ) {
            continue;
        }
        let Some(lines) = store.lines(folder, &file) else {
            continue;
        };
        let Ok(source) = read_source(folder, &file) else {
            continue;
        };
        if source.hash != lines.source_hash {
            continue;
        }
        let line_ids: Vec<u32> = lines
            .translations
            .keys()
            .copied()
            .filter(|id| {
                id.checked_sub(1)
                    .and_then(|i| source.lines.get(i as usize))
                    .is_some_and(|d| strip_for_text(&d.text).contains(term))
            })
            .collect();
        if !line_ids.is_empty() {
            out.push(TermLines { file, line_ids });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::projects::Tone;
    use crate::events::VerifyIssue;

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

    fn issue(line_id: u32, issue_type: &str) -> VerifyIssue {
        VerifyIssue {
            line_id,
            source: String::new(),
            translation: String::new(),
            issue_type: issue_type.into(),
            description: "d".into(),
            severity: "high".into(),
            term: None,
            expected: None,
        }
    }

    /// A folder with one finished episode: three lines, the first tagged,
    /// line 3 untranslated, written to the `.warning` output.
    struct Fixture {
        _data: tempfile::TempDir,
        folder: tempfile::TempDir,
        store: ResultsStore,
    }

    fn fixture(outcome: EpisodeOutcome) -> Fixture {
        let data = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let store = ResultsStore::at(data.path());
        let src = ass_source(&["{\\an8}叶长生不会走的", "你先走", "你算什么东西"]);
        std::fs::write(folder.path().join("ep01.ass"), &src).unwrap();
        let translations: BTreeMap<u32, String> = [
            (1, "{\\an8}Ye Changshen won't leave.".to_string()),
            (2, "You go first.".to_string()),
        ]
        .into();
        let output = "ep01.warning.eng.ass";
        let source = read_source(folder.path(), "ep01.ass").unwrap();
        let pair = LanguagePair::from_codes("zh", "en").unwrap();
        let output_hash =
            write_output(folder.path(), "ep01.ass", &pair, output, &source, &translations).unwrap();
        let record = EpisodeRecord {
            file: "ep01.ass".into(),
            outcome,
            issues: vec![
                issue(1, "name"),
                issue(1, "meaning"),
                issue(3, "untranslated"),
            ],
            output_name: Some(output.into()),
            finished_ms: 1,
            total_lines: 3,
            translated_lines: 2,
            reason: None,
        };
        let lines = StoredLines {
            file: "ep01.ass".into(),
            source_lang: "zh".into(),
            target_lang: "en".into(),
            tone: Tone::Xianxia,
            source_hash: fingerprint(&src),
            total_lines: 3,
            translations,
            output_hash: Some(output_hash),
        };
        store
            .put_episode(folder.path(), record, Some(&lines))
            .unwrap();
        Fixture {
            _data: data,
            folder,
            store,
        }
    }

    fn stored(fx: &Fixture) -> EpisodeSnapshot {
        match stored_snapshot(&fx.store, fx.folder.path(), "ep01.ass") {
            Some(EpisodeView::Stored(s)) => s,
            other => panic!("expected stored, got {other:?}"),
        }
    }

    #[test]
    fn stored_snapshot_rebuilds_the_review_view() {
        let fx = fixture(EpisodeOutcome::NeedsLook);
        let snap = stored(&fx);
        assert_eq!(snap.state, FileStateKind::Warning);
        assert!(!snap.run_active);
        assert_eq!((snap.translated, snap.total), (2, 3));
        assert_eq!(snap.lines[0].source, "叶长生不会走的");
        assert_eq!(
            snap.lines[0].translation.as_deref(),
            Some("Ye Changshen won't leave.")
        );
        assert_eq!(snap.lines[2].status, LineStatus::Untranslated);
        assert_eq!(snap.issues.len(), 3);
        assert!(stored_snapshot(&fx.store, fx.folder.path(), "other.ass").is_none());
    }

    #[test]
    fn a_stopped_episode_shows_its_missing_lines_as_waiting() {
        let fx = fixture(EpisodeOutcome::Stopped);
        assert_eq!(stored(&fx).lines[2].status, LineStatus::Waiting);
        assert_eq!(stored(&fx).state, FileStateKind::Stopped);
    }

    #[test]
    fn a_changed_source_drops_the_stale_translations() {
        let fx = fixture(EpisodeOutcome::NeedsLook);
        std::fs::write(
            fx.folder.path().join("ep01.ass"),
            ass_source(&["别的", "台词"]),
        )
        .unwrap();
        let snap = stored(&fx);
        assert_eq!(snap.detail.as_deref(), Some(SOURCE_CHANGED));
        assert!(snap.lines.iter().all(|l| l.translation.is_none()));
        assert!(snap.issues.is_empty());
        let err = save_line(&fx.store, fx.folder.path(), "ep01.ass", 1, "x").unwrap_err();
        assert!(err.to_string().contains("changed since"));
    }

    #[test]
    fn save_line_keeps_override_tags_and_clears_that_lines_issues() {
        let fx = fixture(EpisodeOutcome::NeedsLook);
        save_line(
            &fx.store,
            fx.folder.path(),
            "ep01.ass",
            1,
            "  Ye Changsheng won't leave. ",
        )
        .unwrap();
        let written =
            std::fs::read_to_string(fx.folder.path().join("ep01.warning.eng.ass")).unwrap();
        assert!(
            written.contains(",,{\\an8}Ye Changsheng won't leave.\n"),
            "{written}"
        );
        assert!(written.contains("You go first."));
        let lines = fx.store.lines(fx.folder.path(), "ep01.ass").unwrap();
        assert_eq!(lines.translations[&1], "{\\an8}Ye Changsheng won't leave.");
        let record = fx.store.record(fx.folder.path(), "ep01.ass").unwrap();
        assert_eq!(
            record.issues.iter().map(|i| i.line_id).collect::<Vec<_>>(),
            [3]
        );

        // Writing an untranslated line fills it in; typed tags are kept as typed.
        save_line(
            &fx.store,
            fx.folder.path(),
            "ep01.ass",
            3,
            "{\\i1}Who{\\i0} are you?",
        )
        .unwrap();
        let record = fx.store.record(fx.folder.path(), "ep01.ass").unwrap();
        assert!(record.issues.is_empty());
        assert_eq!(record.translated_lines, 3);
        let written =
            std::fs::read_to_string(fx.folder.path().join("ep01.warning.eng.ass")).unwrap();
        assert!(written.contains("{\\i1}Who{\\i0} are you?"));
    }

    #[test]
    fn save_line_refuses_what_it_cannot_do() {
        let fx = fixture(EpisodeOutcome::NeedsLook);
        assert!(save_line(&fx.store, fx.folder.path(), "ep01.ass", 1, "   ").is_err());
        assert!(save_line(&fx.store, fx.folder.path(), "ep01.ass", 9, "x").is_err());
        assert!(save_line(&fx.store, fx.folder.path(), "ep01.ass", 0, "x").is_err());
        let stopped = fixture(EpisodeOutcome::Stopped);
        assert!(save_line(&stopped.store, stopped.folder.path(), "ep01.ass", 1, "x").is_err());
        // Nothing changed on the way.
        assert_eq!(
            fx.store
                .record(fx.folder.path(), "ep01.ass")
                .unwrap()
                .issues
                .len(),
            3
        );
    }

    #[test]
    fn dismiss_drops_only_that_issue() {
        let fx = fixture(EpisodeOutcome::NeedsLook);
        dismiss_issue(&fx.store, fx.folder.path(), "ep01.ass", 1, "name", None).unwrap();
        dismiss_issue(&fx.store, fx.folder.path(), "ep01.ass", 1, "name", None).unwrap();
        let types: Vec<(u32, String)> = fx
            .store
            .record(fx.folder.path(), "ep01.ass")
            .unwrap()
            .issues
            .into_iter()
            .map(|i| (i.line_id, i.issue_type))
            .collect();
        assert_eq!(
            types,
            [(1, "meaning".to_string()), (3, "untranslated".to_string())]
        );
    }

    #[test]
    fn looks_fine_on_one_name_leaves_the_lines_other_names() {
        let fx = fixture(EpisodeOutcome::NeedsLook);
        let named = |term: &str| VerifyIssue { term: Some(term.into()), ..issue(2, "name") };
        fx.store
            .edit_episode(fx.folder.path(), "ep01.ass", |record, _| {
                record.issues = vec![named("叶长生"), named("林动"), issue(2, "meaning")];
                Ok(())
            })
            .unwrap();
        dismiss_issue(&fx.store, fx.folder.path(), "ep01.ass", 2, "name", Some("林动")).unwrap();
        let left: Vec<(String, Option<String>)> = fx
            .store
            .record(fx.folder.path(), "ep01.ass")
            .unwrap()
            .issues
            .into_iter()
            .map(|i| (i.issue_type, i.term))
            .collect();
        assert_eq!(
            left,
            [("name".to_string(), Some("叶长生".to_string())), ("meaning".to_string(), None)]
        );
    }

    #[test]
    fn typed_tags_keep_the_lines_hidden_positioning() {
        let fx = fixture(EpisodeOutcome::NeedsLook);
        save_line(&fx.store, fx.folder.path(), "ep01.ass", 1, "{\\i1}Ye Changsheng{\\i0} won't leave.")
            .unwrap();
        let lines = fx.store.lines(fx.folder.path(), "ep01.ass").unwrap();
        assert_eq!(lines.translations[&1], "{\\an8}{\\i1}Ye Changsheng{\\i0} won't leave.");
    }

    #[test]
    fn save_line_wont_overwrite_an_output_changed_on_disk() {
        let fx = fixture(EpisodeOutcome::NeedsLook);
        let out = fx.folder.path().join("ep01.warning.eng.ass");
        std::fs::write(&out, "fixed by hand").unwrap();
        let err = save_line(&fx.store, fx.folder.path(), "ep01.ass", 2, "Go on.").unwrap_err();
        assert_eq!(
            err.to_string(),
            "ep01.warning.eng.ass was changed outside Polygluttony since it was written. Open it \
             in your editor, or translate the episode again."
        );
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "fixed by hand");
        let lines = fx.store.lines(fx.folder.path(), "ep01.ass").unwrap();
        assert_eq!(lines.translations[&2], "You go first.", "nothing stored either");

        // A result stored before fingerprints were kept: rewritten, as before.
        fx.store
            .edit_episode(fx.folder.path(), "ep01.ass", |_, lines| {
                lines.as_mut().unwrap().output_hash = None;
                Ok(())
            })
            .unwrap();
        save_line(&fx.store, fx.folder.path(), "ep01.ass", 2, "Go on.").unwrap();
        assert!(std::fs::read_to_string(&out).unwrap().contains("Go on."));
        // …and from then on it is fingerprinted.
        assert!(fx.store.lines(fx.folder.path(), "ep01.ass").unwrap().output_hash.is_some());
    }

    #[test]
    fn save_line_removes_a_stale_clean_output() {
        let fx = fixture(EpisodeOutcome::NeedsLook);
        let clean = fx.folder.path().join("ep01.eng.ass");
        std::fs::write(&clean, "an older translation").unwrap();
        save_line(&fx.store, fx.folder.path(), "ep01.ass", 2, "Go on.").unwrap();
        assert!(!clean.exists());
        assert!(fx.folder.path().join("ep01.warning.eng.ass").exists());
    }

    #[test]
    fn mark_done_renames_the_warning_file() {
        let fx = fixture(EpisodeOutcome::NeedsLook);
        mark_episode_done(&fx.store, fx.folder.path(), "ep01.ass", None).unwrap();
        assert!(!fx.folder.path().join("ep01.warning.eng.ass").exists());
        assert!(fx.folder.path().join("ep01.eng.ass").exists());
        let record = fx.store.record(fx.folder.path(), "ep01.ass").unwrap();
        assert_eq!(record.outcome, EpisodeOutcome::Done);
        assert_eq!(record.output_name.as_deref(), Some("ep01.eng.ass"));
        assert!(record.issues.is_empty());
        // Again is harmless: the clean file is there.
        mark_episode_done(&fx.store, fx.folder.path(), "ep01.ass", None).unwrap();
    }

    #[test]
    fn mark_done_replaces_an_old_clean_leftover() {
        let fx = fixture(EpisodeOutcome::NeedsLook);
        std::fs::write(fx.folder.path().join("ep01.eng.ass"), "an old translation").unwrap();
        mark_episode_done(&fx.store, fx.folder.path(), "ep01.ass", None).unwrap();
        let clean = std::fs::read_to_string(fx.folder.path().join("ep01.eng.ass")).unwrap();
        assert!(clean.contains("You go first."), "the reviewed file wins");
        assert!(!fx.folder.path().join("ep01.warning.eng.ass").exists());
    }

    #[test]
    fn mark_done_without_a_stored_result_uses_the_folders_languages() {
        let data = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let store = ResultsStore::at(data.path());
        std::fs::write(
            folder.path().join("ep02.ass"),
            ass_source(&["你好", "再见"]),
        )
        .unwrap();
        std::fs::write(
            folder.path().join("ep02.warning.eng.ass"),
            ass_source(&["Hi"]),
        )
        .unwrap();
        assert!(mark_episode_done(&store, folder.path(), "ep02.ass", None).is_err());
        let pair = LanguagePair::from_codes("zh", "en").unwrap();
        mark_episode_done(&store, folder.path(), "ep02.ass", Some(pair)).unwrap();
        assert!(folder.path().join("ep02.eng.ass").exists());
        let record = store.record(folder.path(), "ep02.ass").unwrap();
        assert_eq!(
            (record.outcome, record.total_lines, record.translated_lines),
            (EpisodeOutcome::Done, 2, 1)
        );
        // Nothing to rename and nothing there: say so.
        let gone = mark_episode_done(
            &store,
            folder.path(),
            "ep03.ass",
            Some(LanguagePair::from_codes("zh", "en").unwrap()),
        );
        assert!(gone.unwrap_err().to_string().contains("isn't there"));
    }

    #[test]
    fn find_term_lines_lists_finished_lines_using_the_term() {
        let fx = fixture(EpisodeOutcome::NeedsLook);
        // Another finished episode, and a stopped one that doesn't count.
        let src2 = ass_source(&["好的", "叶长生来了"]);
        std::fs::write(fx.folder.path().join("ep02.ass"), &src2).unwrap();
        for (file, outcome) in [
            ("ep02.ass", EpisodeOutcome::Done),
            ("ep03.ass", EpisodeOutcome::Stopped),
        ] {
            if file == "ep03.ass" {
                std::fs::write(fx.folder.path().join(file), &src2).unwrap();
            }
            let record = EpisodeRecord {
                file: file.into(),
                outcome,
                issues: Vec::new(),
                output_name: Some(format!("{file}.out")),
                finished_ms: 1,
                total_lines: 2,
                translated_lines: 2,
                reason: None,
            };
            let lines = StoredLines {
                file: file.into(),
                source_lang: "zh".into(),
                target_lang: "en".into(),
                tone: Tone::Standard,
                source_hash: fingerprint(&src2),
                total_lines: 2,
                translations: [
                    (1, "Fine.".to_string()),
                    (2, "Ye Changshen is here.".to_string()),
                ]
                .into(),
                output_hash: None,
            };
            fx.store
                .put_episode(fx.folder.path(), record, Some(&lines))
                .unwrap();
        }
        let found = find_term_lines(&fx.store, fx.folder.path(), " 叶长生 ");
        let found: Vec<(String, Vec<u32>)> =
            found.into_iter().map(|t| (t.file, t.line_ids)).collect();
        assert_eq!(
            found,
            [
                ("ep01.ass".to_string(), vec![1]),
                ("ep02.ass".to_string(), vec![2])
            ],
            "ep01 line 1 (tags don't matter), ep02 line 2; not the stopped ep03"
        );
        assert!(find_term_lines(&fx.store, fx.folder.path(), "  ").is_empty());
        assert!(find_term_lines(&fx.store, fx.folder.path(), "没有").is_empty());
    }
}
