//! Prompt catalog: the single source of truth for every LLM prompt template.
//!
//! Owns the embedded defaults (`include_str!`), the per-prompt override file
//! location under `<app-data>/prompts/`, and the placeholder specs that drive
//! BOTH save-validation and the Settings editor's help text. Jobs resolve
//! their templates once at start via [`resolve`] / [`TranslationPrompts`] /
//! [`GlossaryPrompts`] — a running job never sees mid-job edits.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::Manager;
use ts_rs::TS;

use crate::config::projects::Tone;
use crate::error::{AppError, AppResult};
use crate::models::language_pair::LanguagePair;

/// Stable identifier for each editable prompt template. Serialized kebab-case
/// across IPC (e.g. `"translate-zh-en"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum PromptId {
    TranslateZhEn,
    TranslateGeneric,
    ToneStandard,
    ToneXianxia,
    ToneWuxia,
    ToneComedic,
    ToneFunny,
    GlossaryExtract,
    GlossaryNormalizeCharacters,
    GlossaryNormalizeCultivation,
    GlossaryNormalizeSkills,
    GlossaryNormalizeLocations,
    GlossaryNormalizeItems,
    GlossaryNormalizeOrganizations,
    GlossaryPersonalize,
    ReferenceExtract,
    Verify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum PromptGroup {
    Translation,
    Tones,
    Glossary,
    Verify,
}

/// Compile-time placeholder spec. `description` is the help-popover text:
/// what the tag represents and what it is replaced by at runtime.
pub struct Placeholder {
    pub token: &'static str,
    pub required: bool,
    pub description: &'static str,
}

pub struct Entry {
    pub id: PromptId,
    pub name: &'static str,
    pub group: PromptGroup,
    /// Override path relative to `<app-data>/prompts/` — mirrors the embedded
    /// asset path under `src-tauri/prompts/`.
    pub file: &'static str,
    pub default: &'static str,
    pub placeholders: &'static [Placeholder],
}

const WORLD_DESC: &str = "Replaced with the story's world type (xianxia, wuxia, \
historical, modern, …) — auto-detected from the subtitles, overridable in Project.";
const TONE_DESC: &str = "Replaced with the tone guidelines for the folder's selected \
tone (Standard, Xianxia, Wuxia, Comedic, Funny — each editable under Tones).";
const SRC_LANG_DESC: &str = "Replaced with the source language name, e.g. \"Chinese\".";
const TGT_LANG_DESC: &str = "Replaced with the target language name, e.g. \"English\".";

