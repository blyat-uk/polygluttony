//! Strip ASS inline override tags (`{\pos(..)}`, `{\an8}`, `{\i1}`) from dialogue
//! text, leaving plain words for language + world detection. (The positional
//! strip/reapply needed for translation output is a later step.)

use std::sync::LazyLock;

use regex::Regex;

static TAG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{[^}]*\}").unwrap());

/// Remove all `{...}` override blocks, returning the plain text.
pub fn strip_for_text(s: &str) -> String {
    TAG_RE.replace_all(s, "").into_owned()
}

/// Matches only ASS override tag blocks: `{` followed by `\` then non-`}` chars then `}`.
/// Plain brace groups like `{note}` do not match (`ass_tags.py:68`).
/// Note: positions are tracked in CHARS (not bytes) unlike Python's `original_position`
/// which is a code-point (character) offset from `match.start()`. Both yield the same result for stripped_position
/// because Python's `_calculate_stripped_position` also slices by chars via str indexing.
static TAG: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\{\\[^}]+\}").unwrap());

/// One override tag with where it sat in the original and stripped text.
/// Positions are in CHARS (not bytes) — translations are sliced by chars.
/// (`ass_tags.py` `AssTag` dataclass, lines 7-18)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedTag {
    pub content: String,
    /// Char offset of the tag in the original text.
    pub original_position: usize,
    /// Char offset where the tag would sit in the stripped text (all preceding tags removed).
    pub stripped_position: usize,
}

/// Result of a positional strip: original text, stripped text, and tags with positions.
/// (`ass_tags.py` `AssTagResult` dataclass, lines 25-37)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagStrip {
    pub original: String,
    pub stripped: String,
    pub tags: Vec<PlacedTag>,
}

/// Strip `{\...}` override blocks recording char positions (`ass_tags.py:70-107`).
///
/// Unlike Python's `_calculate_stripped_position` which re-strips the prefix each iteration,
/// we maintain a running `removed_chars` counter — identical result, O(n) vs O(n²).
pub fn strip_positional(text: &str) -> TagStrip {
    let mut tags = Vec::new();
    let mut removed_chars = 0usize;
    for m in TAG.find_iter(text) {
        let original_position = text[..m.start()].chars().count();
        tags.push(PlacedTag {
            content: m.as_str().to_string(),
            original_position,
            stripped_position: original_position - removed_chars,
        });
        removed_chars += m.as_str().chars().count();
    }
    TagStrip {
        original: text.to_string(),
        stripped: TAG.replace_all(text, "").to_string(),
        tags,
    }
}

/// Reinsert tags into `translated`: position 0 → prepend, ≥ stripped end →
/// append, mid-text → proportional (`ass_tags.py:109-150`).
///
/// `already_inserted` tracks cumulative char count of tags already spliced in,
/// mirroring Python's `inserted_length` (`ass_tags.py:137`).
pub fn reapply(strip: &TagStrip, translated: &str) -> String {
    if strip.tags.is_empty() {
        return translated.to_string();
    }
    let original_len = strip.stripped.chars().count();
    if original_len == 0 {
        // All tags prefix the translation (`ass_tags.py:131-133`).
        let prefix: String = strip.tags.iter().map(|t| t.content.as_str()).collect();
        return format!("{prefix}{translated}");
    }
    let translated_len = translated.chars().count();
    let mut out: Vec<char> = translated.chars().collect();
    let mut already_inserted = 0usize;
    for tag in &strip.tags {
        // Mirror Python's _calculate_insert_position (`ass_tags.py:189-219`).
        let insert_pos = if tag.stripped_position == 0 {
            // Tags at start: place right after any previously-inserted prefix tags.
            // Python: `return already_inserted` (`ass_tags.py:208-209`).
            already_inserted.min(out.len())
        } else if tag.stripped_position >= original_len {
            // Tags at or beyond end: append after all real translated chars + inserted tags.
            // Python: `return translated_length + already_inserted` (`ass_tags.py:212-213`).
            out.len()
        } else {
            // Proportional positioning (`ass_tags.py:216-219`).
            let ratio = tag.stripped_position as f64 / original_len as f64;
            let calc = (ratio * translated_len as f64).round() as usize;
            (calc + already_inserted).min(out.len())
        };
        let tag_chars: Vec<char> = tag.content.chars().collect();
        already_inserted += tag_chars.len();
        out.splice(insert_pos..insert_pos, tag_chars);
    }
    out.into_iter().collect()
}

