//! Two-stage verification: fast drift over the whole file, then LLM
//! spot-checks. Port of `core/verifier.py`.
//!
//! # Deviations from Python (deliberate)
//!
//! **Deterministic sampling**: Python uses `random.sample` (`verifier.py:321`).
//! We use a deterministic every-Nth stride over the valid-id list so runs are
//! reproducible (important for test assertions and re-run comparability).
//! Quota math, region thirds, bounds [5, 80], flagged-priority, and length
//! preferences all match the Python reference exactly.
//!
//! **Priority IDs — global pre-pass**: Python collects *all* priority IDs into
//! `samples` and `already_sampled` before looping over regions
//! (`verifier.py:267-282`). This implementation matches that behaviour: flagged
//! IDs are inserted once (globally) before the per-region quota fill, so they
//! count against the global deduplication set but *not* against any region quota
//! (same as Python).
//!
//! **Quota formula**: matches Python exactly —
//! `max(MIN, min(MAX, int(region_len * SAMPLE_RATE)))` where `int()` truncates
//! (Rust `as usize` on an `f64` also truncates) (`verifier.py:290-292`).
//!
//! **Stage-1 `failed_line_ids` is deliberately broader than Python**: we return
//! *all* drift-flagged IDs rather than just the single `drift_start` ID. This
//! gives scope-calculation a richer signal — more candidate lines to re-translate
//! — at the cost of a slightly wider retry window. The Python reference only
//! tracks one ID here (`verifier.py:176`).

use std::collections::{BTreeMap, BTreeSet};

use crate::events::VerifyIssue;
use crate::llm::error::LlmError;
use crate::llm::service::LlmService;
use crate::llm::LlmRequest;
use crate::translation::parse_response;
use crate::validation::{drift, LinePair};

const SAMPLE_RATE: f64 = 0.5;
const MIN_PER_REGION: usize = 5;
const MAX_PER_REGION: usize = 80;
const LLM_BATCH: usize = 100;
const MIN_LEN: usize = 4;
const MIN_LEN_FALLBACK: usize = 3;

#[derive(Debug, Default)]
pub struct VerifyReport {
    pub issues: Vec<VerifyIssue>,
    pub sampled_line_ids: Vec<u32>,
    pub failed_line_ids: BTreeSet<u32>,
}

/// `lines`: id → (stripped source, stripped translation).
pub async fn verify_file(
    svc: &LlmService,
    lines: &BTreeMap<u32, (String, String)>,
    glossary_terms: &BTreeMap<String, String>,
    verify_template: &str,
) -> Result<VerifyReport, LlmError> {
    let pairs: Vec<LinePair> = lines
        .iter()
        .map(|(id, (s, t))| LinePair { id: *id, src: s.clone(), tgt: t.clone() })
        .collect();

    // Stage 1: fast drift — failure short-circuits (`verifier.py:141-176`).
    let d = drift::detect(&pairs, glossary_terms);
    if d.has_suspected_drift {
        let start = d.suspected_drift_start_id.unwrap_or(1);
        return Ok(VerifyReport {
            issues: drift_issues(lines, glossary_terms, start),
            sampled_line_ids: Vec::new(),
            failed_line_ids: d.flagged_line_ids.into_iter().collect(),
        });
    }

    // Stage 2: LLM sampling.
    let samples = build_samples(lines, &d.flagged_line_ids.iter().copied().collect());
    let sampled_line_ids: Vec<u32> = samples.iter().map(|s| s.0).collect();
    let mut issues: Vec<VerifyIssue> = Vec::new();
    let mut failed: BTreeSet<u32> = BTreeSet::new();

    for chunk in samples.chunks(LLM_BATCH) {
        let payload: Vec<serde_json::Value> = chunk
            .iter()
            .map(|(id, s, t)| serde_json::json!({ "id": id, "src": s, "tgt": t }))
            .collect();
        let req = LlmRequest {
            system: verify_template.to_string(),
            user: serde_json::to_string(&payload).expect("serializable"),
        };
        let resp = match svc.request(req).await {
            Ok(r) => r,
            // Auth death and user cancellation must NOT degrade gracefully —
            // a dead key or mid-verify cancel would otherwise finish the file
            // as "clean without verification" and write unverified output.
            Err(e) if e.is_auth() || e.is_fatal() || e.is_cancelled() => return Err(e),
            // Other failures keep degrading gracefully (`verifier.py:399-400`).
            Err(_) => continue,
        };
        // `{"issues":[{id,reason}]}` is the expected shape; on parse failure
        // try a bare array `[{id,reason},...]` (the model may truncate the
        // outer wrapper when the context is nearly full). If both fail, degrade
        // gracefully with an empty list (`verifier.py:389-398`).
        let raw_issues: Vec<serde_json::Value> = match parse_response::extract_object(&resp.text) {
            Ok(v) => v.get("issues").and_then(|i| i.as_array()).cloned().unwrap_or_default(),
            Err(_) => parse_response::extract_array(&resp.text).unwrap_or_default(),
        };
        for item in raw_issues {
            let Some(id) = item.get("id").and_then(|i| i.as_u64()).map(|i| i as u32) else {
                continue;
            };
            let Some((src, tgt)) = lines.get(&id) else { continue };
            failed.insert(id);
            let reason = item.get("reason").and_then(|r| r.as_str());
            issues.push(meaning_issue(id, src, tgt, reason));
        }
    }

    Ok(VerifyReport { issues, sampled_line_ids, failed_line_ids: failed })
}

