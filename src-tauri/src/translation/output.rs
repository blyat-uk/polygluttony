//! An episode's translated file next to its source. It is written whole (a
//! temp file, then a rename) as one variant at a time: the clean name or the
//! `.warning` one, never both. Each write gives a fingerprint of its bytes,
//! so Review can tell when someone changed the file since.

use std::path::Path;

use crate::ass::parse::DialogueLine;
use crate::ass::writer::render_translated;
use crate::error::{AppError, AppResult};
use crate::models::language_pair::{output_filename, warning_filename, LanguagePair};
use crate::translation::results::{fingerprint_bytes, write_atomic};

/// The other variant of output `name` for `source`: the `.warning` name for
/// the clean one and back. `None` for a name that is neither.
pub fn other_variant(source: &Path, pair: &LanguagePair, name: &str) -> Option<String> {
    let clean = output_filename(source, pair);
    let warning = warning_filename(source, pair);
    if name == clean {
        Some(warning)
    } else if name == warning {
        Some(clean)
    } else {
        None
    }
}

/// Write output `name` next to `source` from the decoded original and the
/// translated lines, then remove the other variant, so a stale one can't
/// stand in for this. Removing is best effort: a leftover shows as both
/// variants, and the newer file counts. Returns the written bytes' fingerprint.
pub fn write(
    source: &Path,
    pair: &LanguagePair,
    name: &str,
    original: &str,
    lines: &[DialogueLine],
) -> AppResult<String> {
    let dir = source.parent().unwrap_or_else(|| Path::new(""));
    let text = render_translated(original, lines);
    write_atomic(&dir.join(name), text.as_bytes())?;
    if let Some(other) = other_variant(source, pair, name) {
        match std::fs::remove_file(dir.join(&other)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                tracing::warn!("couldn't remove the old {other}: {e}")
            }
            _ => {}
        }
    }
    Ok(fingerprint_bytes(text.as_bytes()))
}

/// Why Review won't rewrite output `name`.
pub fn changed_outside(name: &str) -> String {
    format!(
        "{name} was changed outside Polygluttony since it was written. Open it in your editor, \
         or translate the episode again."
    )
}

/// Refuse to rewrite output `name` in `dir` when it isn't what Polygluttony
/// last wrote there (`written`, its fingerprint). Results stored before
/// fingerprints were kept have none: those are rewritten as before. A file
/// that's gone has nothing to lose.
pub fn check_unchanged(dir: &Path, name: &str, written: Option<&str>) -> AppResult<()> {
    let Some(written) = written else { return Ok(()) };
    match std::fs::read(dir.join(name)) {
        Ok(bytes) if fingerprint_bytes(&bytes) == written => Ok(()),
        Ok(_) => Err(AppError::Other(changed_outside(name))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(AppError::Other(format!("Couldn't read {name}: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "[Events]\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,你好\n";

    fn lines(text: &str) -> Vec<DialogueLine> {
        crate::ass::parse::parse_dialogues(SRC)
            .into_iter()
            .map(|d| DialogueLine { text: text.into(), ..d })
            .collect()
    }

    #[test]
    fn writing_one_variant_removes_the_other() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("ep01.ass");
        let pair = LanguagePair::from_codes("zh", "en").unwrap();
        let (clean, warning) = (dir.path().join("ep01.eng.ass"), dir.path().join("ep01.warning.eng.ass"));

        write(&source, &pair, "ep01.warning.eng.ass", SRC, &lines("Hi")).unwrap();
        assert!(warning.exists() && !clean.exists());
        write(&source, &pair, "ep01.eng.ass", SRC, &lines("Hello")).unwrap();
        assert!(clean.exists() && !warning.exists(), "the clean file replaces the warning one");
        write(&source, &pair, "ep01.warning.eng.ass", SRC, &lines("Hello?")).unwrap();
        assert!(warning.exists() && !clean.exists(), "and back");
        assert_eq!(other_variant(&source, &pair, "notes.ass"), None);
    }

    #[test]
    fn a_changed_output_is_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("ep01.ass");
        let pair = LanguagePair::from_codes("zh", "en").unwrap();
        let name = "ep01.eng.ass";
        let written = write(&source, &pair, name, SRC, &lines("Hi")).unwrap();
        assert!(check_unchanged(dir.path(), name, Some(&written)).is_ok());
        // No fingerprint (stored by an older version): rewritten as before.
        std::fs::write(dir.path().join(name), "fixed by hand").unwrap();
        assert!(check_unchanged(dir.path(), name, None).is_ok());
        let err = check_unchanged(dir.path(), name, Some(&written)).unwrap_err().to_string();
        assert_eq!(
            err,
            "ep01.eng.ass was changed outside Polygluttony since it was written. Open it in your \
             editor, or translate the episode again."
        );
        std::fs::remove_file(dir.path().join(name)).unwrap();
        assert!(check_unchanged(dir.path(), name, Some(&written)).is_ok(), "nothing there to lose");
    }
}
