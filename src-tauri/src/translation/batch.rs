//! One batch end-to-end: markers + hints + prompts in, validated marker-free
//! translations out. Port of `core/batch_translator.py:104-413`. Drift
//! detection (layer 2) runs at the pipeline level, not here.

use std::collections::BTreeMap;

use crate::events::ErrorKind;
use crate::glossary::model::Glossary;
use crate::llm::error::LlmError;
use crate::llm::service::LlmService;
use crate::llm::LlmRequest;
use crate::models::language_pair::LanguagePair;
use crate::translation::{parse_response, prompts};
use crate::validation::markers::{self, LineKind};
use crate::validation::{alignment, LinePair};

#[derive(Debug, Clone)]
pub struct BatchLine {
    pub id: u32,
    pub kind: LineKind,
    pub stripped_src: String,
}

#[derive(Debug, Clone)]
pub struct BatchSettings {
    pub pair: LanguagePair,
    /// Resolved translate.* template for the run's language pair.
    pub template: String,
    /// Resolved tone guideline text.
    pub tone_text: String,
}

#[derive(Debug)]
pub enum BatchOutcome {
    /// All lines translated; map id → marker-free text.
    Success(BTreeMap<u32, String>),
    /// Validation broke at `failed_from`; everything before it is salvaged.
    /// `reason` says why in plain words, for the run view's retry note.
    Partial { translated: BTreeMap<u32, String>, failed_from: u32, reason: &'static str },
    /// Nothing usable (transport error after retries, hopeless JSON).
    Failure(BatchFailure),
    /// A rejected key, an account out of credit, an unknown model — the whole
    /// run is doomed; the pipeline must stop the run instead of halving and
    /// retrying.
    Fatal(BatchFatal),
}

/// A batch that brought nothing back.
#[derive(Debug, Clone)]
pub struct BatchFailure {
    /// The raw error, for the log.
    pub message: String,
    /// Why, in plain words ("a reply came back incomplete").
    pub reason: &'static str,
    pub kind: ErrorKind,
    /// The run stopped while the request was out: nothing really failed.
    pub cancelled: bool,
}

/// An error that stops the run.
#[derive(Debug, Clone)]
pub struct BatchFatal {
    /// The raw error, for the log.
    pub message: String,
    pub kind: ErrorKind,
}

impl std::fmt::Display for BatchFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub const REASON_INCOMPLETE: &str = "a reply came back incomplete";
pub const REASON_MERGED: &str = "lines came back merged";
pub const REASON_MISMATCH: &str = "the reply didn't match the lines sent";
pub const REASON_EMPTY_LINES: &str = "a reply left lines empty";
pub const REASON_UNREADABLE: &str = "a reply couldn't be read";

/// Pure: why a reply failed its checks, in plain words.
fn validation_reason(a: &alignment::AlignmentCheck, m: &markers::MarkerCheck) -> &'static str {
    let only_missing = a.extra_ids.is_empty()
        && a.empty_translations.is_empty()
        && m.order_mismatches.is_empty()
        && m.corrupted_markers.is_empty()
        && m.extra_markers.is_empty();
    if !m.duplicate_markers.is_empty() {
        REASON_MERGED
    } else if !a.missing_ids.is_empty() && only_missing {
        REASON_INCOMPLETE
    } else if !a.empty_translations.is_empty() && a.missing_ids.is_empty() && a.extra_ids.is_empty() {
        REASON_EMPTY_LINES
    } else {
        REASON_MISMATCH
    }
}

/// Pure: why a request failed, in plain words.
fn request_reason(e: &LlmError) -> &'static str {
    match e.kind() {
        ErrorKind::RateLimit => "the provider kept asking us to slow down",
        ErrorKind::Network => "the connection to the provider failed",
        ErrorKind::Server => "the provider had trouble on its side",
        _ => match e {
            LlmError::Empty => "a reply came back empty",
            LlmError::Declined(_) => "the provider declined to answer",
            LlmError::Parse(_) => REASON_UNREADABLE,
            _ => "a request failed",
        },
    }
}