/// What users see for a line that may have lost the original's meaning. The
/// check is called drift inside the engine; users never see that word.
pub const MEANING_DESCRIPTION: &str = "This line may not say what the original says.";

/// A `meaning` issue; the verifier's reason, when it gave one, follows in
/// brackets (unless it talks about "drift").
fn meaning_issue(line_id: u32, source: &str, translation: &str, reason: Option<&str>) -> VerifyIssue {
    let reason = reason.map(str::trim).filter(|r| !r.is_empty() && !r.to_lowercase().contains("drift"));
    VerifyIssue {
        line_id,
        source: source.to_string(),
        translation: translation.to_string(),
        issue_type: "meaning".into(),
        description: match reason {
            Some(r) => format!("{} ({r})", MEANING_DESCRIPTION),
            None => MEANING_DESCRIPTION.into(),
        },
        severity: "high".into(),
        term: None,
        expected: None,
    }
}

/// One `name` issue per (line, glossary term) where the line's source holds
/// the term and its translation doesn't hold the glossary's translation
/// (case-insensitive; `lines` come with their tags stripped). The same check
/// the drift score counts, but on every line, whatever the score: these are
/// review issues only and never decide what gets translated again.
pub fn name_issues(
    lines: &BTreeMap<u32, (String, String)>,
    glossary_terms: &BTreeMap<String, String>,
) -> Vec<VerifyIssue> {
    let pairs: Vec<LinePair> = lines
        .iter()
        .map(|(id, (s, t))| LinePair { id: *id, src: s.clone(), tgt: t.clone() })
        .collect();
    drift::signals::glossary_misses(&pairs, glossary_terms)
        .into_iter()
        .map(|miss| {
            let (src, tgt) = lines.get(&miss.line_id).cloned().unwrap_or_default();
            VerifyIssue {
                line_id: miss.line_id,
                source: src,
                translation: tgt,
                issue_type: "name".into(),
                description: format!(
                    "\"{}\" should be \"{}\" here, per your glossary.",
                    miss.term, miss.expected
                ),
                severity: "high".into(),
                term: Some(miss.term),
                expected: Some(miss.expected),
            }
        })
        .collect()
}

/// Put `extra` into `issues` by line, each after the issues already there
/// for its line. Line 0 (a summary such as "…and 12 more untranslated
/// lines") stays last.
pub fn insert_by_line(issues: &mut Vec<VerifyIssue>, extra: Vec<VerifyIssue>) {
    for issue in extra {
        let at = issues.partition_point(|i| i.line_id != 0 && i.line_id <= issue.line_id);
        issues.insert(at, issue);
    }
}

/// Stage 1's issues: one `name` issue per glossary term a line is missing,
/// then a `meaning` issue where the drift starts unless a name issue already
/// covers that line. Retranslation still follows the drift flags; this only
/// says what is wrong where.
fn drift_issues(
    lines: &BTreeMap<u32, (String, String)>,
    glossary_terms: &BTreeMap<String, String>,
    start: u32,
) -> Vec<VerifyIssue> {
    let mut issues = name_issues(lines, glossary_terms);
    if !issues.iter().any(|i| i.line_id == start) {
        let (src, tgt) = lines.get(&start).cloned().unwrap_or_default();
        let at = issues.partition_point(|i| i.line_id < start);
        issues.insert(at, meaning_issue(start, &src, &tgt, None));
    }
    issues
}

