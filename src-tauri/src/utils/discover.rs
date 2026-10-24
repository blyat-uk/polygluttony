//! Discover source `.ass` files in a folder and read their translation output.
//! Ported from `utils/file_utils.py`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::models::language_pair::{
    output_filename, strip_language_suffix, warning_filename, LanguagePair,
};
use crate::models::TranslationState;

/// All `.ass` files in `dir` (non-recursive) that are *source* files — not
/// themselves translation outputs. A file tagged with the target language
/// (`ep01.eng.ass`) is an output only when a source with the same base name
/// sits beside it; on its own it's a source (an English `movie.en.ass` opened
/// under a zh→en default). Sorted by path.
pub fn discover_source_files(dir: &Path, pair: &LanguagePair) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let target_suffixes = pair.target_suffixes();
    let target_tagged = |stem: &str| {
        let lower = stem.to_lowercase();
        target_suffixes
            .iter()
            .any(|suf| lower.ends_with(&format!(".{}", suf.to_lowercase())))
    };
    let mut candidates: Vec<(PathBuf, String)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_ass = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("ass"))
            == Some(true);
        if !is_ass {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if stem.to_lowercase().contains(".warning.") {
            continue;
        }
        candidates.push((path, stem));
    }
    let source_bases: HashSet<String> = candidates
        .iter()
        .filter(|(_, stem)| !target_tagged(stem))
        .map(|(_, stem)| strip_language_suffix(stem).to_lowercase())
        .collect();
    let mut files: Vec<PathBuf> = candidates
        .into_iter()
        .filter(|(_, stem)| {
            !target_tagged(stem)
                || !source_bases.contains(&strip_language_suffix(stem).to_lowercase())
        })
        .map(|(path, _)| path)
        .collect();
    files.sort();
    files
}

/// The translation output next to a source file, as it is on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputOnDisk {
    pub state: TranslationState,
    /// Both the clean and the `.warning` output exist (an old leftover).
    pub both_variants: bool,
    /// The output that stands for the episode: the clean one when it exists,
    /// else the `.warning` one; `None` when there's neither.
    pub path: Option<PathBuf>,
}

/// What translation output exists next to `source`. A clean output wins over a
/// `.warning` one: the episode is done, the warning file is a leftover.
pub fn output_on_disk(source: &Path, pair: &LanguagePair) -> OutputOnDisk {
    let dir = source.parent().unwrap_or_else(|| Path::new("."));
    let clean = dir.join(output_filename(source, pair));
    let warning = dir.join(warning_filename(source, pair));
    let (has_clean, has_warning) = (clean.is_file(), warning.is_file());
    let (state, path) = match (has_clean, has_warning) {
        (true, _) => (TranslationState::Done, Some(clean)),
        (false, true) => (TranslationState::NeedsLook, Some(warning)),
        (false, false) => (TranslationState::None, None),
    };
    OutputOnDisk {
        state,
        both_variants: has_clean && has_warning,
        path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::language_pair::LanguagePair;
    use std::fs;

    #[test]
    fn discovers_sources_excluding_translations() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        for name in [
            "ep01.ass",
            "ep01.eng.ass",
            "ep02.ass",
            "ep03.ass",
            "ep03.warning.eng.ass",
            "notes.txt",
        ] {
            fs::write(p.join(name), "x").unwrap();
        }
        let pair = LanguagePair::from_codes("zh", "en").unwrap();

        let names: Vec<String> = discover_source_files(p, &pair)
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["ep01.ass", "ep02.ass", "ep03.ass"]);

        let state = |n: &str| output_on_disk(&p.join(n), &pair).state;
        assert_eq!(state("ep01.ass"), TranslationState::Done);
        assert_eq!(state("ep03.ass"), TranslationState::NeedsLook);
        assert_eq!(state("ep02.ass"), TranslationState::None);
    }

    #[test]
    fn lone_target_tagged_file_is_a_source() {
        // An English source opened under the default zh→en pair: `.en` matches
        // the target suffix, but with no sibling source it isn't an output.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        for name in [
            "movie.en.ass",
            "movie.mkv",
            "ep01.ass",
            "ep01.eng.ass",
            "ep02.chi.ass",
            "ep02.en.ass",
        ] {
            fs::write(p.join(name), "x").unwrap();
        }
        let pair = LanguagePair::from_codes("zh", "en").unwrap();

        let names: Vec<String> = discover_source_files(p, &pair)
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["ep01.ass", "ep02.chi.ass", "movie.en.ass"]);
    }

    #[test]
    fn output_on_disk_reads_clean_warning_and_both() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        for name in [
            "ep01.ass",
            "ep01.eng.ass",
            "ep02.ass",
            "ep02.warning.eng.ass",
            "ep03.ass",
            "ep04.ass",
            "ep04.eng.ass",
            "ep04.warning.eng.ass",
        ] {
            fs::write(p.join(name), "x").unwrap();
        }
        let pair = LanguagePair::from_codes("zh", "en").unwrap();

        let done = output_on_disk(&p.join("ep01.ass"), &pair);
        assert_eq!(done.state, TranslationState::Done);
        assert!(!done.both_variants);
        assert_eq!(done.path, Some(p.join("ep01.eng.ass")));

        let look = output_on_disk(&p.join("ep02.ass"), &pair);
        assert_eq!(look.state, TranslationState::NeedsLook);
        assert!(!look.both_variants);
        assert_eq!(look.path, Some(p.join("ep02.warning.eng.ass")));

        let none = output_on_disk(&p.join("ep03.ass"), &pair);
        assert_eq!(none.state, TranslationState::None);
        assert_eq!(none.path, None);

        // Clean wins; the warning file is an old leftover.
        let both = output_on_disk(&p.join("ep04.ass"), &pair);
        assert_eq!(both.state, TranslationState::Done);
        assert!(both.both_variants);
        assert_eq!(both.path, Some(p.join("ep04.eng.ass")));
    }

    #[test]
    fn a_folder_named_like_an_output_is_not_one() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        fs::write(p.join("ep01.ass"), "x").unwrap();
        fs::create_dir(p.join("ep01.eng.ass")).unwrap();
        let pair = LanguagePair::from_codes("zh", "en").unwrap();
        assert_eq!(output_on_disk(&p.join("ep01.ass"), &pair).state, TranslationState::None);
    }
}
