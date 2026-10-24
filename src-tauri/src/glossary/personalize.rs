//! Look up names online (internally "personalize"; build step 8 and the
//! standalone Improve action): one call on the web-capable connection to
//! align terms with established fan usage. Port of
//! `glossary_builder.py:535-563`.

use crate::glossary::model::Glossary;
use crate::glossary::prompts;
use crate::llm::service::LlmService;
use crate::llm::LlmRequest;
use crate::translation::parse_response;

/// `Err(reason)` ⇒ caller keeps the original glossary and records the reason
/// (plain words: it reaches the user as is).
/// A response with zero usable terms is an error — accepting it would replace
/// the whole glossary with nothing (latent Python hazard, guarded here).
///
/// **Deviation:** Python honoured the response's own `world_type` field
/// (`glossary.py:237`, defaulting to "xianxia" when absent). We always
/// preserve the original glossary's `world_type` — user overrides must win.
pub async fn personalize_pass(
    svc: &LlmService,
    glossary: &Glossary,
    context: &str,
    template: &str,
) -> Result<Glossary, String> {
    let world =
        if glossary.world_type.is_empty() { "modern" } else { glossary.world_type.as_str() };
    let req = LlmRequest {
        system: prompts::personalize_prompt(template, world, context),
        user: prompts::personalize_user_prompt(glossary, context),
    };
    let resp = svc.request(req).await.map_err(|e| format!("Look up names online failed: {e}"))?;
    let v = parse_response::extract_object(&resp.text)
        .map_err(|e| format!("Look up names online gave an answer that couldn't be read ({e})."))?;
    let mut out = Glossary::from_terms_value(&v, &glossary.world_type);
    if out.is_empty() {
        return Err("Look up names online answered with no terms, so nothing was changed.".into());
    }
    // Remove entries with empty/invalid translations so that merge_first_wins
    // below restores the original's valid value for those keys.
    out.scrub_invalid();
    // Merge-over: response terms win (they're the personalized versions); any
    // term the response omitted or returned invalid is restored from the
    // original (carry-forward fix for truncated/partial responses).
    out.merge_first_wins(glossary);
    Ok(out)
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

    fn svc(driver: Arc<ScriptedDriver>) -> LlmService {
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        LlmService::new(driver, 1, CancellationToken::new(), tx)
    }

    fn glossary() -> Glossary {
        let mut g = Glossary::new("xianxia");
        g.characters.insert("林动".into(), "Lin Dong".into());
        g
    }

    fn personalize_tpl() -> &'static str {
        crate::prompts::default_text(crate::prompts::PromptId::GlossaryPersonalize)
    }

    #[tokio::test(start_paused = true)]
    async fn replaces_glossary_on_success() {
        let d = ScriptedDriver::new(vec![Ok(
            r#"{"world_type":"xianxia","terms":{"characters":{"林动":"Lin Dong (MC)"}}}"#.into(),
        )]);
        let out = personalize_pass(&svc(d.clone()), &glossary(), "Martial Universe", personalize_tpl()).await.unwrap();
        assert_eq!(out.characters.get("林动").unwrap(), "Lin Dong (MC)");
        // Prompt carried the title + glossary JSON.
        let req = d.last_request().unwrap();
        assert!(req.system.contains("Martial Universe"));
        assert!(req.user.contains("Lin Dong"));
    }

    #[tokio::test(start_paused = true)]
    async fn request_failure_returns_err() {
        let d = ScriptedDriver::new(vec![Err(LlmError::Http { status: 401, body: "no".into(), retry_after: None })]);
        assert!(personalize_pass(&svc(d), &glossary(), "", personalize_tpl()).await.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn empty_or_unparseable_response_returns_err() {
        let d = ScriptedDriver::new(vec![Ok("sorry, no JSON".into())]);
        assert!(personalize_pass(&svc(d), &glossary(), "", personalize_tpl()).await.is_err());
        // Parses but contains zero terms → would wipe the glossary → Err.
        let d = ScriptedDriver::new(vec![Ok(r#"{"terms":{}}"#.into())]);
        assert!(personalize_pass(&svc(d), &glossary(), "", personalize_tpl()).await.is_err());
    }

    /// A truncated response must not drop terms it did not mention.
    /// Response terms win (they're the personalized versions); terms the
    /// response omitted are carried forward from the original.
    #[tokio::test(start_paused = true)]
    async fn truncated_response_keeps_omitted_terms() {
        // Original: one character + one location.
        let mut original = Glossary::new("xianxia");
        original.characters.insert("林动".into(), "Lin Dong".into());
        original.locations.insert("青阳镇".into(), "Qingyang Town".into());

        // Response: only the character, renamed — location is omitted (truncation).
        let d = ScriptedDriver::new(vec![Ok(
            r#"{"characters":{"林动":"Lin Dong (Rock Saint)"}}"#.into(),
        )]);
        let out = personalize_pass(&svc(d), &original, "ctx", personalize_tpl()).await.unwrap();

        // (a) Response's rename wins.
        assert_eq!(out.characters.get("林动").unwrap(), "Lin Dong (Rock Saint)");
        // (b) Omitted term survives from original.
        assert_eq!(out.locations.get("青阳镇").unwrap(), "Qingyang Town");
        // (c) world_type preserved.
        assert_eq!(out.world_type, "xianxia");
    }

    /// An LLM response that returns a key from the original glossary with an
    /// empty/invalid translation must NOT overwrite the original's valid value.
    /// After scrubbing, `merge_first_wins` restores the original's translation.
    #[tokio::test(start_paused = true)]
    async fn empty_response_value_restores_original_translation() {
        // The response "speaks" about 林动 but with an empty value — the original
        // translation must survive, not the invalid response entry.
        let mut original = Glossary::new("xianxia");
        original.characters.insert("林动".into(), "Lin Dong".into());
        original.characters.insert("小炎".into(), "Xiao Yan".into());

        let d = ScriptedDriver::new(vec![Ok(
            r#"{"characters":{"林动":"","小炎":"Little Flame"}}"#.into(),
        )]);
        let out = personalize_pass(&svc(d), &original, "ctx", personalize_tpl()).await.unwrap();

        // 林动's empty response value must be discarded; original "Lin Dong" restored.
        assert_eq!(out.characters.get("林动").unwrap(), "Lin Dong");
        // 小炎's valid new term is kept.
        assert_eq!(out.characters.get("小炎").unwrap(), "Little Flame");
    }

    /// Custom personalize template reaches the wire: a marked template must
    /// appear in req.system with {donghua_title} and {world_type} filled.
    #[tokio::test(start_paused = true)]
    async fn custom_personalize_template_reaches_the_request() {
        let d = ScriptedDriver::new(vec![Ok(
            r#"{"terms":{"characters":{"林动":"Lin Dong (MC)"}}}"#.into(),
        )]);
        let tpl = "XPERSX {donghua_title} {world_type}";
        let out = personalize_pass(&svc(d.clone()), &glossary(), "Martial Universe", tpl).await;
        assert!(out.is_ok(), "personalize must succeed with valid response");
        let req = d.last_request().unwrap();
        assert!(
            req.system.starts_with("XPERSX"),
            "custom personalize template must reach the wire: {:?}",
            req.system
        );
        assert!(
            !req.system.contains("{donghua_title}"),
            "donghua_title placeholder must be filled: {:?}",
            req.system
        );
        assert!(
            !req.system.contains("{world_type}"),
            "world_type placeholder must be filled: {:?}",
            req.system
        );
        assert!(req.system.contains("Martial Universe"), "title must appear: {:?}", req.system);
        assert!(req.system.contains("xianxia"), "world value must appear: {:?}", req.system);
    }

    /// Pin the "modern" fallback: a glossary with an empty world_type should
    /// send "modern" in the system prompt.
    #[tokio::test(start_paused = true)]
    async fn empty_world_type_uses_modern_in_prompt() {
        let mut g = Glossary::new("");
        g.characters.insert("林动".into(), "Lin Dong".into());
        let d = ScriptedDriver::new(vec![Ok(
            r#"{"terms":{"characters":{"林动":"Lin Dong"}}}"#.into(),
        )]);
        personalize_pass(&svc(d.clone()), &g, "", personalize_tpl()).await.unwrap();
        let req = d.last_request().unwrap();
        assert!(req.system.contains("modern"), "expected 'modern' in system prompt");
    }
}