/// Sampling: thirds by sorted id; 50% per region clamped [5, 80]; priority
/// (flagged) IDs collected globally first; then deterministic every-Nth fill
/// per region; prefer source length ≥4 chars, refill with ≥3 if quota not
/// met (`verifier.py:228-338`).
///
/// Python region split (`verifier.py:253-259`):
/// - `third = total // 3`
/// - if `third > 0`: start=`[:third]`, middle=`[third:2*third]`, end=`[2*third:]`
/// - else: start = all ids, middle = [], end = []
///
/// Python quota (`verifier.py:290-292`):
/// `target = max(MIN_SAMPLES, min(MAX_SAMPLES, int(region_len * SAMPLE_RATE)))`
/// where `int()` truncates toward zero.
pub fn build_samples(
    lines: &BTreeMap<u32, (String, String)>,
    flagged: &BTreeSet<u32>,
) -> Vec<(u32, String, String)> {
    let ids: Vec<u32> = lines.keys().copied().collect();
    if ids.is_empty() {
        return Vec::new();
    }

    let total = ids.len();
    let third = total / 3;

    // Mirror Python's conditional split exactly (`verifier.py:256-259`).
    let (start_ids, middle_ids, end_ids): (&[u32], &[u32], &[u32]) = if third > 0 {
        (&ids[..third], &ids[third..2 * third], &ids[2 * third..])
    } else {
        (&ids[..], &[], &[])
    };

    let mut out: Vec<(u32, String, String)> = Vec::new();
    let mut already_sampled: BTreeSet<u32> = BTreeSet::new();

    // Global priority pass: all flagged IDs that exist in `lines`, before any
    // region fill (`verifier.py:267-282`).
    for &id in flagged.iter() {
        if lines.contains_key(&id) {
            let (s, t) = &lines[&id];
            out.push((id, s.clone(), t.clone()));
            already_sampled.insert(id);
        }
    }

    let regions: [&[u32]; 3] = [start_ids, middle_ids, end_ids];
    for region in regions {
        if region.is_empty() {
            continue;
        }

        // Quota: max(MIN, min(MAX, int(region_len * SAMPLE_RATE)))
        // `as usize` on f64 truncates (same as Python `int()`).
        let target: usize = (region.len() as f64 * SAMPLE_RATE) as usize;
        let target = target.clamp(MIN_PER_REGION, MAX_PER_REGION);

        // Build candidate pool with the higher length threshold first;
        // fall back to the lower threshold if the pool is too small
        // (`verifier.py:296-313`).
        let valid_primary: Vec<u32> = region
            .iter()
            .copied()
            .filter(|id| {
                !already_sampled.contains(id)
                    && lines[id].0.chars().count() >= MIN_LEN
            })
            .collect();

        let valid: Vec<u32> = if valid_primary.len() < target {
            region
                .iter()
                .copied()
                .filter(|id| {
                    !already_sampled.contains(id)
                        && lines[id].0.chars().count() >= MIN_LEN_FALLBACK
                })
                .collect()
        } else {
            valid_primary
        };

        if valid.is_empty() {
            continue;
        }

        // `sample_size = min(target, len(valid))` (`verifier.py:318`).
        let sample_size = target.min(valid.len());

        // Deterministic every-Nth stride (deviation from Python's random.sample).
        let step = (valid.len() / sample_size).max(1);
        let picked: Vec<u32> = valid.iter().copied().step_by(step).take(sample_size).collect();

        for id in picked {
            let (s, t) = &lines[&id];
            out.push((id, s.clone(), t.clone()));
            already_sampled.insert(id);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::service::LlmService;
    use crate::llm::test_support::ScriptedDriver;
    use std::collections::BTreeMap;
    use tokio_util::sync::CancellationToken;

    fn tpl() -> &'static str {
        crate::prompts::default_text(crate::prompts::PromptId::Verify)
    }

    fn pairs(n: u32) -> BTreeMap<u32, (String, String)> {
        (1..=n)
            .map(|i| (i, (format!("中文句子内容第{i}行了"), format!("English sentence line {i}"))))
            .collect()
    }

    #[test]
    fn sampling_covers_three_regions_with_bounds() {
        let s = build_samples(&pairs(300), &Default::default());
        // 100 per region → 50% = 50, capped at 80, min 5.
        assert_eq!(s.len(), 150);
        let ids: Vec<u32> = s.iter().map(|x| x.0).collect();
        assert!(ids.iter().any(|&i| i <= 100));
        assert!(ids.iter().any(|&i| i > 100 && i <= 200));
        assert!(ids.iter().any(|&i| i > 200));
    }

    #[test]
    fn sampling_prioritizes_flagged_ids() {
        let flagged: std::collections::BTreeSet<u32> = [7, 8].into();
        let s = build_samples(&pairs(30), &flagged);
        let ids: Vec<u32> = s.iter().map(|x| x.0).collect();
        assert!(ids.contains(&7) && ids.contains(&8));
    }

    #[tokio::test(start_paused = true)]
    async fn fast_drift_stage_short_circuits() {
        // All-empty translations make drift fire without any LLM call.
        let mut p = BTreeMap::new();
        for i in 1..=10u32 {
            p.insert(i, (format!("很长的中文第{i}句？"), String::new()));
        }
        let driver = ScriptedDriver::new(vec![]);
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let svc = LlmService::new(driver.clone(), 2, CancellationToken::new(), tx);
        let r = verify_file(&svc, &p, &BTreeMap::new(), tpl()).await.unwrap();
        assert!(!r.issues.is_empty());
        // No glossary: the one issue says what's wrong in plain words.
        assert_eq!(r.issues[0].issue_type, "meaning");
        assert_eq!(r.issues[0].description, MEANING_DESCRIPTION);
        assert_eq!(driver.call_count(), 0); // stage 2 skipped
    }

    #[tokio::test(start_paused = true)]
    async fn missing_glossary_names_become_one_issue_per_line() {
        let mut terms = BTreeMap::new();
        terms.insert("叶长生".to_string(), "Ye Changsheng".to_string());
        terms.insert("凌天门".to_string(), "Lingtian Sect".to_string());
        let mut p = BTreeMap::new();
        p.insert(1, ("叶长生你要去哪里？".to_string(), "Where are you going, Ye Changshen.".to_string()));
        p.insert(2, ("凌天门在吗？".to_string(), "Is anyone there.".to_string()));
        p.insert(3, ("叶长生不会走的".to_string(), "Ye Changsheng won't leave".to_string()));
        p.insert(4, ("好。".to_string(), String::new()));
        let driver = ScriptedDriver::new(vec![]);
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let svc = LlmService::new(driver.clone(), 2, CancellationToken::new(), tx);
        let r = verify_file(&svc, &p, &terms, tpl()).await.unwrap();
        assert_eq!(driver.call_count(), 0, "drift fired; no model call");

        let names: Vec<_> = r.issues.iter().filter(|i| i.issue_type == "name").collect();
        assert_eq!(names.len(), 2, "{:?}", r.issues);
        assert_eq!(names[0].line_id, 1);
        assert_eq!(names[0].term.as_deref(), Some("叶长生"));
        assert_eq!(names[0].expected.as_deref(), Some("Ye Changsheng"));
        assert_eq!(
            names[0].description,
            "\"叶长生\" should be \"Ye Changsheng\" here, per your glossary."
        );
        assert_eq!(names[0].translation, "Where are you going, Ye Changshen.");
        assert_eq!((names[1].line_id, names[1].term.as_deref()), (2, Some("凌天门")));
        // Line 3 uses the name: no issue. The drift starts on line 1, which a
        // name issue already explains: no extra meaning issue.
        assert!(r.issues.iter().all(|i| i.line_id != 3));
        assert!(r.issues.iter().all(|i| i.issue_type != "drift"));
        assert!(r.issues.iter().all(|i| !i.description.to_lowercase().contains("drift")));
        // Retranslation still follows every drift flag.
        assert!(r.failed_line_ids.contains(&1) && r.failed_line_ids.contains(&2));
    }

    #[tokio::test(start_paused = true)]
    async fn llm_sampling_reports_flagged_ids() {
        let driver =
            ScriptedDriver::new(vec![Ok(r#"{"issues":[{"id":3,"reason":"unrelated"}]}"#.into())]);
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let svc = LlmService::new(driver, 2, CancellationToken::new(), tx);
        let r = verify_file(&svc, &pairs(12), &BTreeMap::new(), tpl()).await.unwrap();
        assert_eq!(r.failed_line_ids, [3].into());
        assert_eq!(r.issues.len(), 1);
        assert_eq!(r.issues[0].line_id, 3);
        assert_eq!(r.issues[0].issue_type, "meaning");
        assert_eq!(r.issues[0].description, format!("{MEANING_DESCRIPTION} (unrelated)"));
    }

    #[test]
    fn meaning_issues_never_say_drift() {
        let i = meaning_issue(1, "s", "t", Some("Line drift: unrelated topics"));
        assert_eq!(i.description, MEANING_DESCRIPTION);
        let i = meaning_issue(1, "s", "t", Some("  "));
        assert_eq!(i.description, MEANING_DESCRIPTION);
    }

    #[tokio::test(start_paused = true)]
    async fn bare_array_response_still_yields_issues() {
        let driver =
            ScriptedDriver::new(vec![Ok(r#"[{"id":3,"reason":"unrelated"}]"#.into())]);
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let svc = LlmService::new(driver, 2, CancellationToken::new(), tx);
        let r = verify_file(&svc, &pairs(12), &BTreeMap::new(), tpl()).await.unwrap();
        assert_eq!(r.failed_line_ids, [3].into());
    }

    #[tokio::test(start_paused = true)]
    async fn llm_failure_means_no_issues_not_an_error() {
        let driver = ScriptedDriver::new(vec![
            Err(crate::llm::error::LlmError::Transport("x".into())),
            Err(crate::llm::error::LlmError::Transport("x".into())),
            Err(crate::llm::error::LlmError::Transport("x".into())),
        ]);
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let svc = LlmService::new(driver, 2, CancellationToken::new(), tx);
        let r = verify_file(&svc, &pairs(12), &BTreeMap::new(), tpl()).await.unwrap();
        assert!(r.issues.is_empty()); // verifier degrades gracefully
    }

    #[tokio::test(start_paused = true)]
    async fn auth_error_aborts_verification() {
        let driver = ScriptedDriver::new(vec![Err(crate::llm::error::LlmError::Http {
            status: 401,
            body: "no".into(),
            retry_after: None,
        })]);
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let svc = LlmService::new(driver, 2, CancellationToken::new(), tx);
        let err = verify_file(&svc, &pairs(12), &BTreeMap::new(), tpl()).await.unwrap_err();
        assert!(err.is_auth());
    }

    #[tokio::test(start_paused = true)]
    async fn custom_verify_template_reaches_the_request() {
        let driver = ScriptedDriver::new(vec![Ok(r#"{"issues":[]}"#.into())]);
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let svc = LlmService::new(driver.clone(), 2, CancellationToken::new(), tx);
        verify_file(&svc, &pairs(12), &BTreeMap::new(), "XVERIFYX").await.unwrap();
        assert_eq!(driver.last_request().unwrap().system, "XVERIFYX");
    }

    #[tokio::test(start_paused = true)]
    async fn cancellation_aborts_verification() {
        // A cancelled service yields a cancellation error on every request.
        // verify_file must propagate it instead of finishing "clean", which
        // would write unverified output.
        let driver = ScriptedDriver::new(vec![]);
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let cancel = CancellationToken::new();
        let svc = LlmService::new(driver, 2, cancel.clone(), tx);
        cancel.cancel();
        let err = verify_file(&svc, &pairs(12), &BTreeMap::new(), tpl()).await.unwrap_err();
        assert!(err.is_cancelled());
    }

    #[test]
    fn every_line_missing_a_name_gets_a_name_issue_whatever_the_score() {
        let mut terms = BTreeMap::new();
        terms.insert("元清".to_string(), "Yuan Qing".to_string());
        let mut p = BTreeMap::new();
        for i in 1..=80u32 {
            p.insert(i, (format!("第{i}句。"), format!("Line {i}.")));
        }
        p.insert(74, ("元清，我不信。".to_string(), "Follow them.".to_string()));
        p.insert(75, ("元清来了。".to_string(), "yuan qing is here.".to_string()));
        let pairs: Vec<LinePair> =
            p.iter().map(|(id, (s, t))| LinePair { id: *id, src: s.clone(), tgt: t.clone() }).collect();
        assert!(!drift::detect(&pairs, &terms).has_suspected_drift, "the score alone wouldn't show it");

        let issues = name_issues(&p, &terms);
        assert_eq!(issues.len(), 1, "case doesn't matter: {issues:?}");
        assert_eq!((issues[0].line_id, issues[0].term.as_deref()), (74, Some("元清")));
        assert_eq!(issues[0].expected.as_deref(), Some("Yuan Qing"));
        assert_eq!(issues[0].description, "\"元清\" should be \"Yuan Qing\" here, per your glossary.");
    }

    #[test]
    fn issues_go_in_by_line_with_the_summary_last() {
        let issue = |line_id: u32, issue_type: &str| VerifyIssue {
            line_id,
            source: String::new(),
            translation: String::new(),
            issue_type: issue_type.into(),
            description: String::new(),
            severity: "high".into(),
            term: None,
            expected: None,
        };
        let mut issues = vec![issue(2, "meaning"), issue(5, "untranslated"), issue(0, "untranslated")];
        insert_by_line(&mut issues, vec![issue(1, "name"), issue(2, "name"), issue(9, "name")]);
        let order: Vec<(u32, &str)> = issues.iter().map(|i| (i.line_id, i.issue_type.as_str())).collect();
        assert_eq!(
            order,
            [(1, "name"), (2, "meaning"), (2, "name"), (5, "untranslated"), (9, "name"), (0, "untranslated")]
        );
    }
}