/// Core path on pre-stripped lines.
pub async fn translate_batch(
    svc: &LlmService,
    lines: &[BatchLine],
    glossary: &Glossary,
    context: &[(String, String)],
    settings: &BatchSettings,
) -> BatchOutcome {
    // Join with \n (Python uses space — no effect on substring glossary matching).
    let batch_content: String =
        lines.iter().map(|l| l.stripped_src.as_str()).collect::<Vec<_>>().join("\n");
    let filtered = glossary.filter_for_batch(&batch_content);

    // Markers + hints (`batch_translator.py:240-246`).
    // Python order: inject markers first (step 3), then inject hints into the
    // marked src (step 4). Net result per line: `<0001:D> 星汉[→Xinghan]那边`.
    // Our order: inject hints into stripped src, then prepend marker — identical
    // final string shape.
    let marked: Vec<(u32, String)> = lines
        .iter()
        .map(|l| (l.id, markers::inject(l.id, l.kind, &filtered.inject_hints(&l.stripped_src))))
        .collect();

    let system =
        prompts::system_prompt(&settings.template, &settings.pair, &filtered, &settings.tone_text);
    let user = prompts::user_prompt(&marked, context);

    let resp = match svc.request(LlmRequest { system, user }).await {
        Ok(r) => r,
        Err(e) if e.is_auth() || e.is_fatal() => {
            return BatchOutcome::Fatal(BatchFatal { message: e.to_string(), kind: e.kind() })
        }
        Err(e) => {
            return BatchOutcome::Failure(BatchFailure {
                message: e.to_string(),
                reason: request_reason(&e),
                kind: e.kind(),
                cancelled: e.is_cancelled(),
            })
        }
    };

    let pairs: Vec<LinePair> = match parse_response::extract_pairs(&resp.text) {
        Ok(p) => p,
        Err(e) => {
            return BatchOutcome::Failure(BatchFailure {
                message: e.to_string(),
                reason: REASON_UNREADABLE,
                kind: ErrorKind::Other,
                cancelled: false,
            })
        }
    };

    let expected: Vec<u32> = lines.iter().map(|l| l.id).collect();

    // Layer 0 then layer 1; either failure salvages the prefix.
    let (first_problem, reason) = {
        let a = alignment::check(&expected, &pairs);
        let m = markers::check(&expected, &pairs);
        match (a.is_valid, m.is_valid) {
            (true, true) => (None, REASON_MISMATCH),
            _ => (
                Some(
                    a.first_problem_id
                        .into_iter()
                        .chain(m.first_mismatch_id)
                        .min()
                        .expect("invalid check has a first id"),
                ),
                validation_reason(&a, &m),
            ),
        }
    };

    let mut translated: BTreeMap<u32, String> = BTreeMap::new();
    for p in &pairs {
        if let Some(cut) = first_problem {
            if p.id >= cut {
                continue;
            }
        }
        if expected.contains(&p.id) {
            translated.insert(p.id, markers::strip(&p.tgt));
        }
    }

    match first_problem {
        None => BatchOutcome::Success(translated),
        Some(cut) if translated.is_empty() => BatchOutcome::Failure(BatchFailure {
            message: format!("validation failed from id {cut}, nothing salvageable"),
            reason,
            kind: ErrorKind::Other,
            cancelled: false,
        }),
        Some(cut) => BatchOutcome::Partial { translated, failed_from: cut, reason },
    }
}

