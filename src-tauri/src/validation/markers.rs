//! Line markers `<NNNN:T>` injected into source text so the LLM's output can be
//! aligned line-by-line, with fuzzy recovery for mangled markers.
//! Port of `validation/line_marker.py` + `marker_result.py`.

use std::sync::LazyLock;

use regex::Regex;

use super::LinePair;

/// Ids are zero-padded to four digits and grow past that: a typeset episode
/// can have more than 9,999 lines, and its ids are positions in the file.
static EXACT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<(\d{4,}):[DL]>").unwrap());
static FUZZY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<\s*(\d+)\s*:\s*[DL]\s*>").unwrap());
static RANGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<(\d+)-(\d+):[DL]>").unwrap());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Dialogue,
    Label,
}

impl LineKind {
    fn tag(self) -> char {
        match self {
            LineKind::Dialogue => 'D',
            LineKind::Label => 'L',
        }
    }
}

/// `<0001:D> text` (`<12001:D>` past 9,999).
pub fn inject(id: u32, kind: LineKind, text: &str) -> String {
    format!("<{:04}:{}> {}", id, kind.tag(), text)
}

/// Remove exact then fuzzy markers; collapse whitespace runs and trim both ends.
/// Mirrors Python: PATTERN.sub + FUZZY.sub + re.sub(r"\s+", " ") + .strip()
pub fn strip(text: &str) -> String {
    let s = EXACT.replace_all(text, "");
    let s = FUZZY.replace_all(&s, "");
    // Collapse all whitespace runs (spaces, tabs, newlines) to a single space,
    // then trim both ends — matching Python's re.sub(r"\s+", " ", ...).strip().
    static WS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
    WS.replace_all(&s, " ").trim().to_string()
}

/// IDs of exact markers found in `text`, in order.
pub fn extract_all(text: &str) -> Vec<u32> {
    EXACT
        .captures_iter(text)
        .filter_map(|c| c[1].parse().ok())
        .collect()
}

#[derive(Debug, Clone, Default)]
pub struct MarkerCheck {
    pub is_valid: bool,
    pub first_mismatch_id: Option<u32>,
    pub missing_markers: Vec<u32>,
    pub extra_markers: Vec<u32>,
    /// Output-line ids that carry more than one marker (merged lines).
    pub duplicate_markers: Vec<u32>,
    /// (line id, offending text) for fuzzy-but-not-exact or range markers.
    pub corrupted_markers: Vec<(u32, String)>,
    pub order_mismatches: Vec<u32>,
}