static ENTRIES: [Entry; 17] = [
    Entry {
        id: PromptId::TranslateZhEn,
        name: "Chinese → English",
        group: PromptGroup::Translation,
        file: "translate.zh-en.txt",
        default: include_str!("../../prompts/translate.zh-en.txt"),
        placeholders: &[
            Placeholder {
                token: "{GLOSSARY}",
                required: true,
                description: "Replaced with the glossary terms found in the lines being \
translated — one \"中文 → English\" line per term. Without it the glossary never \
reaches the model.",
            },
            Placeholder { token: "{TONE}", required: true, description: TONE_DESC },
        ],
    },
    Entry {
        id: PromptId::TranslateGeneric,
        name: "Generic (any pair)",
        group: PromptGroup::Translation,
        file: "translate.generic.txt",
        default: include_str!("../../prompts/translate.generic.txt"),
        placeholders: &[
            Placeholder { token: "{source_language}", required: true, description: SRC_LANG_DESC },
            Placeholder { token: "{target_language}", required: true, description: TGT_LANG_DESC },
            Placeholder { token: "{localization_style}", required: true, description: TONE_DESC },
        ],
    },
    Entry {
        id: PromptId::ToneStandard,
        name: "Standard",
        group: PromptGroup::Tones,
        file: "tones/standard.txt",
        default: include_str!("../../prompts/tones/standard.txt"),
        placeholders: &[],
    },
    Entry {
        id: PromptId::ToneXianxia,
        name: "Xianxia",
        group: PromptGroup::Tones,
        file: "tones/xianxia.txt",
        default: include_str!("../../prompts/tones/xianxia.txt"),
        placeholders: &[],
    },
    Entry {
        id: PromptId::ToneWuxia,
        name: "Wuxia",
        group: PromptGroup::Tones,
        file: "tones/wuxia.txt",
        default: include_str!("../../prompts/tones/wuxia.txt"),
        placeholders: &[],
    },
    Entry {
        id: PromptId::ToneComedic,
        name: "Comedic",
        group: PromptGroup::Tones,
        file: "tones/comedic.txt",
        default: include_str!("../../prompts/tones/comedic.txt"),
        placeholders: &[],
    },
    Entry {
        id: PromptId::ToneFunny,
        name: "Funny",
        group: PromptGroup::Tones,
        file: "tones/funny.txt",
        default: include_str!("../../prompts/tones/funny.txt"),
        placeholders: &[],
    },
    Entry {
        id: PromptId::GlossaryExtract,
        name: "Extraction",
        group: PromptGroup::Glossary,
        file: "glossary.txt",
        default: include_str!("../../prompts/glossary.txt"),
        placeholders: &[
            Placeholder { token: "{world_type}", required: true, description: WORLD_DESC },
            Placeholder {
                token: "{reference_terminology}",
                required: false,
                description: "Replaced with established terms imported from previously \
translated episodes (the Reference review in Glossary). When none exist, the entire \
\"## REFERENCE TERMINOLOGY\" section is removed before sending.",
            },
            Placeholder { token: "{source_language}", required: false, description: SRC_LANG_DESC },
            Placeholder { token: "{target_language}", required: false, description: TGT_LANG_DESC },
        ],
    },
    Entry {
        id: PromptId::GlossaryNormalizeCharacters,
        name: "Normalize: Characters",
        group: PromptGroup::Glossary,
        file: "glossary-normalize-characters.txt",
        default: include_str!("../../prompts/glossary-normalize-characters.txt"),
        placeholders: &[Placeholder { token: "{world_type}", required: true, description: WORLD_DESC }],
    },
    Entry {
        id: PromptId::GlossaryNormalizeCultivation,
        name: "Normalize: Cultivation",
        group: PromptGroup::Glossary,
        file: "glossary-normalize-cultivation.txt",
        default: include_str!("../../prompts/glossary-normalize-cultivation.txt"),
        placeholders: &[Placeholder { token: "{world_type}", required: true, description: WORLD_DESC }],
    },
    Entry {
        id: PromptId::GlossaryNormalizeSkills,
        name: "Normalize: Skills",
        group: PromptGroup::Glossary,
        file: "glossary-normalize-skills.txt",
        default: include_str!("../../prompts/glossary-normalize-skills.txt"),
        placeholders: &[Placeholder { token: "{world_type}", required: true, description: WORLD_DESC }],
    },
    Entry {
        id: PromptId::GlossaryNormalizeLocations,
        name: "Normalize: Locations",
        group: PromptGroup::Glossary,
        file: "glossary-normalize-locations.txt",
        default: include_str!("../../prompts/glossary-normalize-locations.txt"),
        placeholders: &[Placeholder { token: "{world_type}", required: true, description: WORLD_DESC }],
    },
    Entry {
        id: PromptId::GlossaryNormalizeItems,
        name: "Normalize: Items",
        group: PromptGroup::Glossary,
        file: "glossary-normalize-items.txt",
        default: include_str!("../../prompts/glossary-normalize-items.txt"),
        placeholders: &[Placeholder { token: "{world_type}", required: true, description: WORLD_DESC }],
    },
    Entry {
        id: PromptId::GlossaryNormalizeOrganizations,
        name: "Normalize: Organizations",
        group: PromptGroup::Glossary,
        file: "glossary-normalize-organizations.txt",
        default: include_str!("../../prompts/glossary-normalize-organizations.txt"),
        placeholders: &[Placeholder { token: "{world_type}", required: true, description: WORLD_DESC }],
    },
    Entry {
        id: PromptId::GlossaryPersonalize,
        name: "Personalize",
        group: PromptGroup::Glossary,
        file: "glossary-personalize.txt",
        default: include_str!("../../prompts/glossary-personalize.txt"),
        placeholders: &[
            Placeholder {
                token: "{donghua_title}",
                required: true,
                description: "Replaced with the series title — taken from the first line \
of the personalize context box in Glossary (\"Unknown\" when empty).",
            },
            Placeholder { token: "{world_type}", required: true, description: WORLD_DESC },
        ],
    },
    Entry {
        id: PromptId::ReferenceExtract,
        name: "Reference extract",
        group: PromptGroup::Glossary,
        file: "reference-extract.txt",
        default: include_str!("../../prompts/reference-extract.txt"),
        placeholders: &[],
    },
    Entry {
        id: PromptId::Verify,
        name: "Drift check",
        group: PromptGroup::Verify,
        file: "verify.txt",
        default: include_str!("../../prompts/verify.txt"),
        placeholders: &[],
    },
];

