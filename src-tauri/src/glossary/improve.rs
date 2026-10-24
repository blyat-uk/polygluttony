//! The Improve menu: Tidy up and Look up names online, run on the glossary as
//! it is (no subtitle files are read) and scoped to all terms, one category,
//! or picked terms. Both return a review and save nothing; the UI saves what
//! the user accepts through `save_glossary`.
//!
//! The commands in `commands/glossary.rs` do the app plumbing (config, op
//! slot, services); the work lives here so tests can drive it with a
//! scripted LLM.
//!
//! **Cancel:** a user cancel resolves Ok with a no-changes review (the
//! glossary as it is, an empty diff), even when part of the work had finished.
//! The UI discards a review that resolves after the user stopped the op.

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::diff::GlossaryDiff;
use super::model::{Glossary, GlossaryDoc};
use super::normalize::{normalize_pass, NormalizeReview};
use super::personalize::personalize_pass;
use super::scope;
use crate::error::{AppError, AppResult};
use crate::events::{GlossaryEvent, GlossaryPhase};
use crate::llm::service::LlmService;

/// What the UI reviews: the whole glossary after the change, and the diff
/// against it as it is.
pub fn review(original: &Glossary, after: &Glossary) -> NormalizeReview {
    NormalizeReview {
        diff: GlossaryDiff::compute(Some(original), after),
        normalized: GlossaryDoc::from(after),
    }
}

fn terms(n: usize) -> String {
    if n == 1 { "1 term".into() } else { format!("{n} terms") }
}

/// Tidy up `sub` (the scoped part of `original`) and lay the answer back over
/// `original`. A category whose request fails keeps its terms (normalize_pass
/// logs it), so this never fails.
pub async fn tidy_up(
    svc: &LlmService,
    original: &Glossary,
    sub: &Glossary,
    templates: &std::collections::BTreeMap<String, String>,
    tx: &mpsc::Sender<GlossaryEvent>,
    cancel: &CancellationToken,
) -> NormalizeReview {
    let _ = tx
        .send(GlossaryEvent::Phase {
            phase: GlossaryPhase::Normalizing,
            detail: Some(terms(sub.count())),
        })
        .await;
    let tidied = normalize_pass(svc, sub, tx, templates).await;
    if cancel.is_cancelled() {
        return review(original, original);
    }
    review(original, &scope::overlay(original, sub, &tidied))
}

