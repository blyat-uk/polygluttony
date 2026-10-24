//! Translated-file writer: header verbatim up to [Events] minus dropped
//! sections, credit comment under [Script Info], regenerated [Events] sorted
//! by start time, UTF-8 + BOM.

use crate::ass::parse::DialogueLine;

const CREDIT: &str = "; Translated at home with Polygluttony";
const DROPPED_SECTIONS: [&str; 3] = ["[aegisub project garbage]", "[fonts]", "[graphics]"];
const EVENTS_FORMAT: &str =
    "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text";

/// Build the full output text from the decoded original + translated lines.
pub fn render_translated(original: &str, translated: &[DialogueLine]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_dropped = false;

    for line in original.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();
        if lower == "[events]" {
            break; // header done — we regenerate everything from here
        }
        if trimmed.starts_with('[') {
            in_dropped = DROPPED_SECTIONS.contains(&lower.as_str());
        }
        if in_dropped {
            continue;
        }
        out.push(line.to_string());
        if lower == "[script info]" {
            out.push(CREDIT.to_string());
        }
    }

    // Trim trailing blank lines left by a dropped section, keep exactly one.
    while out.last().map(|l| l.trim().is_empty()) == Some(true) {
        out.pop();
    }
    out.push(String::new());
    out.push("[Events]".to_string());
    out.push(EVENTS_FORMAT.to_string());

    let mut sorted: Vec<&DialogueLine> = translated.iter().collect();
    sorted.sort_by_key(|d| d.start_cs);
    out.extend(sorted.iter().map(|d| d.render()));

    format!("\u{FEFF}{}\n", out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ass::parse::DialogueLine;

    const SOURCE: &str = "[Script Info]\n\
Title: 第一集\n\
ScriptType: v4.00+\n\
\n\
[Aegisub Project Garbage]\n\
Audio File: ep.mkv\n\
\n\
[V4+ Styles]\n\
Format: Name, Fontname\n\
Style: Default,Arial\n\
\n\
[Fonts]\n\
fontname: foo.ttf\n\
\n\
[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:05.00,0:00:06.00,Default,,0,0,0,,后来的\n\
Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,你好\n\
Comment: 0,0:00:00.00,0:00:00.01,Default,,0,0,0,,note\n";

    fn line(start_cs: i64, text: &str) -> DialogueLine {
        DialogueLine {
            layer: 0,
            start_cs,
            end_cs: start_cs + 100,
            style: "Default".into(),
            name: "".into(),
            margin_l: 0,
            margin_r: 0,
            margin_v: 0,
            effect: "".into(),
            text: text.into(),
        }
    }

    #[test]
    fn writes_header_credit_and_sorted_events() {
        let out = render_translated(SOURCE, &[line(500, "Later on"), line(100, "Hello")]);
        // BOM present exactly once, at the front.
        assert!(out.starts_with('\u{FEFF}'));
        let body = out.trim_start_matches('\u{FEFF}');
        // Credit injected under [Script Info].
        assert!(body.starts_with("[Script Info]\n; Translated at home with Polygluttony\n"));
        // Garbage + Fonts sections dropped, Styles kept verbatim.
        assert!(!body.contains("[Aegisub Project Garbage]"));
        assert!(!body.contains("fontname"));
        assert!(body.contains("Style: Default,Arial"));
        // Events regenerated: sorted by start, comments dropped.
        let hello = body.find("Hello").unwrap();
        let later = body.find("Later on").unwrap();
        assert!(hello < later);
        assert!(!body.contains("Comment:"));
        assert!(body.contains(
            "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text"
        ));
    }

    #[test]
    fn handles_missing_optional_sections() {
        let minimal = "[Script Info]\nTitle: x\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n";
        let out = render_translated(minimal, &[line(100, "Hi")]);
        assert!(out.contains("; Translated at home with Polygluttony"));
        assert!(out.contains("Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,Hi"));
    }

    #[test]
    fn gbk_encoded_source_roundtrips_through_decode() {
        // The writer consumes decoded text — prove the chain works for a GBK
        // input file end-to-end.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gbk.ass");
        let src = "[Script Info]\nTitle: 第一集\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n";
        let (encoded, _, _) = encoding_rs::GBK.encode(src);
        std::fs::write(&path, &encoded).unwrap();
        let decoded = crate::ass::decode::decode_file(&path).unwrap();
        let out = render_translated(&decoded, &[line(100, "Hello")]);
        assert!(out.contains("Title: 第一集")); // CJK header survived GBK decode
        assert!(out.contains("Hello"));
    }
}
