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
use crate::models::language_pair::{output_filename, warning_filename};
use crate::models::{ProjectView, SourceFile, TranslationState};
use crate::utils::discover::{discover_source_files, has_existing_translation};

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

    let had_saved = saved.is_some();
    let mut prefs = projects::resolve_prefs(
        saved,
        detected_source_lang.as_deref(),
        &app_cfg.default_source,
        &app_cfg.default_target,
    );
    // A freshly opened folder (no saved prefs) starts with every file selected.
    // `selected_files` is an explicit list — an empty list means "none selected".
    if !had_saved {
        prefs.selected_files = analyzed.files.iter().map(|f| f.name.clone()).collect();
    }

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
    if !had_saved {
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
    })
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
        let dir_of = src.parent().unwrap_or_else(|| Path::new("."));
        let clean = dir_of.join(output_filename(&src, pair)).exists();
        let warning = dir_of.join(warning_filename(&src, pair)).exists();
        let translation = match (clean, warning) {
            (true, _) => TranslationState::Done,
            (false, true) => TranslationState::NeedsLook,
            (false, false) => TranslationState::None,
        };
        files.push(SourceFile {
            translation,
            both_variants: clean && warning,
            name: src
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string(),
            path: src.to_string_lossy().into_owned(),
            dialogue_count: dialogues.len() as u32,
            has_translation: has_existing_translation(&src, pair),
        });
    }
    Analyzed {
        files,
        combined_text: combined,
    }
}