/// Look up names online for `sub` (the scoped part of `original`) and lay the
/// answer back over `original`. `show_name` is what the model searches for.
pub async fn look_up(
    svc: &LlmService,
    original: &Glossary,
    sub: &Glossary,
    show_name: &str,
    template: &str,
    tx: &mpsc::Sender<GlossaryEvent>,
    cancel: &CancellationToken,
) -> AppResult<NormalizeReview> {
    let _ = tx
        .send(GlossaryEvent::Phase {
            phase: GlossaryPhase::Personalizing,
            detail: Some(terms(sub.count())),
        })
        .await;
    match personalize_pass(svc, sub, show_name, template).await {
        _ if cancel.is_cancelled() => Ok(review(original, original)),
        Ok(found) => Ok(review(original, &scope::overlay(original, sub, &found))),
        Err(reason) => Err(AppError::Other(reason)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glossary::meta::{GlossaryScope, TermRef};
    use crate::llm::error::LlmError;
    use crate::llm::test_support::ScriptedDriver;
    use std::sync::Arc;

    fn svc(driver: Arc<ScriptedDriver>, cancel: &CancellationToken) -> LlmService {
        let (tx, _rx) = mpsc::channel(64);
        LlmService::new(driver, 1, cancel.clone(), tx)
    }

    fn original() -> Glossary {
        let mut g = Glossary::new("xianxia");
        g.characters.insert("林动".into(), "lin dong".into());
        g.characters.insert("应欢欢".into(), "Ying Huanhuan".into());
        g.locations.insert("青阳镇".into(), "qingyang town".into());
        g.skills.insert("大荒囚天指".into(), "Great Desolate Finger".into());
        g
    }

    fn templates() -> std::collections::BTreeMap<String, String> {
        crate::prompts::GlossaryPrompts::defaults().normalize
    }

    fn personalize_tpl() -> &'static str {
        crate::prompts::default_text(crate::prompts::PromptId::GlossaryPersonalize)
    }

    fn scope_terms(terms: &[(&str, &str)]) -> GlossaryScope {
        GlossaryScope::Terms {
            terms: terms
                .iter()
                .map(|(c, s)| TermRef { category: c.to_string(), source: s.to_string() })
                .collect(),
        }
    }

    fn drain(rx: &mut mpsc::Receiver<GlossaryEvent>) -> Vec<GlossaryEvent> {
        let mut out = Vec::new();
        while let Ok(e) = rx.try_recv() {
            out.push(e);
        }
        out
    }

    #[tokio::test(start_paused = true)]
    async fn tidy_up_one_category_sends_only_that_category() {
        let o = original();
        let sub =
            scope::resolve(&o, Some(&GlossaryScope::Category { category: "locations".into() }))
                .unwrap();
        let cancel = CancellationToken::new();
        // One category in scope → exactly one request.
        let d = ScriptedDriver::new(vec![Ok(r#"{"青阳镇":"Qingyang Town"}"#.into())]);
        let (tx, mut rx) = mpsc::channel(64);
        let r = tidy_up(&svc(d.clone(), &cancel), &o, &sub, &templates(), &tx, &cancel).await;

        assert_eq!(d.call_count(), 1);
        assert!(d.last_request().unwrap().user.contains("青阳镇"));
        assert!(!d.last_request().unwrap().user.contains("林动"));
        assert_eq!(r.normalized.terms["locations"]["青阳镇"], "Qingyang Town");
        assert_eq!(r.normalized.terms["characters"]["林动"], "lin dong", "out of scope");
        assert_eq!(r.normalized.count, 4, "the review holds the whole glossary");
        assert_eq!(r.diff.total_modified, 1);
        assert_eq!(r.diff.total_added + r.diff.total_removed, 0);
        assert!(drain(&mut rx).iter().any(|e| matches!(
            e,
            GlossaryEvent::Phase { phase: GlossaryPhase::Normalizing, detail: Some(d) } if d == "1 term"
        )));
    }

    #[tokio::test(start_paused = true)]
    async fn tidy_up_picked_terms_leaves_the_rest_of_their_category() {
        let o = original();
        let sub = scope::resolve(&o, Some(&scope_terms(&[("characters", "林动")]))).unwrap();
        let cancel = CancellationToken::new();
        // The model also answers about a term it wasn't given.
        let d = ScriptedDriver::new(vec![Ok(r#"{"林动":"Lin Dong","应欢欢":"WRONG"}"#.into())]);
        let (tx, _rx) = mpsc::channel(64);
        let r = tidy_up(&svc(d, &cancel), &o, &sub, &templates(), &tx, &cancel).await;
        assert_eq!(r.normalized.terms["characters"]["林动"], "Lin Dong");
        assert_eq!(r.normalized.terms["characters"]["应欢欢"], "Ying Huanhuan");
        assert_eq!(r.diff.total_modified, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn tidy_up_cancel_gives_a_no_changes_review() {
        let o = original();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let d = ScriptedDriver::new(vec![]); // nothing reaches the driver
        let (tx, _rx) = mpsc::channel(64);
        let r = tidy_up(&svc(d, &cancel), &o, &o, &templates(), &tx, &cancel).await;
        assert!(!r.diff.has_changes);
        assert_eq!(r.normalized.count, 4);
    }

    #[tokio::test(start_paused = true)]
    async fn look_up_sends_only_the_scope_and_overlays_the_answer() {
        let o = original();
        let sub = scope::resolve(&o, Some(&scope_terms(&[("characters", "林动"), ("locations", "青阳镇")])))
            .unwrap();
        let cancel = CancellationToken::new();
        let d = ScriptedDriver::new(vec![Ok(r#"{"terms":{
            "characters":{"林动":"Lin Dong","应欢欢":"WRONG"},
            "locations":{"青阳镇":"Qingyang Town"}}}"#
            .into())]);
        let (tx, mut rx) = mpsc::channel(64);
        let r = look_up(&svc(d.clone(), &cancel), &o, &sub, "Martial Universe", personalize_tpl(), &tx, &cancel)
            .await
            .unwrap();

        let req = d.last_request().unwrap();
        assert!(req.system.contains("Martial Universe"), "the show name is what gets searched");
        assert!(req.user.contains("林动") && req.user.contains("青阳镇"));
        assert!(!req.user.contains("应欢欢") && !req.user.contains("大荒囚天指"), "only the scope is sent");

        assert_eq!(r.normalized.terms["characters"]["林动"], "Lin Dong");
        assert_eq!(r.normalized.terms["locations"]["青阳镇"], "Qingyang Town");
        assert_eq!(r.normalized.terms["characters"]["应欢欢"], "Ying Huanhuan", "out of scope");
        assert_eq!(r.diff.total_modified, 2);
        assert!(drain(&mut rx).iter().any(|e| matches!(
            e,
            GlossaryEvent::Phase { phase: GlossaryPhase::Personalizing, detail: Some(d) } if d == "2 terms"
        )));
    }

    #[tokio::test(start_paused = true)]
    async fn look_up_failure_is_a_plain_error() {
        let o = original();
        let cancel = CancellationToken::new();
        let d = ScriptedDriver::new(vec![Ok("I couldn't find anything, sorry.".into())]);
        let (tx, _rx) = mpsc::channel(64);
        let err = look_up(&svc(d, &cancel), &o, &o, "Martial Universe", personalize_tpl(), &tx, &cancel)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.starts_with("Look up names online"), "{err}");
    }

    #[tokio::test(start_paused = true)]
    async fn look_up_cancel_gives_a_no_changes_review() {
        let o = original();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let d = ScriptedDriver::new(vec![Err(LlmError::Http {
            status: 500,
            body: "never sent".into(),
            retry_after: None,
        })]);
        let (tx, _rx) = mpsc::channel(64);
        let r = look_up(&svc(d, &cancel), &o, &o, "Martial Universe", personalize_tpl(), &tx, &cancel)
            .await
            .unwrap();
        assert!(!r.diff.has_changes);
        assert_eq!(r.normalized.count, 4);
    }
}
