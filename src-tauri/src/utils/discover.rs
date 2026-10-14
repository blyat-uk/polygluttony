//! Discover source `.ass` files in a folder and detect existing translations.
//! Ported from `utils/file_utils.py`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::models::language_pair::{
    output_filename, strip_language_suffix, warning_filename, LanguagePair,
};

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

/// Whether `source` already has a translation (output or warning file present).
pub fn has_existing_translation(source: &Path, pair: &LanguagePair) -> bool {
    let dir = source.parent().unwrap_or_else(|| Path::new("."));
    dir.join(output_filename(source, pair)).exists()
        || dir.join(warning_filename(source, pair)).exists()
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

        assert!(has_existing_translation(&p.join("ep01.ass"), &pair));
        assert!(has_existing_translation(&p.join("ep03.ass"), &pair));
        assert!(!has_existing_translation(&p.join("ep02.ass"), &pair));
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
}
