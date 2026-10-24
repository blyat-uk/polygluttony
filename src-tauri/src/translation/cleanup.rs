//! Cleanup pass: re-translate lines still carrying source-language text
//! (`core/translator.py:583-684`). Skipped entirely when more than
//! MAX_CLEANUP_LINES need it (`translator.py:613-621`).

use std::collections::BTreeMap;

use crate::glossary::model::Glossary;
use crate::llm::service::LlmService;
use crate::translation::batch::{self, BatchFatal, BatchOutcome, BatchSettings};
use crate::translation::batching::{MAX_CLEANUP_ITERATIONS, MAX_CLEANUP_LINES};
use crate::translation::source_detect::SourceDetector;

#[derive(Debug)]
pub struct CleanupReport {
    pub cleaned: Vec<u32>,
    pub failed: Vec<u32>,
    pub skipped_too_many: bool,
    /// Auth death mid-cleanup: the pipeline must doom the run, not proceed
    /// to verify on a dead key (carry-forward fix).
    pub fatal: Option<BatchFatal>,
}

/// `sources`: id → raw source text (tags intact). `translations`: id → current
/// tagged translation; updated in place for every successfully cleaned line.
pub async fn cleanup_pass(
    svc: &LlmService,
    detector: &SourceDetector,
    sources: &BTreeMap<u32, String>,
    translations: &mut BTreeMap<u32, String>,
    glossary: &Glossary,
    settings: &BatchSettings,
) -> CleanupReport {
    let mut dirty: Vec<u32> = translations
        .iter()
        .filter(|(_, text)| detector.needs_cleanup(text))
        .map(|(id, _)| *id)
        .collect();

    if dirty.is_empty() {
        return CleanupReport { cleaned: vec![], failed: vec![], skipped_too_many: false, fatal: None };
    }
    if dirty.len() > MAX_CLEANUP_LINES {
        return CleanupReport { cleaned: vec![], failed: dirty, skipped_too_many: true, fatal: None };
    }

    let mut cleaned: Vec<u32> = Vec::new();
    let mut fatal: Option<BatchFatal> = None;
    for _ in 0..MAX_CLEANUP_ITERATIONS {
        let raw: Vec<(u32, String)> =
            dirty.iter().filter_map(|id| sources.get(id).map(|s| (*id, s.clone()))).collect();
        let outcome = batch::translate_batch_tagged(svc, &raw, glossary, &[], settings).await;
        let map = match outcome {
            BatchOutcome::Success(m) => m,
            BatchOutcome::Partial { translated, .. } => translated,
            // The run stopped: no point asking again.
            BatchOutcome::Failure(f) if f.cancelled => break,
            BatchOutcome::Failure(_) => continue,
            BatchOutcome::Fatal(f) => {
                fatal = Some(f);
                break;
            }
        };
        // Track which ids the LLM actually returned so that unreturned ids
        // (silently omitted in a Partial response) are kept dirty rather than
        // quietly dropping off the retry list.
        let returned: std::collections::BTreeSet<u32> = map.keys().copied().collect();
        let mut still_dirty = Vec::new();
        for (id, text) in map {
            if detector.needs_cleanup(&text) {
                still_dirty.push(id);
            } else {
                translations.insert(id, text);
                cleaned.push(id);
            }
        }
        // Keep an id dirty when: the LLM returned it but it still needs
        // cleanup (still_dirty), OR the LLM never returned it at all.
        dirty.retain(|id| still_dirty.contains(id) || !returned.contains(id));
        if dirty.is_empty() {
            break;
        }
    }
    CleanupReport { cleaned, failed: dirty, skipped_too_many: false, fatal }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::llm::test_support::ScriptedDriver;
    use crate::models::language_pair::LanguagePair;
    use tokio_util::sync::CancellationToken;

    fn service(
        responses: Vec<Result<String, crate::llm::error::LlmError>>,
    ) -> (LlmService, Arc<ScriptedDriver>) {
        let driver = ScriptedDriver::new(responses);
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        (LlmService::new(driver.clone(), 2, CancellationToken::new(), tx), driver)
    }

    fn settings() -> BatchSettings {
        BatchSettings {
            pair: LanguagePair::from_codes("zh", "en").unwrap(),
            template: crate::prompts::default_text(crate::prompts::PromptId::TranslateZhEn).into(),
            tone_text: crate::prompts::default_text(crate::prompts::PromptId::ToneStandard).into(),
        }
    }

    fn zh() -> SourceDetector {
        SourceDetector::for_language("zh").unwrap()
    }

    #[tokio::test]
    async fn skips_when_too_many_lines_are_dirty() {
        let (svc, driver) = service(vec![]);
        let sources: BTreeMap<u32, String> = (1..=11).map(|i| (i, "中文".to_string())).collect();
        let mut translations: BTreeMap<u32, String> =
            (1..=11).map(|i| (i, "还是中文的翻译".to_string())).collect();
        let report =
            cleanup_pass(&svc, &zh(), &sources, &mut translations, &Glossary::default(), &settings())
                .await;
        assert!(report.skipped_too_many);
        assert_eq!(report.failed.len(), 11);
        assert!(report.cleaned.is_empty());
        assert_eq!(driver.call_count(), 0);
    }

    #[tokio::test]
    async fn cleans_dirty_lines_in_place() {
        let (svc, driver) =
            service(vec![Ok(r#"[{"id":1,"tgt":"<0001:D> Clean now friend"}]"#.into())]);
        let sources: BTreeMap<u32, String> = [(1, "你好".to_string())].into();
        let mut translations: BTreeMap<u32, String> =
            [(1, "你好 leftover 中文 text".to_string())].into();
        let report =
            cleanup_pass(&svc, &zh(), &sources, &mut translations, &Glossary::default(), &settings())
                .await;
        assert_eq!(report.cleaned, vec![1]);
        assert!(report.failed.is_empty());
        assert!(!report.skipped_too_many);
        assert_eq!(translations.get(&1).unwrap(), "Clean now friend");
        assert_eq!(driver.call_count(), 1);
    }

    #[tokio::test]
    async fn gives_up_after_max_iterations() {
        let still_dirty = || Ok::<_, crate::llm::error::LlmError>(
            r#"[{"id":1,"tgt":"<0001:D> 还是中文"}]"#.to_string(),
        );
        let (svc, driver) = service(vec![still_dirty(), still_dirty(), still_dirty()]);
        let sources: BTreeMap<u32, String> = [(1, "你好".to_string())].into();
        let mut translations: BTreeMap<u32, String> = [(1, "全是中文的翻译".to_string())].into();
        let report =
            cleanup_pass(&svc, &zh(), &sources, &mut translations, &Glossary::default(), &settings())
                .await;
        assert!(report.cleaned.is_empty());
        assert_eq!(report.failed, vec![1]);
        // Dirty re-translations are never merged.
        assert_eq!(translations.get(&1).unwrap(), "全是中文的翻译");
        assert_eq!(driver.call_count(), MAX_CLEANUP_ITERATIONS);
    }

    #[tokio::test]
    async fn partial_response_keeps_unreturned_id_dirty_until_failed() {
        // Two dirty lines (ids 1 and 2). The scripted LLM returns only id 1
        // (clean) in every iteration, never mentioning id 2. Without the fix,
        // id 2 silently vanishes; with the fix it stays dirty and ends up in
        // `failed` once iterations exhaust.
        let clean_only_id1 =
            || Ok::<_, crate::llm::error::LlmError>(r#"[{"id":1,"tgt":"<0001:D> Clean now"}]"#.to_string());
        // Supply MAX_CLEANUP_ITERATIONS responses; id 2 is never returned.
        let responses: Vec<_> = (0..MAX_CLEANUP_ITERATIONS).map(|_| clean_only_id1()).collect();
        let (svc, driver) = service(responses);
        let sources: BTreeMap<u32, String> =
            [(1, "你好".to_string()), (2, "再见".to_string())].into();
        let mut translations: BTreeMap<u32, String> = [
            (1, "你好 leftover 中文".to_string()),
            (2, "再见 leftover 中文".to_string()),
        ]
        .into();
        let report =
            cleanup_pass(&svc, &zh(), &sources, &mut translations, &Glossary::default(), &settings())
                .await;
        // Id 1 was returned clean — cleaned.
        assert!(report.cleaned.contains(&1), "id 1 should be cleaned");
        // Id 2 was never returned — must appear in failed, not silently dropped.
        assert!(report.failed.contains(&2), "id 2 must be in failed (unreturned)");
        assert_eq!(driver.call_count(), MAX_CLEANUP_ITERATIONS);
    }

    #[tokio::test]
    async fn fatal_stops_early_instead_of_burning_iterations() {
        let (svc, driver) = service(vec![Err(crate::llm::error::LlmError::Http {
            status: 401,
            body: "no".into(),
            retry_after: None,
        })]);
        let sources: BTreeMap<u32, String> = [(1, "你好".to_string())].into();
        let mut translations: BTreeMap<u32, String> = [(1, "全是中文的翻译".to_string())].into();
        let report =
            cleanup_pass(&svc, &zh(), &sources, &mut translations, &Glossary::default(), &settings())
                .await;
        assert_eq!(report.failed, vec![1]);
        assert_eq!(driver.call_count(), 1); // not 3 — break, don't continue
        assert!(report.fatal.is_some(), "fatal must be surfaced to the pipeline");
    }
}
