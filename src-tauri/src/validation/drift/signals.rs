//! The five drift signals. Exact ports of `validation/signals/*.py` — the odd
//! denominators (`checks * 0.3` etc.) are calibrated against real LLM failures;
//! do not "fix" them.

use std::collections::{BTreeMap, BTreeSet};

use crate::validation::LinePair;

pub type Signal = (f64, Option<u32>, BTreeSet<u32>);

fn char_len(s: &str) -> usize {
    s.chars().count()
}

// ── punctuation helpers ──────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
enum PunctType {
    Question,
    Exclamation,
    Statement,
    Ellipsis,
}

/// Detect Chinese sentence-ending punctuation (`punctuation.py:_get_chinese_punctuation_ending`).
fn chinese_punct(text: &str) -> Option<PunctType> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    match t.chars().last()? {
        '？' | '?' => Some(PunctType::Question),
        '！' | '!' => Some(PunctType::Exclamation),
        '。' | '.' => Some(PunctType::Statement),
        '…' => Some(PunctType::Ellipsis),
        _ => None,
    }
}

/// Detect English sentence-ending punctuation (`punctuation.py:_get_english_punctuation_ending`).
/// Handles "?!" / "!?" combos.
fn english_punct(text: &str) -> Option<PunctType> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    // Two-char combo check — allocation-free via char iterator
    let mut rev = t.chars().rev();
    let (a, b) = (rev.next(), rev.next());
    if matches!((b, a), (Some('?'), Some('!')) | (Some('!'), Some('?'))) {
        return Some(PunctType::Question);
    }
    match t.chars().last()? {
        '?' => Some(PunctType::Question),
        '!' => Some(PunctType::Exclamation),
        '.' => Some(PunctType::Statement),
        _ => None,
    }
}

/// Punctuation transformation signal (`punctuation.py`).
///
/// Violations:
/// - Chinese question → English non-question
/// - Chinese exclamation → English non-exclamation
/// - Chinese statement → English question
///
/// Score = min(1.0, violations / max(1, checks * 0.3))
pub fn punctuation(pairs: &[LinePair]) -> Signal {
    let mut violations = 0usize;
    let mut checks = 0usize;
    let mut first: Option<u32> = None;
    let mut flagged = BTreeSet::new();

    for p in pairs {
        let Some(chinese_ending) = chinese_punct(&p.src) else {
            continue; // No significant punctuation to check
        };
        checks += 1;

        let english_ending = english_punct(&p.tgt);

        let is_violation = match chinese_ending {
            PunctType::Question => english_ending != Some(PunctType::Question),
            PunctType::Exclamation => english_ending != Some(PunctType::Exclamation),
            PunctType::Statement => english_ending == Some(PunctType::Question),
            PunctType::Ellipsis => false,
        };

        if is_violation {
            violations += 1;
            first.get_or_insert(p.id);
            flagged.insert(p.id);
        }
    }

    if checks == 0 {
        return (0.0, None, BTreeSet::new());
    }

    let score = (violations as f64 / (1.0f64).max(checks as f64 * 0.3)).min(1.0);
    (score, first, flagged)
}

// ── glossary position ────────────────────────────────────────────────────────

/// Glossary terms present in src must have their translation in tgt
/// (case-insensitive) (`glossary_position.py`).
///
/// Score = min(1.0, violations / max(1, checks * 0.5))
pub fn glossary_position(pairs: &[LinePair], terms: &BTreeMap<String, String>) -> Signal {
    let (checks, misses) = glossary_checks(pairs, terms);
    if checks == 0 {
        return (0.0, None, BTreeSet::new());
    }
    let violations = misses.len();
    let first = misses.first().map(|m| m.line_id);
    let flagged: BTreeSet<u32> = misses.iter().map(|m| m.line_id).collect();

    let score = (violations as f64 / (1.0f64).max(checks as f64 * 0.5)).min(1.0);
    (score, first, flagged)
}

