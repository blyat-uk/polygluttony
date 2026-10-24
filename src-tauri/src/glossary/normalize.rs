//! Per-category glossary normalization (O12 + build step 6). Port of
//! `glossary_builder.py:441-533`.
//!
//! When invoked standalone (O12), the world type is derived from the glossary
//! file itself — not from the project's effective override — because the file's
//! world was set by the build that created it. This is accepted behaviour.

use std::collections::BTreeMap;

use futures::future::join_all;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use ts_rs::TS;

use crate::events::{GlossaryEvent, LogLevel};
use crate::glossary::diff::GlossaryDiff;
use crate::glossary::model::{Glossary, GlossaryDoc, CATEGORIES};
use crate::glossary::prompts;
use crate::llm::service::LlmService;
use crate::llm::LlmRequest;
use crate::translation::parse_response;

/// O12 result: the normalized glossary + diff, NOT yet saved — the UI shows a
/// review and saves on accept.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct NormalizeReview {
    pub normalized: GlossaryDoc,
    pub diff: GlossaryDiff,
}

/// Parse one category response into a validated term map.
///
/// Returns `Ok(map)` on success. Returns `Err(reason)` when the response
/// cannot be parsed at all, so the caller can log the specific error and
/// keep the original category.
///
/// Values trimmed + re-validated via [`Glossary::is_valid_translation`]
/// (deviation 8 — Python trusted the response wholesale,
/// `glossary_builder.py:470-473`).
///
/// Note: an empty `Ok` map (`{}` or all-invalid values) does NOT mean "keep
/// original" — that is the caller's responsibility (see wipe-guard in
/// `normalize_pass`). This matches the deviation from Python where any parsed
/// dict, including empty, was blindly accepted.
fn parse_category_response(text: &str) -> Result<BTreeMap<String, String>, String> {
    // extract_object guarantees the returned Value is an object.
    let v = parse_response::extract_object(text)
        .map_err(|e| format!("the answer couldn't be read ({e})"))?;
    let obj = v.as_object().expect("extract_object guarantees object");
    let mut out = BTreeMap::new();
    for (k, val) in obj {
        if let Some(s) = val.as_str() {
            let trimmed = s.trim();
            if Glossary::is_valid_translation(trimmed) {
                out.insert(k.clone(), trimmed.to_string());
            }
        }
    }
    Ok(out)
}