/// Exhaustive by construction: adding a `PromptId` variant without a catalog
/// entry is a compile error, and the test below pins each arm to the entry
/// with the matching id.
pub fn entry(id: PromptId) -> &'static Entry {
    let e = match id {
        PromptId::TranslateZhEn => &ENTRIES[0],
        PromptId::TranslateGeneric => &ENTRIES[1],
        PromptId::ToneStandard => &ENTRIES[2],
        PromptId::ToneXianxia => &ENTRIES[3],
        PromptId::ToneWuxia => &ENTRIES[4],
        PromptId::ToneComedic => &ENTRIES[5],
        PromptId::ToneFunny => &ENTRIES[6],
        PromptId::GlossaryExtract => &ENTRIES[7],
        PromptId::GlossaryNormalizeCharacters => &ENTRIES[8],
        PromptId::GlossaryNormalizeCultivation => &ENTRIES[9],
        PromptId::GlossaryNormalizeSkills => &ENTRIES[10],
        PromptId::GlossaryNormalizeLocations => &ENTRIES[11],
        PromptId::GlossaryNormalizeItems => &ENTRIES[12],
        PromptId::GlossaryNormalizeOrganizations => &ENTRIES[13],
        PromptId::GlossaryPersonalize => &ENTRIES[14],
        PromptId::ReferenceExtract => &ENTRIES[15],
        PromptId::Verify => &ENTRIES[16],
    };
    debug_assert!(e.id == id, "ENTRIES order must match the entry() arms");
    e
}

/// The whole catalog, in display order.
pub fn entries() -> &'static [Entry] {
    &ENTRIES
}

pub fn default_text(id: PromptId) -> &'static str {
    entry(id).default
}

// ---- id helpers (selection logic that used to live beside the statics) -----

/// Template selection: `translate.{src}-{tgt}.txt` → `translate.generic.txt`.
pub fn translate_id(pair: &LanguagePair) -> PromptId {
    match (pair.source.as_str(), pair.target.as_str()) {
        ("zh", "en") => PromptId::TranslateZhEn,
        _ => PromptId::TranslateGeneric,
    }
}

pub fn tone_id(tone: Tone) -> PromptId {
    match tone {
        Tone::Standard => PromptId::ToneStandard,
        Tone::Xianxia => PromptId::ToneXianxia,
        Tone::Wuxia => PromptId::ToneWuxia,
        Tone::Comedic => PromptId::ToneComedic,
        Tone::Funny => PromptId::ToneFunny,
    }
}

/// Panics on an unknown category — mirrors the `unreachable!` the normalize
/// prompt selection has always had (categories come from `CATEGORIES`).
pub fn normalize_id(category: &str) -> PromptId {
    match category {
        "characters" => PromptId::GlossaryNormalizeCharacters,
        "cultivation" => PromptId::GlossaryNormalizeCultivation,
        "skills" => PromptId::GlossaryNormalizeSkills,
        "locations" => PromptId::GlossaryNormalizeLocations,
        "items" => PromptId::GlossaryNormalizeItems,
        "organizations" => PromptId::GlossaryNormalizeOrganizations,
        _ => unreachable!("unknown glossary category: {category}"),
    }
}