/// A glossary term whose source is in a line but whose translation isn't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlossaryMiss {
    pub line_id: u32,
    pub term: String,
    pub expected: String,
}

/// Every glossary miss, in line order (terms in glossary order within a
/// line): exactly the violations [`glossary_position`] scores.
pub fn glossary_misses(pairs: &[LinePair], terms: &BTreeMap<String, String>) -> Vec<GlossaryMiss> {
    glossary_checks(pairs, terms).1
}

/// (checks made, misses found) — the one walk both the score and the
/// per-line name issues come from, so they can never disagree.
fn glossary_checks(
    pairs: &[LinePair],
    terms: &BTreeMap<String, String>,
) -> (usize, Vec<GlossaryMiss>) {
    if terms.is_empty() {
        return (0, Vec::new());
    }

    // Precompute lowercased translations once — avoids repeated allocation inside the inner loop.
    let terms_lower: Vec<(&String, &String, String)> = terms
        .iter()
        .map(|(term, tr)| (term, tr, tr.to_lowercase()))
        .collect();

    let mut checks = 0usize;
    let mut misses = Vec::new();

    for p in pairs {
        let tgt_lower = p.tgt.to_lowercase();
        for (term, tr, tr_lower) in &terms_lower {
            if p.src.contains(term.as_str()) {
                checks += 1;
                if !tgt_lower.contains(tr_lower.as_str()) {
                    misses.push(GlossaryMiss {
                        line_id: p.id,
                        term: (*term).clone(),
                        expected: (*tr).clone(),
                    });
                }
            }
        }
    }
    (checks, misses)
}

// ── sentence type ────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Clone, Copy)]
enum SentenceType {
    Dialogue,
    Question,
    Exclamation,
    Statement,
}

/// Check for dialogue markers (`sentence_type.py:_has_dialogue_markers`).
///
/// Chinese markers: `"`, `"`, `「`, `」`, `『`, `』`
/// English: two or more `"` characters.
fn has_dialogue_markers(text: &str) -> bool {
    let chinese_markers = ['"', '"', '「', '」', '『', '』'];
    if text.chars().any(|c| chinese_markers.contains(&c)) {
        return true;
    }
    text.chars().filter(|&c| c == '"').count() >= 2
}

/// Classify a sentence (`sentence_type.py:_classify_sentence`).
/// Returns None for empty or unrecognised sentences.
fn classify(s: &str) -> Option<SentenceType> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    if has_dialogue_markers(t) {
        return Some(SentenceType::Dialogue);
    }
    match t.chars().last()? {
        '？' | '?' => Some(SentenceType::Question),
        '！' | '!' => Some(SentenceType::Exclamation),
        '。' | '.' => Some(SentenceType::Statement),
        _ => None,
    }
}

/// Sentence-type signal (`sentence_type.py`).
///
/// Violations:
/// - statement → question
/// - question → statement
/// - dialogue → non-dialogue (any type that isn't Dialogue)
///
/// Score = min(1.0, violations / max(1, checks * 0.3))
pub fn sentence_type(pairs: &[LinePair]) -> Signal {
    let mut violations = 0usize;
    let mut checks = 0usize;
    let mut first: Option<u32> = None;
    let mut flagged = BTreeSet::new();

    for p in pairs {
        let (Some(s), Some(t)) = (classify(&p.src), classify(&p.tgt)) else {
            continue;
        };
        checks += 1;

        let is_violation = matches!(
            (s, t),
            (SentenceType::Statement, SentenceType::Question)
                | (SentenceType::Question, SentenceType::Statement)
                // dialogue → anything that is not dialogue
                | (SentenceType::Dialogue, SentenceType::Question)
                | (SentenceType::Dialogue, SentenceType::Statement)
                | (SentenceType::Dialogue, SentenceType::Exclamation)
        );

        if is_violation {
            violations += 1;
            first.get_or_insert(p.id);
            flagged.insert(p.id);
        }
    }

    if checks == 0 {
        return (0.0, None, BTreeSet::new());
    }

    let score = (violations as f64 / (1.0f64).max(checks as f64 * 0.3)).min(1.0);
    (score, first, flagged)
}

