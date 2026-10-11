//! Shared data types surfaced to the webview. TypeScript bindings are generated
//! by `ts-rs` into `src/types/generated/` (run `cargo test` to regenerate).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::config::projects::FolderPrefs;
use crate::config::{Driver, Model, Steps};
use crate::glossary::world_detector::WorldType;

pub mod language_pair;

/// Basic application/core metadata. Doubles as an IPC health check.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
}

/// The Connections page's whole state.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct ConnectionsConfig {
    pub models: Vec<Model>,
    pub steps: Option<Steps>,
    pub per_step: bool,
    pub provider_keys: BTreeMap<String, String>,
}

/// Which field a failed connection test points at, so the UI can show the
/// error under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum TestProblem {
    /// The provider rejected the API key.
    Key,
    /// The provider doesn't know the model.
    Model,
    /// The server couldn't be reached (or didn't answer in time).
    Address,
    Other,
}

/// Result of a connection test.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct TestResult {
    pub ok: bool,
    pub model: String,
    pub detected_driver: Option<Driver>,
    pub message: String,
    /// `None` when ok.
    pub problem: Option<TestProblem>,
    /// The failure may be temporary (unreachable, rate-limited, server error),
    /// so saving the connection untested is reasonable. Never for a bad key or model.
    pub can_save_anyway: bool,
}

/// O21 — first-run check.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct FirstRunStatus {
    pub has_usable_connection: bool,
}

/// One discovered source file.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct SourceFile {
    pub path: String,
    pub name: String,
    pub dialogue_count: u32,
    pub has_translation: bool,
}

/// Result of opening a folder (O6/O7/O8 bundled).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct ProjectView {
    pub folder: String,
    pub files: Vec<SourceFile>,
    pub total_dialogue_lines: u32,
    pub detected_source_lang: Option<String>,
    pub detected_world: WorldType,
    pub prefs: FolderPrefs,
    pub supports_glossary: bool,
    /// Number of glossary terms in `glossary.json`, or `None` if no glossary exists.
    pub glossary_terms: Option<u32>,
}
