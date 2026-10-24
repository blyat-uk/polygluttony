//! Setup export / import commands (Connections page). Thin wrappers over
//! `config::transfer`, which reads, checks and writes the export file.

use std::path::Path;

use tauri::{AppHandle, Manager};

use crate::config::store;
use crate::config::transfer::{self, SetupSummary};
use crate::error::{AppError, AppResult};
use crate::prompts;

/// Write this install's setup (connections, keys, edited prompts) to `path`.
#[tauri::command]
pub fn export_setup(app: AppHandle, path: String) -> AppResult<()> {
    transfer::export_to(Path::new(&path), &store::load(&app)?, &prompts::overrides_dir(&app)?)
}

/// Read and check the export at `path`, for the confirm step. Writes nothing.
#[tauri::command]
pub fn inspect_setup(app: AppHandle, path: String) -> AppResult<SetupSummary> {
    let imported = transfer::read_file(Path::new(&path))?;
    Ok(transfer::summary(&imported, &store::load(&app)?, &prompts::overrides_dir(&app)?))
}

/// Replace this install's setup with the export at `path`. The setup it
/// replaces is saved to `<app-data>/setup-backup.zip` first (`summary.backup`;
/// importing that file is the undo). The config goes through the store plugin,
/// so its cache stays in step with the file.
#[tauri::command]
pub fn import_setup(app: AppHandle, path: String) -> AppResult<SetupSummary> {
    let imported = transfer::read_file(Path::new(&path))?;
    let current = store::load(&app)?;
    let dir = prompts::overrides_dir(&app)?;
    let backup = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Other(format!("no app-data dir: {e}")))?
        .join(transfer::BACKUP_FILE);
    let summary = transfer::summary(&imported, &current, &dir);
    let made = transfer::replace_with_backup(&imported, &current, &dir, &backup, |cfg| {
        store::save(&app, cfg)
    })?;
    Ok(SetupSummary { backup: made.map(|p| p.to_string_lossy().into_owned()), ..summary })
}