/// Normalize every non-empty category concurrently (the service bounds
/// concurrency). World type comes from the glossary itself
/// (`glossary_builder.py:504`: `glossary.world_type or "modern"`). A failed
/// category keeps its original terms (ONE warning log per failure).
pub async fn normalize_pass(
    svc: &LlmService,
    glossary: &Glossary,
    tx: &mpsc::Sender<GlossaryEvent>,
    templates: &std::collections::BTreeMap<String, String>,
) -> Glossary {
    let world =
        if glossary.world_type.is_empty() { "modern" } else { glossary.world_type.as_str() };

    let jobs: Vec<&str> = CATEGORIES
        .iter()
        .copied()
        .filter(|c| !glossary.category(c).is_empty())
        .collect();
    let futures = jobs.iter().map(|c| {
        let template = templates
            .get(*c)
            .unwrap_or_else(|| panic!("GlossaryPrompts.normalize missing category {c}"));
        let req = LlmRequest {
            system: prompts::normalize_prompt(template, world),
            user: prompts::normalize_user_prompt(glossary.category(c)),
        };
        svc.request(req)
    });
    let results = join_all(futures).await;

    let mut out = glossary.clone();
    for (c, result) in jobs.iter().zip(results) {
        let map = match result {
            Err(e) => {
                let _ = tx
                    .send(GlossaryEvent::Log {
                        level: LogLevel::Warning,
                        message: format!("Tidy up, {c}: the request failed, so they stay as they were ({e})"),
                    })
                    .await;
                continue; // original kept; one log only
            }
            Ok(resp) => match parse_category_response(&resp.text) {
                Err(reason) => {
                    let _ = tx
                        .send(GlossaryEvent::Log {
                            level: LogLevel::Warning,
                            message: format!(
                                "Tidy up, {c}: {reason}, so they stay as they were"
                            ),
                        })
                        .await;
                    continue;
                }
                Ok(m) => m,
            },
        };
        // Wipe-guard: if the original category is non-empty and the parsed
        // replacement is empty (e.g. LLM returned `{}` or all-invalid values),
        // keep the original rather than silently deleting every term. This is a
        // deliberate deviation from Python, which would accept the wipe
        // (`glossary_builder.py:470-473` trusts any dict); it mirrors the
        // wipe-guard in `personalize_pass`.
        if map.is_empty() && !glossary.category(c).is_empty() {
            let _ = tx
                .send(GlossaryEvent::Log {
                    level: LogLevel::Warning,
                    message: format!(
                        "Tidy up, {c}: the answer had no usable terms, so they stay as they were"
                    ),
                })
                .await;
            continue;
        }
        *out.category_mut(c) = map;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glossary::model::Glossary;
    use crate::llm::error::LlmError;
    use crate::llm::service::LlmService;
    use crate::llm::test_support::ScriptedDriver;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    fn svc(driver: Arc<ScriptedDriver>, cap: u32) -> LlmService {
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        LlmService::new(driver, cap, CancellationToken::new(), tx)
    }

    fn gtx() -> tokio::sync::mpsc::Sender<crate::events::GlossaryEvent> {
        tokio::sync::mpsc::channel(64).0
    }

    fn default_templates() -> std::collections::BTreeMap<String, String> {
        crate::prompts::GlossaryPrompts::defaults().normalize
    }

    #[tokio::test(start_paused = true)]
    async fn normalizes_each_nonempty_category() {
        let mut g = Glossary::new("xianxia");
        g.characters.insert("林动".into(), "lin dong".into());
        // Only one non-empty category → exactly one LLM call.
        let d = ScriptedDriver::new(vec![Ok(r#"{"林动":"Lin Dong"}"#.into())]);
        let templates = default_templates();
        let out = normalize_pass(&svc(d.clone(), 2), &g, &gtx(), &templates).await;
        assert_eq!(out.characters.get("林动").unwrap(), "Lin Dong");
        assert_eq!(d.call_count(), 1);
        assert_eq!(out.world_type, "xianxia");
    }

    #[tokio::test(start_paused = true)]
    async fn failed_category_keeps_original_terms() {
        let mut g = Glossary::new("wuxia");
        g.characters.insert("张三".into(), "zhang san".into());
        g.locations.insert("华山".into(), "Mount Hua".into());
        // cap 1 + non-retryable error → one driver call per category, in
        // CATEGORIES order (characters before locations) — deterministic.
        let d = ScriptedDriver::new(vec![
            Err(LlmError::Http { status: 400, body: "bad".into(), retry_after: None }), // characters fails
            Ok(r#"{"华山":"Mt. Hua"}"#.into()),                       // locations succeeds
        ]);
        let templates = default_templates();
        let out = normalize_pass(&svc(d, 1), &g, &gtx(), &templates).await;
        assert_eq!(out.characters.get("张三").unwrap(), "zhang san"); // kept
        assert_eq!(out.locations.get("华山").unwrap(), "Mt. Hua"); // replaced
    }

    #[tokio::test(start_paused = true)]
    async fn unparseable_response_keeps_original() {
        let mut g = Glossary::new("modern");
        g.items.insert("a".into(), "A".into());
        let d = ScriptedDriver::new(vec![Ok("I refuse to answer with JSON".into())]);
        let templates = default_templates();
        let out = normalize_pass(&svc(d, 2), &g, &gtx(), &templates).await;
        assert_eq!(out.items.get("a").unwrap(), "A");
    }

    #[tokio::test(start_paused = true)]
    async fn normalized_values_are_revalidated() {
        let mut g = Glossary::new("modern");
        g.skills.insert("k1".into(), "V1".into());
        g.skills.insert("k2".into(), "V2".into());
        // LLM merges k2 away, empties k1's value (invalid → dropped), adds k3.
        let d = ScriptedDriver::new(vec![Ok(r#"{"k1":"   ","k3":"  V3  "}"#.into())]);
        let templates = default_templates();
        let out = normalize_pass(&svc(d, 2), &g, &gtx(), &templates).await;
        assert!(!out.skills.contains_key("k1")); // empty value dropped
        assert!(!out.skills.contains_key("k2")); // legitimately merged away
        assert_eq!(out.skills.get("k3").unwrap(), "V3"); // trimmed
    }

    #[tokio::test(start_paused = true)]
    async fn empty_glossary_makes_no_calls() {
        let d = ScriptedDriver::new(vec![]); // would panic if called
        let g = Glossary::new("modern");
        let templates = default_templates();
        let out = normalize_pass(&svc(d, 2), &g, &gtx(), &templates).await;
        assert!(out.is_empty());
    }

    /// Wipe-guard: if the original category is non-empty and the LLM returns
    /// `{}` (or only invalid values), the original is preserved.
    #[tokio::test(start_paused = true)]
    async fn empty_response_keeps_original_not_wiped() {
        let mut g = Glossary::new("xianxia");
        g.characters.insert("林动".into(), "Lin Dong".into());
        // `{}` parses fine but yields no valid terms — original must survive.
        let d = ScriptedDriver::new(vec![Ok("{}".into())]);
        let templates = default_templates();
        let out = normalize_pass(&svc(d.clone(), 2), &g, &gtx(), &templates).await;
        assert_eq!(out.characters.get("林动").unwrap(), "Lin Dong");
        // Also test a nested-object response that strips to empty after validation.
        let mut g2 = Glossary::new("xianxia");
        g2.characters.insert("林动".into(), "Lin Dong".into());
        let d2 = ScriptedDriver::new(vec![Ok(r#"{"林动":{"nested":"obj"}}"#.into())]);
        let out2 = normalize_pass(&svc(d2, 2), &g2, &gtx(), &templates).await;
        assert_eq!(out2.characters.get("林动").unwrap(), "Lin Dong");
    }

    /// Pin the "modern" fallback: a glossary with an empty world_type should
    /// send "modern" in the system prompt.
    #[tokio::test(start_paused = true)]
    async fn empty_world_type_uses_modern_in_prompt() {
        let mut g = Glossary::new("");
        g.characters.insert("a".into(), "A".into());
        let d = ScriptedDriver::new(vec![Ok(r#"{"a":"A"}"#.into())]);
        let templates = default_templates();
        normalize_pass(&svc(d.clone(), 2), &g, &gtx(), &templates).await;
        let req = d.last_request().unwrap();
        assert!(req.system.contains("modern"), "expected 'modern' in system prompt");
    }

    /// Custom normalize template reaches the wire: a marked template must appear
    /// in req.system, and its {world_type} placeholder must be filled.
    #[tokio::test(start_paused = true)]
    async fn custom_normalize_template_reaches_the_request() {
        let mut g = Glossary::new("wuxia");
        g.characters.insert("林动".into(), "Lin Dong".into());
        let d = ScriptedDriver::new(vec![Ok(r#"{"林动":"Lin Dong"}"#.into())]);
        let mut templates = default_templates();
        templates.insert("characters".into(), "XNORMX {world_type}".into());
        normalize_pass(&svc(d.clone(), 2), &g, &gtx(), &templates).await;
        let req = d.last_request().unwrap();
        assert!(
            req.system.starts_with("XNORMX"),
            "custom normalize template must reach the wire: {:?}",
            req.system
        );
        assert!(
            !req.system.contains("{world_type}"),
            "world_type placeholder must be filled: {:?}",
            req.system
        );
        assert!(req.system.contains("wuxia"), "world value must appear: {:?}", req.system);
    }
}
