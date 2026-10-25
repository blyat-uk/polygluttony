//! Folder-pickup commands (O6/O7/O8) + per-folder persistence. Thin wrappers over
//! the tested engine modules; `open_folder` bundles discovery + counts + detection.

use std::path::{Path, PathBuf};

use tauri::AppHandle;

use crate::ass::{decode::decode_file, parse::parse_dialogues, tags::strip_for_text};
use crate::config::languages::{detect_source_language, get_language, languages, Language};
use crate::config::projects::{self, FolderPrefs, RecentFolder};
use crate::config::store as config_store;
use crate::error::{AppError, AppResult};
use crate::glossary::world_detector::detect;
use crate::models::language_pair::LanguagePair;
use crate::models::{ProjectView, SourceFile, TranslationState};
use crate::utils::discover::{discover_source_files, output_on_disk};

#[tauri::command]
pub fn list_languages() -> Vec<Language> {
    // Sorted by display name for the UI selects. The canonical `languages()`
    // order is preserved there (source detection relies on its tie-break order).
    let mut langs = languages();
    langs.sort_by(|a, b| a.name.cmp(&b.name));
    langs
}

#[tauri::command]
pub fn list_recents(app: AppHandle) -> AppResult<Vec<RecentFolder>> {
    let mut cfg = projects::load(&app)?;
    let before = cfg.recents.len();
    projects::prune_recents(&mut cfg, |p| Path::new(p).is_dir());
    if cfg.recents.len() != before {
        projects::save(&app, &cfg)?;
    }
    Ok(cfg.recents)
}

#[tauri::command]
pub fn remove_recent(app: AppHandle, path: String) -> AppResult<()> {
    let mut cfg = projects::load(&app)?;
    projects::remove_recent(&mut cfg, &path);
    projects::save(&app, &cfg)
}

#[tauri::command]
pub fn clear_recents(app: AppHandle) -> AppResult<()> {
    let mut cfg = projects::load(&app)?;
    projects::clear_recents(&mut cfg);
    projects::save(&app, &cfg)
}

#[tauri::command]
pub fn save_folder_prefs(app: AppHandle, path: String, prefs: FolderPrefs) -> AppResult<()> {
    let mut cfg = projects::load(&app)?;
    projects::set_prefs(&mut cfg, &path, prefs);
    projects::save(&app, &cfg)
}

/// Persist the source/target pair as the global default — used to seed new
/// folders and remembered across sessions.
#[tauri::command]
pub fn set_default_languages(app: AppHandle, source: String, target: String) -> AppResult<()> {
    let mut cfg = config_store::load(&app)?;
    config_store::set_default_languages(&mut cfg, &source, &target);
    config_store::save(&app, &cfg)
}

