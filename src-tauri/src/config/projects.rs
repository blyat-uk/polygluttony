//! Per-folder project state: recent folders + saved preferences. Persisted in its
//! own Tauri store file (`projects.json`), separate from `config.json`. Pure
//! helpers are unit-tested; the Tauri glue is thin.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;
use ts_rs::TS;

use crate::error::{AppError, AppResult};
use crate::glossary::world_detector::WorldType;

const STORE_FILE: &str = "projects.json";
const STORE_KEY: &str = "projects";
const RECENTS_CAP: usize = 10;

/// Translation tone (register). Persisted per folder; used by Translate (Step 3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum Tone {
    #[default]
    Standard,
    Xianxia,
    Wuxia,
    Comedic,
    Funny,
}

/// A folder's saved preferences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct FolderPrefs {
    pub source_lang: String,
    pub target_lang: String,
    pub world_override: Option<WorldType>,
    pub tone: Tone,
    /// Explicit list of ticked file names (empty = none ticked). Kept here
    /// during a session, but never restored: every open ticks the episodes
    /// that have no translation yet; see `open_folder`.
    #[serde(default)]
    pub selected_files: Vec<String>,
}

/// One recent-folder entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct RecentFolder {
    pub path: String,
    pub file_count: u32,
    pub last_opened: i64,
}

/// The whole persisted projects document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectsConfig {
    #[serde(default)]
    pub recents: Vec<RecentFolder>,
    #[serde(default)]
    pub folders: BTreeMap<String, FolderPrefs>,
}

// ---- pure helpers ----------------------------------------------------------

/// Insert/refresh a recent entry at the front (MRU), capped at RECENTS_CAP.
pub fn record_recent(cfg: &mut ProjectsConfig, path: &str, file_count: u32, now: i64) {
    cfg.recents.retain(|r| r.path != path);
    cfg.recents.insert(
        0,
        RecentFolder {
            path: path.to_string(),
            file_count,
            last_opened: now,
        },
    );
    cfg.recents.truncate(RECENTS_CAP);
}

pub fn remove_recent(cfg: &mut ProjectsConfig, path: &str) {
    cfg.recents.retain(|r| r.path != path);
}

pub fn clear_recents(cfg: &mut ProjectsConfig) {
    cfg.recents.clear();
}

/// Drop recents whose path no longer satisfies `exists` (injected for testing).
pub fn prune_recents(cfg: &mut ProjectsConfig, exists: impl Fn(&str) -> bool) {
    cfg.recents.retain(|r| exists(&r.path));
}

pub fn get_prefs(cfg: &ProjectsConfig, path: &str) -> Option<FolderPrefs> {
    cfg.folders.get(path).cloned()
}

pub fn set_prefs(cfg: &mut ProjectsConfig, path: &str, prefs: FolderPrefs) {
    cfg.folders.insert(path.to_string(), prefs);
}

/// Tone implied by a detected/overridden world type. Keep in sync with
/// `toneForWorld` in `src/features/episodes/folder-labels.ts`.
pub fn tone_for_world(world: crate::glossary::world_detector::WorldType) -> Tone {
    use crate::glossary::world_detector::WorldType;
    match world {
        WorldType::Xianxia => Tone::Xianxia,
        WorldType::Wuxia => Tone::Wuxia,
        WorldType::Historical | WorldType::Modern => Tone::Standard,
    }
}

/// Resolve effective prefs for a freshly opened folder: saved prefs win; otherwise
/// seed source from detection → global default, target from global default.
pub fn resolve_prefs(
    saved: Option<FolderPrefs>,
    detected_source: Option<&str>,
    default_source: &str,
    default_target: &str,
) -> FolderPrefs {
    if let Some(p) = saved {
        return p;
    }
    FolderPrefs {
        source_lang: detected_source.unwrap_or(default_source).to_string(),
        target_lang: default_target.to_string(),
        world_override: None,
        tone: Tone::Standard,
        selected_files: Vec::new(),
    }
}

// ---- Tauri glue (thin; not unit-tested) ------------------------------------

pub fn load<R: Runtime>(app: &AppHandle<R>) -> AppResult<ProjectsConfig> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| AppError::Other(e.to_string()))?;
    match store.get(STORE_KEY) {
        Some(value) => serde_json::from_value(value).map_err(AppError::from),
        None => Ok(ProjectsConfig::default()),
    }
}

pub fn save<R: Runtime>(app: &AppHandle<R>, cfg: &ProjectsConfig) -> AppResult<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| AppError::Other(e.to_string()))?;
    store.set(STORE_KEY, serde_json::to_value(cfg)?);
    store.save().map_err(|e| AppError::Other(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glossary::world_detector::WorldType;

    #[test]
    fn tone_for_world_maps_martial_worlds() {
        assert_eq!(tone_for_world(WorldType::Xianxia), Tone::Xianxia);
        assert_eq!(tone_for_world(WorldType::Wuxia), Tone::Wuxia);
        assert_eq!(tone_for_world(WorldType::Historical), Tone::Standard);
        assert_eq!(tone_for_world(WorldType::Modern), Tone::Standard);
    }

    #[test]
    fn record_recent_is_mru_and_capped() {
        let mut c = ProjectsConfig::default();
        for i in 0..12 {
            record_recent(&mut c, &format!("/f{i}"), i as u32, i as i64);
        }
        assert_eq!(c.recents.len(), 10);
        assert_eq!(c.recents[0].path, "/f11"); // newest first
        record_recent(&mut c, "/f5", 5, 99);
        assert_eq!(c.recents[0].path, "/f5");
        assert_eq!(c.recents.iter().filter(|r| r.path == "/f5").count(), 1);
    }

    #[test]
    fn prune_drops_missing() {
        let mut c = ProjectsConfig::default();
        record_recent(&mut c, "/keep", 1, 1);
        record_recent(&mut c, "/gone", 1, 2);
        prune_recents(&mut c, |p| p == "/keep");
        assert_eq!(c.recents.len(), 1);
        assert_eq!(c.recents[0].path, "/keep");
    }

    #[test]
    fn prefs_round_trip_and_resolution() {
        let mut c = ProjectsConfig::default();
        assert!(get_prefs(&c, "/x").is_none());
        let p = FolderPrefs {
            source_lang: "zh".into(),
            target_lang: "en".into(),
            world_override: Some(WorldType::Wuxia),
            tone: Tone::Comedic,
            selected_files: vec!["a.ass".into()],
        };
        set_prefs(&mut c, "/x", p.clone());
        assert_eq!(get_prefs(&c, "/x"), Some(p.clone()));

        // saved prefs win outright
        assert_eq!(resolve_prefs(Some(p.clone()), Some("ja"), "zh", "en"), p);
        // unsaved: detection seeds source, global seeds target
        let r = resolve_prefs(None, Some("ja"), "zh", "en");
        assert_eq!(r.source_lang, "ja");
        assert_eq!(r.target_lang, "en");
        assert_eq!(r.world_override, None);
        assert_eq!(r.tone, Tone::Standard);
        // unsaved + no detection: global default source
        assert_eq!(resolve_prefs(None, None, "zh", "en").source_lang, "zh");
    }
}