/// Convenience wrapper around raw (id, tagged-text) lines: strips tags,
/// translates, reapplies tags. Label kind derived from positioning tags
/// (`core/translator.py:380-387`).
pub async fn translate_batch_tagged(
    svc: &LlmService,
    raw: &[(u32, String)],
    glossary: &Glossary,
    context: &[(String, String)],
    settings: &BatchSettings,
) -> BatchOutcome {
    use crate::ass::tags;
    let strips: BTreeMap<u32, tags::TagStrip> =
        raw.iter().map(|(id, text)| (*id, tags::strip_positional(text))).collect();
    let lines: Vec<BatchLine> = raw
        .iter()
        .map(|(id, _text)| {
            let strip = &strips[id];
            // Python parity (translator.py:380-387): label only when stripped
            // text is non-empty AND a tag contains \pos or \an.  Pure-tag lines
            // (e.g. `{\an8}` alone) stay Dialogue.
            let kind = if !strip.stripped.trim().is_empty()
                && strip.tags.iter().any(|t| t.content.contains(r"\pos") || t.content.contains(r"\an"))
            {
                LineKind::Label
            } else {
                LineKind::Dialogue
            };
            BatchLine { id: *id, kind, stripped_src: strip.stripped.clone() }
        })
        .collect();

    let reapply = |map: BTreeMap<u32, String>| -> BTreeMap<u32, String> {
        map.into_iter().map(|(id, t)| (id, tags::reapply(&strips[&id], &t))).collect()
    };

    match translate_batch(svc, &lines, glossary, context, settings).await {
        BatchOutcome::Success(m) => BatchOutcome::Success(reapply(m)),
        BatchOutcome::Partial { translated, failed_from, reason } => {
            BatchOutcome::Partial { translated: reapply(translated), failed_from, reason }
        }
        f => f, // Failure / Fatal pass through
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glossary::model::Glossary;
    use crate::llm::service::LlmService;
    use crate::llm::test_support::ScriptedDriver;
    use crate::models::language_pair::LanguagePair;
    use crate::validation::markers::LineKind;
    use tokio_util::sync::CancellationToken;

    fn service(responses: Vec<&str>) -> LlmService {
        let script = responses.into_iter().map(|r| Ok(r.to_string())).collect();
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        LlmService::new(ScriptedDriver::new(script), 2, CancellationToken::new(), tx)
    }

    fn lines(v: &[(u32, &str)]) -> Vec<BatchLine> {
        v.iter()
            .map(|(id, src)| BatchLine {
                id: *id,
                kind: LineKind::Dialogue,
                stripped_src: src.to_string(),
            })
            .collect()
    }

    fn settings() -> BatchSettings {
        BatchSettings {
            pair: LanguagePair::from_codes("zh", "en").unwrap(),
            template: crate::prompts::default_text(crate::prompts::PromptId::TranslateZhEn).into(),
            tone_text: crate::prompts::default_text(crate::prompts::PromptId::ToneStandard).into(),
        }
    }

    #[tokio::test]
    async fn full_success_returns_all_translations_marker_free() {
        let svc = service(vec![
            r#"[{"id":1,"tgt":"<0001:D> Hello"},{"id":2,"tgt":"<0002:D> Goodbye"}]"#,
        ]);
        let out = translate_batch(
            &svc,
            &lines(&[(1, "你好"), (2, "再见")]),
            &Glossary::default(),
            &[],
            &settings(),
        )
        .await;
        let ok = match out {
            BatchOutcome::Success(map) => map,
            other => panic!("expected success, got {other:?}"),
        };
        assert_eq!(ok.get(&1).unwrap(), "Hello");
        assert_eq!(ok.get(&2).unwrap(), "Goodbye");
    }

    #[tokio::test]
    async fn partial_success_salvages_prefix() {
        // id 2 missing → keep id 1, report failure from id 2.
        let svc = service(vec![r#"[{"id":1,"tgt":"<0001:D> Hello"}]"#]);
        let out = translate_batch(
            &svc,
            &lines(&[(1, "你好"), (2, "再见")]),
            &Glossary::default(),
            &[],
            &settings(),
        )
        .await;
        match out {
            BatchOutcome::Partial { translated, failed_from, reason } => {
                assert_eq!(translated.get(&1).unwrap(), "Hello");
                assert_eq!(failed_from, 2);
                assert_eq!(reason, REASON_INCOMPLETE);
            }
            other => panic!("expected partial, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn merged_lines_say_so() {
        // Lines 2 and 3 came back as one.
        let svc = service(vec![
            r#"[{"id":1,"tgt":"<0001:D> Hello"},{"id":2,"tgt":"<0002:D> Bye <0003:D> now"}]"#,
        ]);
        let out = translate_batch(
            &svc,
            &lines(&[(1, "你好"), (2, "再见"), (3, "现在")]),
            &Glossary::default(),
            &[],
            &settings(),
        )
        .await;
        match out {
            BatchOutcome::Partial { reason, .. } => assert_eq!(reason, REASON_MERGED),
            other => panic!("expected partial, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unreadable_and_failed_requests_say_why() {
        let svc = service(vec!["sorry, no"]);
        match translate_batch(&svc, &lines(&[(1, "你好")]), &Glossary::default(), &[], &settings()).await {
            BatchOutcome::Failure(f) => {
                assert_eq!(f.reason, REASON_UNREADABLE);
                assert!(!f.cancelled);
            }
            other => panic!("expected failure, got {other:?}"),
        }
        let driver = ScriptedDriver::new(
            (0..3)
                .map(|_| {
                    Err(crate::llm::error::LlmError::Http {
                        status: 503,
                        body: "x".into(),
                        retry_after: None,
                    })
                })
                .collect(),
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let svc = LlmService::new(driver, 2, CancellationToken::new(), tx);
        match translate_batch(&svc, &lines(&[(1, "你好")]), &Glossary::default(), &[], &settings()).await {
            BatchOutcome::Failure(f) => {
                assert_eq!(f.kind, ErrorKind::Server);
                assert_eq!(f.reason, "the provider had trouble on its side");
            }
            other => panic!("expected failure, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn out_of_credit_is_fatal() {
        let driver = ScriptedDriver::new(vec![Err(crate::llm::error::LlmError::Http {
            status: 402,
            body: "Insufficient credits".into(),
            retry_after: None,
        })]);
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let svc = LlmService::new(driver, 2, CancellationToken::new(), tx);
        let out = translate_batch(&svc, &lines(&[(1, "你好")]), &Glossary::default(), &[], &settings()).await;
        assert!(matches!(out, BatchOutcome::Fatal(BatchFatal { kind: ErrorKind::Billing, .. })));
    }

    #[tokio::test]
    async fn unparseable_json_is_a_failure() {
        // One response is enough: parse errors are not transport-retried.
        let svc = service(vec!["sorry, no"]);
        let out =
            translate_batch(&svc, &lines(&[(1, "你好")]), &Glossary::default(), &[], &settings())
                .await;
        assert!(matches!(out, BatchOutcome::Failure(_)));
    }

    #[tokio::test]
    async fn auth_error_is_fatal() {
        let driver = ScriptedDriver::new(vec![Err(crate::llm::error::LlmError::Http {
            status: 401,
            body: "no".into(),
            retry_after: None,
        })]);
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let svc = LlmService::new(driver, 2, CancellationToken::new(), tx);
        let out =
            translate_batch(&svc, &lines(&[(1, "你好")]), &Glossary::default(), &[], &settings())
                .await;
        assert!(matches!(out, BatchOutcome::Fatal(_)));
    }

    #[tokio::test]
    async fn glossary_hints_reach_the_request() {
        let mut g = Glossary::new("xianxia");
        g.characters.insert("星汉".into(), "Xinghan".into());
        let driver = ScriptedDriver::new(vec![Ok(
            r#"[{"id":1,"tgt":"<0001:D> Xinghan, hello"}]"#.to_string(),
        )]);
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let svc = LlmService::new(driver.clone(), 2, CancellationToken::new(), tx);
        let _ = translate_batch(&svc, &lines(&[(1, "星汉你好")]), &g, &[], &settings()).await;
        let sent = driver.last_request().expect("a request was sent");
        assert!(sent.user.contains("星汉[→Xinghan]"));
        assert!(sent.system.contains("星汉 → Xinghan"));
    }

    #[tokio::test]
    async fn custom_template_reaches_the_request() {
        let driver = ScriptedDriver::new(vec![Ok(
            r#"[{"id":1,"tgt":"<0001:D> Hello"}]"#.to_string(),
        )]);
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let svc = LlmService::new(driver.clone(), 2, CancellationToken::new(), tx);
        let settings = BatchSettings {
            pair: LanguagePair::from_codes("zh", "en").unwrap(),
            template: "XCUSTOMX {GLOSSARY} {TONE}".into(),
            tone_text: "XTONEX".into(),
        };
        let _ = translate_batch(&svc, &lines(&[(1, "你好")]), &Glossary::default(), &[], &settings).await;
        let req = driver.last_request().expect("a request was sent");
        assert!(req.system.starts_with("XCUSTOMX"), "system starts with XCUSTOMX: {:?}", req.system);
        assert!(req.system.contains("XTONEX"), "system contains XTONEX: {:?}", req.system);
    }

    #[tokio::test]
    async fn tagged_partial_reapplies_tags_to_salvaged_prefix() {
        // id 2 missing from response → Partial; id 1's tag must still be reapplied.
        let svc = service(vec![r#"[{"id":1,"tgt":"<0001:D> Hello"}]"#]);
        let raw = vec![(1u32, r"{\an8}你好".to_string()), (2u32, "再见".to_string())];
        let out = translate_batch_tagged(&svc, &raw, &Glossary::default(), &[], &settings()).await;
        match out {
            BatchOutcome::Partial { translated, failed_from, .. } => {
                assert_eq!(translated.get(&1).unwrap(), r"{\an8}Hello");
                assert_eq!(failed_from, 2);
            }
            other => panic!("expected partial, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tags_are_reapplied_to_translations() {
        let svc = service(vec![r#"[{"id":1,"tgt":"<0001:D> Hello"}]"#]);
        let raw = vec![(1u32, r"{\an8}你好".to_string())];
        let out =
            translate_batch_tagged(&svc, &raw, &Glossary::default(), &[], &settings()).await;
        match out {
            BatchOutcome::Success(map) => assert_eq!(map.get(&1).unwrap(), r"{\an8}Hello"),
            other => panic!("expected success, got {other:?}"),
        }
    }
}