#[tauri::command]
pub async fn open_folder(app: AppHandle, path: String, now: i64) -> AppResult<ProjectView> {
    let dir = PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(AppError::Other("please choose a folder, not a file".into()));
    }

    let app_cfg = config_store::load(&app)?;
    let projects_cfg = projects::load(&app)?;
    let saved = projects::get_prefs(&projects_cfg, &path);

    // Discovery uses the saved-or-default language pair (fall back to zh→en).
    let src_code = saved
        .as_ref()
        .map(|p| p.source_lang.clone())
        .unwrap_or_else(|| app_cfg.default_source.clone());
    let tgt_code = saved
        .as_ref()
        .map(|p| p.target_lang.clone())
        .unwrap_or_else(|| app_cfg.default_target.clone());
    let pair = LanguagePair::from_codes(&src_code, &tgt_code)
        .or_else(|_| LanguagePair::from_codes("zh", "en"))?;

    // Decode + parse every file off the async runtime thread (we're inside a
    // Tauri async command, which runs on the tokio runtime, so tokio's
    // spawn_blocking is available and its JoinError implements Display).
    let dir_for_blocking = dir.clone();
    let pair_for_blocking = pair.clone();
    let analyzed = tokio::task::spawn_blocking(move || {
        analyze_folder(&dir_for_blocking, &pair_for_blocking)
    })
    .await
    .map_err(|e| AppError::Other(e.to_string()))?;

    let detected_source_lang = detect_source_language(&analyzed.combined_text);

    let (mut prefs, first_visit) = prefs_for_open(
        saved,
        &analyzed.files,
        detected_source_lang.as_deref(),
        &app_cfg.default_source,
        &app_cfg.default_target,
    );

    // Capabilities + world detection follow the *resolved* source language
    // (saved → detected → default), not the pre-detection pair used for discovery.
    let effective_src = get_language(&prefs.source_lang);
    let supports_world = effective_src
        .as_ref()
        .is_some_and(|l| l.supports_world_detection);
    let supports_glossary = effective_src
        .as_ref()
        .is_some_and(|l| l.supports_glossary);
    let detected_world = detect(&analyzed.combined_text, supports_world);
    // Fresh folders inherit the tone implied by the detected world (xianxia/
    // wuxia → matching tone); saved prefs are never touched. Non-Chinese
    // sources chain to Standard automatically (detect() returns Modern).
    if first_visit {
        prefs.tone = projects::tone_for_world(detected_world);
    }

    let mut projects_cfg = projects_cfg;
    projects::record_recent(&mut projects_cfg, &path, analyzed.files.len() as u32, now);
    projects::save(&app, &projects_cfg)?;

    let glossary_terms =
        crate::glossary::io::load_folder_glossary(&dir).map(|g| g.count() as u32);

    Ok(ProjectView {
        folder: path,
        total_dialogue_lines: analyzed.files.iter().map(|f| f.dialogue_count).sum(),
        files: analyzed.files,
        detected_source_lang,
        detected_world,
        prefs,
        supports_glossary,
        glossary_terms,
        first_visit,
    })
}

/// The prefs an open starts from, and whether this is the folder's first
/// visit (nothing saved yet). Saved languages, world and tone win; the
/// selection never comes back from an earlier session: every open ticks the
/// episodes that have no translation yet, so the new episode is the one
/// that's ticked and nothing gets overwritten by surprise.
fn prefs_for_open(
    saved: Option<FolderPrefs>,
    files: &[SourceFile],
    detected_source: Option<&str>,
    default_source: &str,
    default_target: &str,
) -> (FolderPrefs, bool) {
    let first_visit = saved.is_none();
    let mut prefs =
        projects::resolve_prefs(saved, detected_source, default_source, default_target);
    prefs.selected_files = untranslated_names(files);
    (prefs, first_visit)
}

/// Names of the files with no translation on disk, in list order.
fn untranslated_names(files: &[SourceFile]) -> Vec<String> {
    files
        .iter()
        .filter(|f| f.translation == TranslationState::None)
        .map(|f| f.name.clone())
        .collect()
}

struct Analyzed {
    files: Vec<SourceFile>,
    combined_text: String,
}