// ---- resolution -------------------------------------------------------------

/// `<app-data>/prompts/` — where user overrides live.
pub fn overrides_dir(app: &tauri::AppHandle) -> AppResult<PathBuf> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Other(format!("no app-data dir: {e}")))?;
    Ok(base.join("prompts"))
}

/// Override file if present, else the embedded default. An override that
/// exists but cannot be read (permissions, invalid UTF-8) is a HARD error —
/// silently translating with the wrong prompt is invisible drift.
pub fn resolve(id: PromptId, overrides_dir: &Path) -> AppResult<String> {
    let e = entry(id);
    let path = overrides_dir.join(e.file);
    if path.is_file() {
        std::fs::read_to_string(&path).map_err(|err| {
            AppError::Other(format!(
                "custom \"{}\" prompt exists but can't be read ({err}) — fix or restore \
it on the Prompts page",
                e.name
            ))
        })
    } else {
        Ok(e.default.to_string())
    }
}

// ---- single-pass placeholder substitution ----------------------------------

/// Single-pass placeholder substitution. Scans `template` for `{token}`
/// patterns (ASCII letters/underscore), looks the token name up
/// case-insensitively in `values` (keys must be lowercase), and inserts the
/// value VERBATIM — inserted values are never re-scanned, so a glossary line
/// containing "{tone}" can't trigger a second substitution. Unknown tokens
/// (e.g. `{established_terms}`) are left intact for the caller to handle.
pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start..]; // starts at '{'
        if let Some(end) = after.find('}') {
            let inner = &after[1..end];
            if !inner.is_empty() && inner.chars().all(|c| c.is_ascii_alphabetic() || c == '_') {
                let key = inner.to_ascii_lowercase();
                if let Some((_, v)) = values.iter().find(|(k, _)| *k == key) {
                    out.push_str(v);
                    rest = &after[end + 1..];
                    continue;
                }
            }
            // Not a known token: emit the '{' literally and keep scanning after it.
            out.push('{');
            rest = &after[1..];
        } else {
            out.push_str(after);
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

// ---- validation -------------------------------------------------------------

/// Placeholder matching is case-tolerant: the engine fills both `{WORLD_TYPE}`
/// and `{world_type}` forms, so either satisfies the requirement.
fn contains_either_case(text: &str, token: &str) -> bool {
    text.contains(token.to_lowercase().as_str()) || text.contains(token.to_uppercase().as_str())
}

/// Reject empty prompts and prompts missing a required placeholder. The UI
/// blocks Save on the same conditions; this is the authoritative check.
pub fn validate(id: PromptId, text: &str) -> AppResult<()> {
    if text.trim().is_empty() {
        return Err(AppError::Other("prompt is empty".into()));
    }
    let missing: Vec<&str> = entry(id)
        .placeholders
        .iter()
        .filter(|p| p.required && !contains_either_case(text, p.token))
        .map(|p| p.token)
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(AppError::Other(format!(
            "missing required placeholder{}: {}",
            if missing.len() > 1 { "s" } else { "" },
            missing.join(", ")
        )))
    }
}

// ---- per-job prompt packs (resolved once at job start) ----------------------

/// Everything a translation run sends as system-prompt material.
pub struct TranslationPrompts {
    /// The translate.* template for the run's language pair.
    pub template: String,
    /// The tone guideline text for the folder's selected tone.
    pub tone: String,
    /// The verify (drift check) system prompt.
    pub verify: String,
}

impl TranslationPrompts {
    pub fn resolve(dir: &Path, pair: &LanguagePair, tone: Tone) -> AppResult<Self> {
        Ok(TranslationPrompts {
            template: resolve(translate_id(pair), dir)?,
            tone: resolve(tone_id(tone), dir)?,
            verify: resolve(PromptId::Verify, dir)?,
        })
    }