// ── last line ────────────────────────────────────────────────────────────────

/// Last-line coherence signal (`last_line.py`).
///
/// The last line (highest ID) gets three sub-checks:
/// - For EACH glossary term present in src but absent in tgt: +0.3
///   (Python iterates ALL terms, not breaking after the first match)
/// - Length ratio outside [0.5, 5.0]: +0.3 (uses `len()` i.e. char count)
/// - Empty translation: score = 1.0 (overrides, then capped at 1.0)
///
/// (Python had a missing-translation fallback of 0.5; our LinePair model always
/// provides both sides so the case cannot arise here.)
pub fn last_line(pairs: &[LinePair], terms: &BTreeMap<String, String>) -> Signal {
    let Some(p) = pairs.iter().max_by_key(|p| p.id) else {
        return (0.0, None, BTreeSet::new());
    };

    // Precompute lowercased translations once — avoids repeated allocation inside the loop.
    let terms_lower: Vec<(&String, String)> = terms
        .iter()
        .map(|(term, tr)| (term, tr.to_lowercase()))
        .collect();

    let mut score = 0.0f64;

    // Check 1: glossary term presence — ALL matching terms, not just the first
    let tgt_lower = p.tgt.to_lowercase();
    for (term, tr_lower) in &terms_lower {
        if p.src.contains(term.as_str()) && !tgt_lower.contains(tr_lower.as_str()) {
            score += 0.3;
        }
    }

    // Check 2: length anomaly
    // Python uses len() which counts code points (chars for BMP); char_len matches.
    let source_len = char_len(p.src.trim());
    let trans_len = char_len(p.tgt.trim());

    if source_len > 0 {
        // Python: ratio = trans_len / source_len — no special case for trans_len==0
        let ratio = trans_len as f64 / source_len as f64;
        if !(0.5..=5.0).contains(&ratio) {
            score += 0.3;
        }
    }

    // Check 3: empty translation overrides to 1.0
    if p.tgt.trim().is_empty() {
        score = 1.0;
    }

    let score = score.min(1.0);
    let first = (score > 0.0).then_some(p.id);
    let mut flagged = BTreeSet::new();
    if score > 0.0 {
        flagged.insert(p.id);
    }
    (score, first, flagged)
}

// ── length ratio ─────────────────────────────────────────────────────────────

/// Per-line char-length ratio outside [0.8, 4.0] (`length_ratio.py`).
///
/// Skip sources < 3 chars. Unlike the skeleton, do NOT skip tl==0 —
/// Python counts it as ratio=0.0 which is < MIN_RATIO and therefore flags it.
///
/// Score = min(1.0, anomalies / max(1, checks * 0.2))
pub fn length_ratio(pairs: &[LinePair]) -> Signal {
    const MIN_RATIO: f64 = 0.8;
    const MAX_RATIO: f64 = 4.0;

    let mut anomalies = 0usize;
    let mut checks = 0usize;
    let mut first: Option<u32> = None;
    let mut flagged = BTreeSet::new();

    for p in pairs {
        let sl = char_len(p.src.trim());
        if sl < 3 {
            continue;
        }
        checks += 1;
        let tl = char_len(p.tgt.trim());
        // When tl==0, ratio = 0.0 < MIN_RATIO — Python flags this.
        let ratio = tl as f64 / sl as f64;
        if !(MIN_RATIO..=MAX_RATIO).contains(&ratio) {
            anomalies += 1;
            first.get_or_insert(p.id);
            flagged.insert(p.id);
        }
    }

    if checks == 0 {
        return (0.0, None, BTreeSet::new());
    }

    let score = (anomalies as f64 / (1.0f64).max(checks as f64 * 0.2)).min(1.0);
    (score, first, flagged)
}