fn analyze_folder(dir: &Path, pair: &LanguagePair) -> Analyzed {
    let mut files = Vec::new();
    let mut combined = String::new();
    for src in discover_source_files(dir, pair) {
        let text = decode_file(&src).unwrap_or_default();
        let dialogues = parse_dialogues(&text);
        for d in &dialogues {
            combined.push_str(&strip_for_text(&d.text));
            combined.push('\n');
        }
        let output = output_on_disk(&src, pair);
        files.push(SourceFile {
            has_translation: output.state != TranslationState::None,
            translation: output.state,
            both_variants: output.both_variants,
            output_path: output.path.map(|p| p.to_string_lossy().into_owned()),
            name: src
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string(),
            path: src.to_string_lossy().into_owned(),
            dialogue_count: dialogues.len() as u32,
        });
    }
    Analyzed {
        files,
        combined_text: combined,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::projects::Tone;
    use crate::glossary::world_detector::WorldType;
    use std::fs;

    const ASS: &str = "[Script Info]\nTitle: t\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,你好\nDialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,再见\n";

    /// ep01 done, ep02 needs a look, ep03 new, ep04 done with an old warning
    /// file left over, ep05 new.
    fn season() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        for name in ["ep01.ass", "ep02.ass", "ep03.ass", "ep04.ass", "ep05.ass"] {
            fs::write(p.join(name), ASS).unwrap();
        }
        for name in [
            "ep01.eng.ass",
            "ep02.warning.eng.ass",
            "ep04.warning.eng.ass",
            "ep04.eng.ass",
        ] {
            fs::write(p.join(name), "x").unwrap();
        }
        // The leftover warning file is the older one.
        let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        fs::File::options()
            .write(true)
            .open(p.join("ep04.warning.eng.ass"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        dir
    }

    fn zh_en() -> LanguagePair {
        LanguagePair::from_codes("zh", "en").unwrap()
    }

    #[test]
    fn analyze_reads_each_episodes_output_from_disk() {
        let dir = season();
        let a = analyze_folder(dir.path(), &zh_en());
        let by_name = |n: &str| a.files.iter().find(|f| f.name == n).unwrap().clone();

        assert_eq!(a.files.len(), 5);
        assert_eq!(by_name("ep01.ass").translation, TranslationState::Done);
        assert_eq!(by_name("ep02.ass").translation, TranslationState::NeedsLook);
        assert_eq!(by_name("ep03.ass").translation, TranslationState::None);
        assert!(!by_name("ep03.ass").has_translation);
        assert_eq!(by_name("ep03.ass").output_path, None);
        assert_eq!(by_name("ep03.ass").dialogue_count, 2);

        let ep02 = by_name("ep02.ass");
        assert!(ep02.has_translation);
        assert!(ep02.output_path.unwrap().ends_with("ep02.warning.eng.ass"));

        let ep04 = by_name("ep04.ass");
        assert_eq!(ep04.translation, TranslationState::Done);
        assert!(ep04.both_variants);
        assert!(ep04.output_path.unwrap().ends_with("ep04.eng.ass"));
    }

    #[test]
    fn first_open_ticks_only_untranslated_episodes() {
        let dir = season();
        let a = analyze_folder(dir.path(), &zh_en());
        let (prefs, first_visit) = prefs_for_open(None, &a.files, Some("zh"), "zh", "en");
        assert!(first_visit);
        assert_eq!(prefs.selected_files, vec!["ep03.ass", "ep05.ass"]);
    }

    #[test]
    fn reopening_never_restores_an_old_selection() {
        // Last session ticked 01–03 and saved it; a new episode (05) has
        // turned up since. The open ticks what's untranslated instead.
        let dir = season();
        let a = analyze_folder(dir.path(), &zh_en());
        let saved = FolderPrefs {
            source_lang: "zh".into(),
            target_lang: "en".into(),
            world_override: Some(WorldType::Wuxia),
            tone: Tone::Comedic,
            selected_files: vec!["ep01.ass".into(), "ep02.ass".into(), "ep03.ass".into()],
        };
        let (prefs, first_visit) =
            prefs_for_open(Some(saved.clone()), &a.files, Some("ja"), "zh", "en");
        assert!(!first_visit);
        assert_eq!(prefs.selected_files, vec!["ep03.ass", "ep05.ass"]);
        // Everything else saved still wins.
        assert_eq!(prefs.source_lang, "zh");
        assert_eq!(prefs.world_override, Some(WorldType::Wuxia));
        assert_eq!(prefs.tone, Tone::Comedic);
    }

    #[test]
    fn a_fully_translated_folder_opens_with_nothing_ticked() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        fs::write(p.join("ep01.ass"), ASS).unwrap();
        fs::write(p.join("ep01.eng.ass"), "x").unwrap();
        let a = analyze_folder(p, &zh_en());
        let (prefs, _) = prefs_for_open(None, &a.files, None, "zh", "en");
        assert!(prefs.selected_files.is_empty());
    }
}