    /// Embedded defaults — for tests.
    #[cfg(test)]
    pub fn defaults(pair: &LanguagePair, tone: Tone) -> Self {
        TranslationPrompts {
            template: default_text(translate_id(pair)).to_string(),
            tone: default_text(tone_id(tone)).to_string(),
            verify: default_text(PromptId::Verify).to_string(),
        }
    }
}

/// Everything a glossary op may send as system-prompt material.
pub struct GlossaryPrompts {
    pub extract: String,
    /// category name → resolved normalize template (all six, always).
    pub normalize: BTreeMap<String, String>,
    pub personalize: String,
    pub reference: String,
}

impl GlossaryPrompts {
    /// Just the six per-category normalize templates — for standalone
    /// normalize (O12), so an unreadable override of an UNRELATED prompt
    /// (e.g. Extraction) can't fail an op that never uses it.
    pub fn resolve_normalize(dir: &Path) -> AppResult<BTreeMap<String, String>> {
        let mut normalize = BTreeMap::new();
        for c in crate::glossary::model::CATEGORIES {
            normalize.insert(c.to_string(), resolve(normalize_id(c), dir)?);
        }
        Ok(normalize)
    }

    pub fn resolve(dir: &Path) -> AppResult<Self> {
        Ok(GlossaryPrompts {
            extract: resolve(PromptId::GlossaryExtract, dir)?,
            normalize: Self::resolve_normalize(dir)?,
            personalize: resolve(PromptId::GlossaryPersonalize, dir)?,
            reference: resolve(PromptId::ReferenceExtract, dir)?,
        })
    }

    /// Embedded defaults — for tests.
    #[cfg(test)]
    pub fn defaults() -> Self {
        let mut normalize = BTreeMap::new();
        for c in crate::glossary::model::CATEGORIES {
            normalize.insert(c.to_string(), default_text(normalize_id(c)).to_string());
        }
        GlossaryPrompts {
            extract: default_text(PromptId::GlossaryExtract).to_string(),
            normalize,
            personalize: default_text(PromptId::GlossaryPersonalize).to_string(),
            reference: default_text(PromptId::ReferenceExtract).to_string(),
        }
    }
}