/// The overrides in one `{...}` block, each with its leading `\`
/// (`{\an8\fad(200,0)}` → `\an8`, `\fad(200,0)`). A `\` inside
/// parentheses belongs to its tag (`\t(0,500,\fs30)` is one).
fn overrides(block: &str) -> Vec<&str> {
    let inner = block.strip_prefix('{').and_then(|b| b.strip_suffix('}')).unwrap_or(block);
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    for (i, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            '\\' if depth == 0 => {
                if let Some(s) = start {
                    out.push(&inner[s..i]);
                }
                start = Some(i);
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push(&inner[s..]);
    }
    out
}

/// What an override sets, so two spellings of one setting match: `\an8`
/// and `\a5` both align the line, `\pos` and `\move` both place it,
/// `\fad` and `\fade` both fade it, `\c` is `\1c`.
fn setting(tag: &str) -> String {
    let body = tag.trim_start_matches('\\');
    let digits: String = body.chars().take_while(|c| c.is_ascii_digit()).collect();
    let letters: String =
        body[digits.len()..].chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let name = format!("{digits}{letters}").to_ascii_lowercase();
    match name.as_str() {
        "a" | "an" => "align".into(),
        "pos" | "move" => "position".into(),
        "fad" | "fade" => "fade".into(),
        "clip" | "iclip" => "clip".into(),
        "c" => "1c".into(),
        _ => name,
    }
}

/// The user's own text for a line that may hold tags they typed, with the
/// source line's tags put back where the typed text doesn't set the same
/// thing: a hidden `\an8`, `\pos`, `\fad` or `\move` (or any other
/// override) survives the edit, a typed `\an2` wins over the source's
/// `\an8`. Source tags go where [`reapply`] would put them on the typed
/// words; typed tags stay where they were typed, after source ones at the
/// same spot.
pub fn reapply_missing(source: &TagStrip, typed: &str) -> String {
    let typed_strip = strip_positional(typed);
    let typed_settings: std::collections::HashSet<String> = typed_strip
        .tags
        .iter()
        .flat_map(|t| overrides(&t.content))
        .map(setting)
        .collect();
    let missing: Vec<PlacedTag> = source
        .tags
        .iter()
        .filter_map(|t| {
            let kept: String = overrides(&t.content)
                .into_iter()
                .filter(|o| !typed_settings.contains(&setting(o)))
                .collect();
            (!kept.is_empty()).then(|| PlacedTag { content: format!("{{{kept}}}"), ..t.clone() })
        })
        .collect();
    if missing.is_empty() {
        return typed.to_string();
    }
    let missing = TagStrip { tags: missing, ..source.clone() };
    // Source tags placed on the typed words, then the typed tags back at
    // their own spots.
    let placed = reapply(&missing, &typed_strip.stripped);
    let mut typed_tags = typed_strip.tags.iter().peekable();
    let mut out = String::with_capacity(placed.len() + typed.len());
    let mut plain = 0usize;
    let mut last = 0usize;
    let mut text = |out: &mut String, from: &str, plain: &mut usize| {
        for ch in from.chars() {
            while let Some(t) = typed_tags.next_if(|t| t.stripped_position <= *plain) {
                out.push_str(&t.content);
            }
            out.push(ch);
            *plain += 1;
        }
    };
    for m in TAG.find_iter(&placed) {
        text(&mut out, &placed[last..m.start()], &mut plain);
        out.push_str(m.as_str());
        last = m.end();
    }
    text(&mut out, &placed[last..], &mut plain);
    for t in typed_tags {
        out.push_str(&t.content);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_split_a_block_and_keep_transform_arguments_whole() {
        assert_eq!(overrides(r"{\an8\fad(200,0)}"), [r"\an8", r"\fad(200,0)"]);
        assert_eq!(overrides(r"{\t(0,500,\fs30)\i1}"), [r"\t(0,500,\fs30)", r"\i1"]);
        assert_eq!(setting(r"\an8"), setting(r"\a5"));
        assert_eq!(setting(r"\pos(1,2)"), setting(r"\move(1,2,3,4)"));
        assert_eq!(setting(r"\1c&H00FFFF&"), setting(r"\c&HFFFFFF&"));
        assert_ne!(setting(r"\i1"), setting(r"\an8"));
    }

    #[test]
    fn typed_tags_keep_the_sources_hidden_ones() {
        let src = strip_positional(r"{\an8\fad(200,200)}叶长生不会走的");
        // Typed styling: the hidden positioning comes back in front of it.
        assert_eq!(
            reapply_missing(&src, r"{\i1}Ye Changsheng{\i0} won't leave."),
            r"{\an8\fad(200,200)}{\i1}Ye Changsheng{\i0} won't leave."
        );
        // A typed alignment wins; the fade is still put back.
        assert_eq!(
            reapply_missing(&src, r"{\an2}Ye Changsheng won't leave."),
            r"{\fad(200,200)}{\an2}Ye Changsheng won't leave."
        );
        // Typed text that sets everything the source did stays as typed.
        assert_eq!(
            reapply_missing(&src, r"{\an7\fade(0,100)}Hi"),
            r"{\an7\fade(0,100)}Hi"
        );
        // Placement and positions: \move counts as positioning.
        let sign = strip_positional(r"{\pos(320,40)}青阳镇");
        assert_eq!(
            reapply_missing(&sign, r"{\move(0,0,320,40)}Qingyang Town"),
            r"{\move(0,0,320,40)}Qingyang Town"
        );
        // Mid-line source tags go where `reapply` puts them; typed ones stay put.
        let mid = strip_positional(r"你好{\c&H00FFFF&}世界");
        let out = reapply_missing(&mid, r"Hello {\b1}world{\b0}");
        assert_eq!(strip_for_text(&out), "Hello world");
        assert!(out.contains(r"{\c&H00FFFF&}") && out.contains(r"{\b1}world{\b0}"), "{out}");
        // No source tags: as typed.
        assert_eq!(reapply_missing(&strip_positional("你好"), r"{\i1}Hi"), r"{\i1}Hi");
    }

    #[test]
    fn strips_override_blocks() {
        assert_eq!(strip_for_text("{\\pos(1,2)\\an5}Episode Title"), "Episode Title");
        assert_eq!(strip_for_text("{\\i1}斜体{\\i0}文字"), "斜体文字");
        assert_eq!(strip_for_text("plain text"), "plain text");
    }

    #[test]
    fn strip_records_positions() {
        let r = strip_positional(r"{\pos(100,200)}Episode Title");
        assert_eq!(r.stripped, "Episode Title");
        assert_eq!(r.tags.len(), 1);
        assert_eq!(r.tags[0].content, r"{\pos(100,200)}");
        assert_eq!(r.tags[0].stripped_position, 0);
    }

    #[test]
    fn strip_mid_text_tag_position() {
        let r = strip_positional(r"Hello {\i1}world");
        assert_eq!(r.stripped, "Hello world");
        assert_eq!(r.tags[0].stripped_position, 6);
    }

    #[test]
    fn reapply_prefix_tag() {
        let r = strip_positional(r"{\an8}你好");
        assert_eq!(reapply(&r, "Hello"), r"{\an8}Hello");
    }

    #[test]
    fn reapply_mid_text_is_proportional() {
        // Tag at 5/10 chars of the stripped source → middle of the translation.
        let r = strip_positional(r"01234{\i1}56789");
        assert_eq!(r.tags[0].stripped_position, 5);
        assert_eq!(reapply(&r, "abcdefgh"), r"abcd{\i1}efgh");
    }

    #[test]
    fn reapply_end_tag_appends() {
        let r = strip_positional(r"你好{\r}");
        assert_eq!(reapply(&r, "Hello"), r"Hello{\r}");
    }

    #[test]
    fn reapply_on_empty_stripped_text_prefixes_all_tags() {
        let r = strip_positional(r"{\pos(1,2)}{\an8}");
        assert_eq!(r.stripped, "");
        assert_eq!(reapply(&r, "Hi"), r"{\pos(1,2)}{\an8}Hi");
    }

    #[test]
    fn non_override_braces_are_not_tags() {
        let r = strip_positional("{note} hello");
        assert!(r.tags.is_empty());
        assert_eq!(r.stripped, "{note} hello");
    }

    #[test]
    fn no_tags_roundtrip() {
        let r = strip_positional("plain");
        assert_eq!(reapply(&r, "translated"), "translated");
    }

    #[test]
    fn reapply_two_tags_accumulates_offsets() {
        let r = strip_positional(r"{\an8}ab{\i1}cd");
        assert_eq!(r.stripped, "abcd");
        assert_eq!(reapply(&r, "WXYZ"), r"{\an8}WX{\i1}YZ");
    }
}