/// Validate the markers the model echoed back in its `tgt` fields against the
/// ids we sent. Mirrors `LineMarker.validate` (`line_marker.py:70-162`).
pub fn check(expected_ids: &[u32], output: &[LinePair]) -> MarkerCheck {
    let mut r = MarkerCheck { is_valid: true, ..Default::default() };
    let mut found: Vec<u32> = Vec::new();

    for pair in output {
        let exact = extract_all(&pair.tgt);
        // Corrupted: fuzzy matches that aren't exact matches, or range markers.
        for m in FUZZY.find_iter(&pair.tgt) {
            if !EXACT.is_match(m.as_str()) {
                r.corrupted_markers.push((pair.id, m.as_str().to_string()));
            }
        }
        for m in RANGE.find_iter(&pair.tgt) {
            r.corrupted_markers.push((pair.id, m.as_str().to_string()));
        }
        if exact.len() > 1 {
            r.duplicate_markers.push(pair.id);
        }
        found.extend(exact);
    }

    let expected: Vec<u32> = expected_ids.to_vec();
    r.missing_markers = expected.iter().copied().filter(|id| !found.contains(id)).collect();
    r.extra_markers = found.iter().copied().filter(|id| !expected.contains(id)).collect();

    // Order: dedup found, compare against expected filtered to found ids.
    let mut found_dedup: Vec<u32> = Vec::new();
    for id in &found {
        if !found_dedup.contains(id) {
            found_dedup.push(*id);
        }
    }
    let expected_present: Vec<u32> =
        expected.iter().copied().filter(|id| found_dedup.contains(id)).collect();
    if found_dedup != expected_present {
        for (a, b) in found_dedup.iter().zip(expected_present.iter()) {
            if a != b {
                // Record the EXPECTED id at this position (mirrors Python line 142:
                // `order_mismatches.append(expected_order[i])`).
                r.order_mismatches.push(*b);
            }
        }
    }

    if !r.missing_markers.is_empty()
        || !r.extra_markers.is_empty()
        || !r.duplicate_markers.is_empty()
        || !r.corrupted_markers.is_empty()
        || !r.order_mismatches.is_empty()
    {
        r.is_valid = false;
        // First problem id: smallest id implicated anywhere.
        r.first_mismatch_id = r
            .missing_markers
            .iter()
            .chain(r.extra_markers.iter())
            .chain(r.duplicate_markers.iter())
            .chain(r.corrupted_markers.iter().map(|(id, _)| id))
            .chain(r.order_mismatches.iter())
            .copied()
            .min();
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_padded_markers() {
        assert_eq!(inject(1, LineKind::Dialogue, "你好"), "<0001:D> 你好");
        assert_eq!(inject(42, LineKind::Label, "第一集"), "<0042:L> 第一集");
    }

    #[test]
    fn strips_exact_and_fuzzy_markers() {
        assert_eq!(strip("<0001:D> hello"), "hello");
        assert_eq!(strip("< 1 : d > hello"), "hello"); // fuzzy + case-insensitive
        assert_eq!(strip("no markers"), "no markers");
        // Inline marker: collapses double-space left behind + trims nothing extra.
        assert_eq!(strip("Some text <0001:D> rest"), "Some text rest");
        // Leading marker + trailing space: collapse + trim both ends.
        assert_eq!(strip("<0001:D> hello "), "hello");
    }

    fn pair(id: u32, tgt: &str) -> crate::validation::LinePair {
        crate::validation::LinePair { id, src: String::new(), tgt: tgt.into() }
    }

    #[test]
    fn valid_when_all_markers_match_in_order() {
        let out = vec![pair(1, "<0001:D> a"), pair(2, "<0002:D> b")];
        let r = check(&[1, 2], &out);
        assert!(r.is_valid);
        assert_eq!(r.first_mismatch_id, None);
    }

    #[test]
    fn detects_missing_and_extra() {
        let out = vec![pair(1, "<0001:D> a"), pair(3, "<0003:D> c")];
        let r = check(&[1, 2], &out);
        assert!(!r.is_valid);
        assert_eq!(r.missing_markers, vec![2]);
        assert_eq!(r.extra_markers, vec![3]);
        assert_eq!(r.first_mismatch_id, Some(2));
    }

    #[test]
    fn detects_merged_lines_via_duplicate_markers_on_one_line() {
        let out = vec![pair(1, "<0001:D> a <0002:D> b")];
        let r = check(&[1, 2], &out);
        assert!(!r.is_valid);
        assert_eq!(r.duplicate_markers, vec![1]); // line id 1 carries 2 markers
    }

    #[test]
    fn detects_range_markers_as_corrupted() {
        let out = vec![pair(1, "<0001-0002:D> merged")];
        let r = check(&[1, 2], &out);
        assert!(!r.is_valid);
        assert!(!r.corrupted_markers.is_empty());
        assert_eq!(r.corrupted_markers[0].0, 1);
        assert!(r.corrupted_markers[0].1.contains("0001-0002"));
    }

    #[test]
    fn detects_order_mismatch() {
        // LLM returned markers in reverse order: found=[2,1], expected=[1,2].
        // Both positions differ; we record the EXPECTED id at each mismatch position.
        // Position 0: found=2, expected=1 → push 1
        // Position 1: found=1, expected=2 → push 2
        let out = vec![pair(1, "<0002:D> b"), pair(2, "<0001:D> a")];
        let r = check(&[1, 2], &out);
        assert!(!r.is_valid);
        assert_eq!(r.order_mismatches, vec![1, 2]);
    }

    #[test]
    fn fuzzy_marker_counts_as_corrupted_not_exact() {
        let out = vec![pair(1, "< 1 : D > a"), pair(2, "<0002:D> b")];
        let r = check(&[1, 2], &out);
        assert!(!r.is_valid);
        assert_eq!(r.corrupted_markers.len(), 1);
        assert_eq!(r.first_mismatch_id, Some(1));
    }

    #[test]
    fn ids_past_four_digits_round_trip() {
        let marked = inject(12001, LineKind::Dialogue, "Hello");
        assert_eq!(marked, "<12001:D> Hello");
        assert_eq!(extract_all(&marked), vec![12001]);
        assert_eq!(strip(&marked), "Hello");
    }
}