// ---- Settings view-models ----------------------------------------------------

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct PlaceholderSpec {
    pub token: String,
    pub required: bool,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct PromptMeta {
    pub id: PromptId,
    pub name: String,
    pub group: PromptGroup,
    /// An override file exists for this prompt.
    pub modified: bool,
    pub placeholders: Vec<PlaceholderSpec>,
}

/// Catalog → list view-model, in display order (ENTRIES order is the UI order).
pub fn list_meta(overrides_dir: &Path) -> Vec<PromptMeta> {
    ENTRIES
        .iter()
        .map(|e| PromptMeta {
            id: e.id,
            name: e.name.to_string(),
            group: e.group,
            modified: overrides_dir.join(e.file).is_file(),
            placeholders: e
                .placeholders
                .iter()
                .map(|p| PlaceholderSpec {
                    token: p.token.to_string(),
                    required: p.required,
                    description: p.description.to_string(),
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The catalog's required-placeholder specs must hold for the embedded
    /// defaults themselves — catches catalog ↔ template drift forever.
    #[test]
    fn defaults_are_nonempty_and_contain_required_placeholders() {
        for e in &ENTRIES {
            assert!(!e.default.trim().is_empty(), "{:?}: empty default", e.id);
            for p in e.placeholders.iter().filter(|p| p.required) {
                assert!(
                    contains_either_case(e.default, p.token),
                    "{:?}: default missing required {}",
                    e.id,
                    p.token
                );
            }
            validate(e.id, e.default).unwrap_or_else(|err| {
                panic!("{:?}: default fails its own validation: {err}", e.id)
            });
        }
    }

    #[test]
    fn entries_cover_all_ids_with_unique_files_and_correct_arms() {
        let files: std::collections::BTreeSet<&str> = ENTRIES.iter().map(|e| e.file).collect();
        assert_eq!(files.len(), ENTRIES.len(), "override paths must be unique");
        // Every entry's id round-trips through entry() to itself — pins the
        // match-arm indices to the ENTRIES order (the debug_assert's release-
        // mode counterpart) and implies id uniqueness.
        for e in &ENTRIES {
            assert!(std::ptr::eq(entry(e.id), e), "{:?}: entry() returns the wrong Entry", e.id);
        }
    }

    #[test]
    fn resolve_prefers_override_and_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        // No override → embedded default.
        let d = resolve(PromptId::Verify, dir.path()).unwrap();
        assert_eq!(d, default_text(PromptId::Verify));
        // Override wins (including nested tones/ paths).
        std::fs::create_dir_all(dir.path().join("tones")).unwrap();
        std::fs::write(dir.path().join("tones/xianxia.txt"), "custom tone").unwrap();
        assert_eq!(resolve(PromptId::ToneXianxia, dir.path()).unwrap(), "custom tone");
        // list_meta reflects the override as modified=true (and others stay false).
        let meta = list_meta(dir.path());
        assert!(meta.iter().find(|m| m.id == PromptId::ToneXianxia).unwrap().modified);
        assert!(!meta.iter().find(|m| m.id == PromptId::Verify).unwrap().modified);
    }

    #[test]
    fn resolve_errors_on_unreadable_override() {
        let dir = tempfile::tempdir().unwrap();
        // Invalid UTF-8 → read_to_string fails → hard error naming the prompt.
        std::fs::write(dir.path().join("verify.txt"), [0xff, 0xfe, 0xfd]).unwrap();
        let err = resolve(PromptId::Verify, dir.path()).unwrap_err();
        assert!(err.to_string().contains("Drift check"), "error names the prompt: {err}");
    }

    #[test]
    fn validate_blocks_empty_and_missing_required_tokens() {
        assert!(validate(PromptId::TranslateZhEn, "   \n ").is_err());
        let err = validate(PromptId::TranslateZhEn, "translate well, use {TONE}").unwrap_err();
        assert!(err.to_string().contains("{GLOSSARY}"), "{err}");
        // Case-tolerant: lowercase satisfies an uppercase canonical token.
        validate(PromptId::TranslateZhEn, "{glossary} and {tone}").unwrap();
        // Optional tokens never block.
        validate(PromptId::GlossaryExtract, "extract terms for {world_type}").unwrap();
        // Token-less prompts only require non-empty text.
        validate(PromptId::Verify, "check the lines").unwrap();
    }

    #[test]
    fn fill_is_single_pass_case_insensitive_and_leaves_unknown_tokens() {
        // Any case variant of a known token fills.
        let r = fill("{GLOSSARY}|{Tone}|{tone}", &[("glossary", "G"), ("tone", "T")]);
        assert_eq!(r, "G|T|T");
        // Inserted values are never re-scanned.
        let r = fill("{glossary} {tone}", &[("glossary", "has {tone} inside"), ("tone", "T")]);
        assert_eq!(r, "has {tone} inside T");
        // Unknown tokens and non-tokens stay intact.
        let r = fill("{established_terms} {} {123} x", &[("tone", "T")]);
        assert_eq!(r, "{established_terms} {} {123} x");
    }

    #[test]
    fn id_helpers_match_selection_rules() {
        let zh_en = LanguagePair::from_codes("zh", "en").unwrap();
        let ko_en = LanguagePair::from_codes("ko", "en").unwrap();
        assert_eq!(translate_id(&zh_en), PromptId::TranslateZhEn);
        assert_eq!(translate_id(&ko_en), PromptId::TranslateGeneric);
        assert_eq!(tone_id(Tone::Wuxia), PromptId::ToneWuxia);
        for c in crate::glossary::model::CATEGORIES {
            normalize_id(c); // must not panic for any real category
        }
    }
}
