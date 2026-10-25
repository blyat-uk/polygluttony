//! Setup export and import: the model library, steps, keys, default languages
//! and every edited prompt, carried between installs in one zip laid out like
//! the app-data dir (`config.json` + `prompts/…`). Pure (no `AppHandle`) and
//! unit-tested; the commands in `commands/setup.rs` load and save around it.
//!
//! Import reads only the names it knows — `config.json` and `prompts/<file>`
//! for each catalog prompt — and never builds a path from an archive name.
//! Everything is read and checked before anything is written.

use std::fs;
use std::io::{self, BufReader, BufWriter, ErrorKind, Read, Seek, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;
use zip::result::ZipError;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::config::store;
use crate::config::{AppConfig, Model, CONFIG_VERSION};
use crate::error::{AppError, AppResult};
use crate::prompts::{self, Entry, PromptId};

/// The config's place in the archive, as in the app-data dir.
const CONFIG_ENTRY: &str = "config.json";
/// The key the config sits under, as the store plugin writes it.
const CONFIG_KEY: &str = "app";
/// Edited prompts sit under this, at their catalog file.
const PROMPTS_PREFIX: &str = "prompts/";
/// A picked file bigger than this isn't an export (a real one is a few KB).
const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
/// Every entry read stops here, so a zip bomb can't run away.
const MAX_ENTRY_BYTES: u64 = 1024 * 1024;
/// A zip with more entries than this isn't an export (one holds at most the
/// config and each catalog prompt), and its names would flood `skipped`.
const MAX_ENTRIES: usize = 256;
/// Siblings of the prompts dir: where an import stages the new prompts, and
/// where the old ones wait while the new ones move in.
const STAGING_SUFFIX: &str = ".import";
const OLD_SUFFIX: &str = ".old";

/// An import saves the setup it replaces in the app-data dir as
/// `setup-backup-<epoch ms>.zip`, laid out like an export, so importing it
/// is the undo. The newest [`BACKUPS_KEPT`] are kept.
const BACKUP_PREFIX: &str = "setup-backup-";
const BACKUP_SUFFIX: &str = ".zip";
pub const BACKUPS_KEPT: usize = 3;

const NOT_AN_EXPORT: &str = "This isn't a Polygluttony export.";
const NEWER_EXPORT: &str =
    "This export was made by a newer Polygluttony. Update Polygluttony, then import it again.";
const TOO_BIG: &str = "it's larger than 1 MB";

/// What an export file brings in: shown before the import and after it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct SetupSummary {
    /// The file's library models by name ("Claude Sonnet 5.5"), in library order.
    pub models: Vec<String>,
    /// How many edited prompts the file carries.
    pub edited_prompts: u32,
    /// Entries this version doesn't know, ignored (e.g. from a newer Polygluttony).
    pub skipped: Vec<String>,
    /// This install already has models or edited prompts, which the import replaces.
    pub replaces_existing: bool,
    /// After an import that replaced a setup: the export of the replaced setup,
    /// saved first. Importing it undoes the import. `None` before an import.
    pub backup: Option<String>,
}

/// An export file, read and checked: ready to apply.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    /// Migrated and normalized, like a stored config. No work folder.
    pub config: AppConfig,
    /// The edited prompts, in catalog order; each passed `prompts::validate`.
    pub prompts: Vec<(PromptId, String)>,
    /// Names this version doesn't read, in archive order.
    pub skipped: Vec<String>,
}

// ---- errors -------------------------------------------------------------------

fn not_an_export() -> AppError {
    AppError::Other(NOT_AN_EXPORT.into())
}

/// An edited prompt in the file that can't be used, named as the Prompts page
/// names it.
fn unusable_prompt(e: &Entry, why: &str) -> AppError {
    AppError::Other(format!("The edited \"{}\" prompt in this export can't be used: {why}.", e.name))
}

fn write_failed(e: ZipError) -> AppError {
    io::Error::from(e).into()
}

// ---- shared -------------------------------------------------------------------

/// `path` with `suffix` added to its file name (`prompts` → `prompts.old`).
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

fn prompt_entry_name(e: &Entry) -> String {
    format!("{PROMPTS_PREFIX}{}", e.file)
}

/// Why an edited prompt can't travel, if it can't: over the size cap, or not a
/// prompt `validate` accepts. Export checks as import does, so every export
/// imports.
fn prompt_problem(e: &Entry, text: &str) -> Option<String> {
    if text.len() as u64 > MAX_ENTRY_BYTES {
        return Some(TOO_BIG.into());
    }
    prompts::validate(e.id, text).err().map(|err| err.to_string())
}

// ---- export -------------------------------------------------------------------

/// `config.json`'s contents: the store's document shape, without the work
/// folder (a path on this computer; an import keeps the one it has).
fn config_document(cfg: &AppConfig) -> AppResult<Vec<u8>> {
    let cfg = AppConfig { default_workdir: None, ..cfg.clone() };
    let mut doc = Map::new();
    doc.insert(CONFIG_KEY.to_string(), serde_json::to_value(cfg)?);
    Ok(serde_json::to_vec_pretty(&doc)?)
}

/// This install's edited prompts (those with an override file), in catalog
/// order, read the way a run reads them.
fn edited_prompts(overrides_dir: &Path) -> AppResult<Vec<(&'static Entry, String)>> {
    let mut out = Vec::new();
    for e in prompts::entries().iter().filter(|e| overrides_dir.join(e.file).is_file()) {
        let text = prompts::resolve(e.id, overrides_dir)?;
        if let Some(why) = prompt_problem(e, &text) {
            return Err(AppError::Other(format!(
                "Your edited \"{}\" prompt can't be exported: {why}. Fix it on the Prompts page, \
                 then export again.",
                e.name
            )));
        }
        out.push((e, text));
    }
    Ok(out)
}

/// Write an export of `cfg` and the edited prompts in `overrides_dir` to `w`.
/// The prompts are read and checked before the first byte is written.
pub fn write_archive<W: Write + Seek>(w: W, cfg: &AppConfig, overrides_dir: &Path) -> AppResult<W> {
    let edited = edited_prompts(overrides_dir)?;
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut zip = ZipWriter::new(w);
    zip.start_file(CONFIG_ENTRY, options).map_err(write_failed)?;
    zip.write_all(&config_document(cfg)?)?;
    for (e, text) in edited {
        zip.start_file(prompt_entry_name(e), options).map_err(write_failed)?;
        zip.write_all(text.as_bytes())?;
    }
    zip.finish().map_err(write_failed)
}

/// Export to `path` through a temp sibling renamed over it, so a failed export
/// never leaves a half-written file where the user asked for one.
pub fn export_to(path: &Path, cfg: &AppConfig, overrides_dir: &Path) -> AppResult<()> {
    let tmp = with_suffix(path, ".tmp");
    let written = fs::File::create(&tmp)
        .map_err(AppError::from)
        .and_then(|file| Ok(write_archive(BufWriter::new(file), cfg, overrides_dir)?.flush()?))
        .and_then(|()| Ok(fs::rename(&tmp, path)?));
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

// ---- import: reading and checking -----------------------------------------------

/// Read and check the export at `path`. Writes nothing.
pub fn read_file(path: &Path) -> AppResult<Imported> {
    // Size from the open handle, before the zip reader sees the file.
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > MAX_FILE_BYTES {
        return Err(not_an_export());
    }
    read_archive(BufReader::new(file))
}

/// Read and check an export. Only `config.json` and the catalog's prompt files
/// are read; every other name is skipped (directories silently).
pub fn read_archive<R: Read + Seek>(reader: R) -> AppResult<Imported> {
    let mut zip = ZipArchive::new(reader).map_err(|_| not_an_export())?;
    if zip.len() > MAX_ENTRIES {
        return Err(not_an_export());
    }
    let names: Vec<String> = zip
        .file_names()
        .map(|name| name.map(|n| n.into_owned()))
        .collect::<Result<_, _>>()
        .map_err(|_| not_an_export())?;

    let config_at = zip.index_for_name(CONFIG_ENTRY).ok_or_else(not_an_export)?;
    let config = config_from_export(&read_entry(&mut zip, config_at).map_err(|_| not_an_export())?)?;

    let mut edited = Vec::new();
    for e in prompts::entries() {
        if let Some(at) = zip.index_for_name(&prompt_entry_name(e)) {
            edited.push((e.id, read_prompt(&mut zip, at, e)?));
        }
    }

    let skipped = names.into_iter().filter(|n| !n.ends_with('/') && !is_known(n)).collect();
    Ok(Imported { config, prompts: edited, skipped })
}

/// A name the import reads: `config.json` or a catalog prompt's file.
fn is_known(name: &str) -> bool {
    name == CONFIG_ENTRY
        || name
            .strip_prefix(PROMPTS_PREFIX)
            .is_some_and(|file| prompts::entries().iter().any(|e| e.file == file))
}

/// One entry's bytes, capped at `MAX_ENTRY_BYTES`. `Err` says why, for the
/// message; the zip error itself goes to the log. Bytes first, size check
/// second: the cap can split a multibyte character, so UTF-8 comes after.
fn read_entry<R: Read + Seek>(zip: &mut ZipArchive<R>, index: usize) -> Result<Vec<u8>, &'static str> {
    let mut buf = Vec::new();
    let read = zip
        .by_index(index)
        .map_err(io::Error::from)
        .and_then(|file| file.take(MAX_ENTRY_BYTES + 1).read_to_end(&mut buf));
    if let Err(e) = read {
        tracing::warn!("couldn't read entry #{index} of the export ({e})");
        return Err("it can't be read");
    }
    if buf.len() as u64 > MAX_ENTRY_BYTES {
        return Err(TOO_BIG);
    }
    Ok(buf)
}

/// The config an export carries. It must be `{"app": {...}}` with a numeric
/// `version` no newer than this build's; then it loads as a stored config does
/// (migrated, normalized), minus the work folder.
fn config_from_export(bytes: &[u8]) -> AppResult<AppConfig> {
    let doc: Value = serde_json::from_slice(bytes).map_err(|_| not_an_export())?;
    let app = doc.get(CONFIG_KEY).filter(|v| v.is_object()).ok_or_else(not_an_export)?;
    let version = app.get("version").and_then(Value::as_f64).ok_or_else(not_an_export)?;
    if version > f64::from(CONFIG_VERSION) {
        return Err(AppError::Other(NEWER_EXPORT.into()));
    }
    Ok(AppConfig { default_workdir: None, ..store::config_from_stored(app.clone()) })
}

/// An edited prompt from the file: within the cap, UTF-8, and valid.
fn read_prompt<R: Read + Seek>(zip: &mut ZipArchive<R>, index: usize, e: &Entry) -> AppResult<String> {
    let bytes = read_entry(zip, index).map_err(|why| unusable_prompt(e, why))?;
    let text = String::from_utf8(bytes).map_err(|_| unusable_prompt(e, "it isn't UTF-8 text"))?;
    match prompt_problem(e, &text) {
        Some(why) => Err(unusable_prompt(e, &why)),
        None => Ok(text),
    }
}

/// What importing `imported` brings in, and whether it replaces a setup this
/// install already has (`current` and the edited prompts in `overrides_dir`).
pub fn summary(imported: &Imported, current: &AppConfig, overrides_dir: &Path) -> SetupSummary {
    SetupSummary {
        models: imported.config.models.iter().map(Model::label).collect(),
        edited_prompts: imported.prompts.len() as u32,
        skipped: imported.skipped.clone(),
        replaces_existing: has_setup(current, overrides_dir),
        backup: None,
    }
}

/// This install has something an import would replace: models or edited prompts.
fn has_setup(current: &AppConfig, overrides_dir: &Path) -> bool {
    !current.models.is_empty() || prompts::list_meta(overrides_dir).iter().any(|m| m.modified)
}

// ---- import: applying -----------------------------------------------------------

fn remove_dir_if_present(dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

/// Each prompt at its catalog path under `dir` — never at a name from the archive.
fn write_prompts(dir: &Path, edited: &[(PromptId, String)]) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    for (id, text) in edited {
        let path = dir.join(prompts::entry(*id).file);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, text)?;
    }
    Ok(())
}

/// Write `edited` into a fresh staging dir beside `overrides_dir` (a leftover
/// one from an interrupted import goes first) and return it. On failure the
/// staging dir is gone again.
pub fn stage_prompts(overrides_dir: &Path, edited: &[(PromptId, String)]) -> AppResult<PathBuf> {
    let staging = with_suffix(overrides_dir, STAGING_SUFFIX);
    remove_dir_if_present(&staging)?;
    if let Err(e) = write_prompts(&staging, edited) {
        let _ = fs::remove_dir_all(&staging);
        return Err(e.into());
    }
    Ok(staging)
}

/// Put `staged` in place of `overrides_dir`: the current dir moves aside, the
/// staged one moves in, the old one goes. If the staged dir can't move in, the
/// old one goes back.
pub fn swap_prompts(overrides_dir: &Path, staged: &Path) -> AppResult<()> {
    let old = with_suffix(overrides_dir, OLD_SUFFIX);
    remove_dir_if_present(&old)?;
    let had_dir = overrides_dir.exists();
    if had_dir {
        fs::rename(overrides_dir, &old)?;
    }
    if let Err(e) = fs::rename(staged, overrides_dir) {
        if had_dir {
            if let Err(back) = fs::rename(&old, overrides_dir) {
                tracing::warn!("couldn't put the prompts back from {} ({back})", old.display());
            }
        }
        return Err(e.into());
    }
    if had_dir {
        // The import is done; a leftover is cleared by the next one.
        if let Err(e) = fs::remove_dir_all(&old) {
            tracing::warn!("couldn't remove the replaced prompts at {} ({e})", old.display());
        }
    }
    Ok(())
}

/// Replace this install's setup with `imported`: stage its prompts, save its
/// config through `save` (keeping `current`'s work folder), then swap the
/// prompts in. A failure before `save` succeeds leaves the disk as it was.
pub fn apply(
    imported: &Imported,
    current: &AppConfig,
    overrides_dir: &Path,
    save: impl FnOnce(&AppConfig) -> AppResult<()>,
) -> AppResult<()> {
    let staged = stage_prompts(overrides_dir, &imported.prompts)?;
    let cfg = AppConfig { default_workdir: current.default_workdir.clone(), ..imported.config.clone() };
    if let Err(e) = save(&cfg) {
        let _ = fs::remove_dir_all(&staged);
        return Err(e);
    }
    swap_prompts(overrides_dir, &staged).map_err(|e| {
        let _ = fs::remove_dir_all(&staged);
        AppError::Other(format!(
            "Your connections were imported, but the prompts couldn't be ({e}). Import the file \
             again to finish."
        ))
    })
}

// ---- import: the backup of the setup it replaces --------------------------------

/// The epoch ms of a backup's file name, when `name` is one.
fn backup_stamp(name: &str) -> Option<i64> {
    name.strip_prefix(BACKUP_PREFIX)?.strip_suffix(BACKUP_SUFFIX)?.parse().ok()
}

/// The backups in `dir`, newest first.
fn backups_in(dir: &Path) -> Vec<(i64, PathBuf)> {
    let mut found: Vec<(i64, PathBuf)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| Some((backup_stamp(e.file_name().to_str()?)?, e.path())))
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found
}

/// Save this install's setup as it is on disk into a new
/// `setup-backup-<at_ms>.zip` in `dir`: `config_file` byte for byte (the
/// stored config, or `current` written out when there's no file yet) and
/// every edited prompt's file, unchecked, so a prompt that wouldn't pass
/// an import can't block the import that replaces it. Then only the newest
/// [`BACKUPS_KEPT`] backups stay. Returns the new backup's path.
pub fn backup_setup(
    dir: &Path,
    config_file: &Path,
    current: &AppConfig,
    overrides_dir: &Path,
    at_ms: i64,
) -> AppResult<PathBuf> {
    let config = match fs::read(config_file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == ErrorKind::NotFound => config_document(current)?,
        Err(e) => return Err(e.into()),
    };
    let mut prompt_files = Vec::new();
    for e in prompts::entries() {
        match fs::read(overrides_dir.join(e.file)) {
            Ok(bytes) => prompt_files.push((prompt_entry_name(e), bytes)),
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }

    fs::create_dir_all(dir)?;
    let mut stamp = at_ms.max(0);
    let mut path = dir.join(format!("{BACKUP_PREFIX}{stamp}{BACKUP_SUFFIX}"));
    while path.exists() {
        stamp += 1;
        path = dir.join(format!("{BACKUP_PREFIX}{stamp}{BACKUP_SUFFIX}"));
    }
    let tmp = with_suffix(&path, ".tmp");
    let written = (|| -> AppResult<()> {
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let mut zip = ZipWriter::new(BufWriter::new(fs::File::create(&tmp)?));
        zip.start_file(CONFIG_ENTRY, options).map_err(write_failed)?;
        zip.write_all(&config)?;
        for (name, bytes) in &prompt_files {
            zip.start_file(name.as_str(), options).map_err(write_failed)?;
            zip.write_all(bytes)?;
        }
        zip.finish().map_err(write_failed)?.flush()?;
        fs::rename(&tmp, &path)?;
        Ok(())
    })();
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }

    for (_, old) in backups_in(dir).into_iter().skip(BACKUPS_KEPT) {
        if let Err(e) = fs::remove_file(&old) {
            tracing::warn!("couldn't remove the old setup backup {} ({e})", old.display());
        }
    }
    Ok(path)
}

/// `apply`, after first saving the setup it replaces (when there is one) to
/// a new backup in `backup_dir` (see [`backup_setup`]), so the import can be
/// undone by importing the backup. Returns the backup's path when one was
/// made. A backup that can't be written stops the import before anything
/// changes.
pub fn replace_with_backup(
    imported: &Imported,
    current: &AppConfig,
    overrides_dir: &Path,
    config_file: &Path,
    backup_dir: &Path,
    save: impl FnOnce(&AppConfig) -> AppResult<()>,
) -> AppResult<Option<PathBuf>> {
    let made = if has_setup(current, overrides_dir) {
        let at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let path = backup_setup(backup_dir, config_file, current, overrides_dir, at_ms).map_err(|e| {
            AppError::Other(format!(
                "Couldn't save a backup of your current setup first ({e}), so nothing was replaced."
            ))
        })?;
        Some(path)
    } else {
        None
    };
    apply(imported, current, overrides_dir, save)?;
    Ok(made)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    use serde_json::json;

    use crate::config::providers::default_config;
    use crate::config::{sample_model, Provider, Step, StepSettings};
    use PromptId::{ToneFunny, ToneXianxia, TranslateZhEn, Verify};

    // ---- fixtures ----

    /// A setup after some use: Sonnet does the text steps, Gemini searches and
    /// verifies with its own tuning (so the page shows every step), both keyed,
    /// languages changed, and a work folder.
    fn configured() -> AppConfig {
        let mut cfg = default_config();
        let sonnet = Model { api_key: "sk-ant".into(), ..sample_model(Provider::Anthropic, "claude-sonnet-5-5") };
        let gemini = Model { api_key: "AIza".into(), ..sample_model(Provider::Google, "gemini-3-flash-preview") };
        store::save_model(&mut cfg, sonnet, None).unwrap();
        store::save_model(&mut cfg, gemini, Some(Step::WebSearch)).unwrap();
        let verify = StepSettings { timeout: Some(90), max_tokens: Some(4000), ..StepSettings::new("m2") };
        store::set_step(&mut cfg, Step::Verify, Some(verify)).unwrap();
        store::set_default_languages(&mut cfg, "ko", "fr");
        cfg.default_workdir = Some("/home/me/subs".into());
        cfg
    }

    fn owned(edited: &[(PromptId, &str)]) -> Vec<(PromptId, String)> {
        edited.iter().map(|(id, text)| (*id, text.to_string())).collect()
    }

    /// An app-data dir whose `prompts/` holds `edited`.
    fn app_data(edited: &[(PromptId, &str)]) -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("prompts");
        write_prompts(&dir, &owned(edited)).unwrap();
        (root, dir)
    }

    fn export(cfg: &AppConfig, overrides_dir: &Path) -> Vec<u8> {
        write_archive(Cursor::new(Vec::new()), cfg, overrides_dir).unwrap().into_inner()
    }

    /// A zip of `entries` as any zip tool might write it (`…/` = a directory).
    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            if name.ends_with('/') {
                zip.add_directory(*name, SimpleFileOptions::default()).unwrap();
            } else {
                zip.start_file(*name, SimpleFileOptions::default()).unwrap();
                zip.write_all(bytes).unwrap();
            }
        }
        zip.finish().unwrap().into_inner()
    }

    fn read(bytes: Vec<u8>) -> AppResult<Imported> {
        read_archive(Cursor::new(bytes))
    }

    fn error(result: AppResult<Imported>) -> String {
        result.unwrap_err().to_string()
    }

    /// Every file under `root`, sorted.
    fn files_under(root: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mut dirs = vec![root.to_path_buf()];
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() { dirs.push(path) } else { files.push(path) }
            }
        }
        files.sort();
        files
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    // ---- export ----

    #[test]
    fn an_export_round_trips_the_whole_setup() {
        let cfg = configured();
        let edited = [(TranslateZhEn, "{GLOSSARY} my way {TONE}"), (ToneXianxia, "my xianxia")];
        let (_root, dir) = app_data(&edited);
        let imported = read(export(&cfg, &dir)).unwrap();
        assert_eq!(imported.config, AppConfig { default_workdir: None, ..cfg.clone() });
        // The parts people would retype by hand, spelled out.
        let c = &imported.config;
        assert_eq!(c.provider_keys.get("anthropic").map(String::as_str), Some("sk-ant"));
        assert_eq!(c.models[1].api_key, "AIza");
        let steps = c.steps.as_ref().unwrap();
        assert_eq!(steps.web_search.as_ref().map(|s| s.model.as_str()), Some("m2"));
        assert_eq!((steps.verify.timeout, steps.verify.max_tokens), (Some(90), Some(4000)));
        assert!(c.per_step);
        assert_eq!((c.default_source.as_str(), c.default_target.as_str()), ("ko", "fr"));
        assert_eq!(imported.prompts, owned(&edited));
        assert!(imported.skipped.is_empty());
    }

    #[test]
    fn an_export_holds_the_config_and_the_edited_prompts_but_not_the_work_folder() {
        let (_root, dir) = app_data(&[(ToneXianxia, "my xianxia")]);
        let mut zip = ZipArchive::new(Cursor::new(export(&configured(), &dir))).unwrap();
        let names: Vec<String> = zip.file_names().map(|n| n.unwrap().into_owned()).collect();
        assert_eq!(names, ["config.json", "prompts/tones/xianxia.txt"]);
        let config = zip.by_name("config.json").unwrap();
        assert_eq!(config.compression(), CompressionMethod::Deflated);
        let doc: Value = serde_json::from_reader(config).unwrap();
        assert_eq!(doc["app"]["version"], CONFIG_VERSION);
        assert_eq!(doc["app"]["default_workdir"], Value::Null);
    }

    #[test]
    fn export_to_replaces_the_file_and_leaves_no_temp_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("polygluttony-export-2026-10-13.zip");
        fs::write(&path, "an older export").unwrap();
        export_to(&path, &configured(), &root.path().join("prompts")).unwrap();
        assert_eq!(read_file(&path).unwrap().config.models.len(), 2);
        assert_eq!(names_in(root.path()), ["polygluttony-export-2026-10-13.zip"]);
    }

    #[test]
    fn an_edited_prompt_an_import_would_refuse_stops_the_export() {
        let (root, dir) = app_data(&[(Verify, "  \n")]);
        let path = root.path().join("export.zip");
        let err = export_to(&path, &configured(), &dir).unwrap_err().to_string();
        assert_eq!(
            err,
            "Your edited \"Verify\" prompt can't be exported: prompt is empty. Fix it on the \
             Prompts page, then export again."
        );
        assert_eq!(names_in(root.path()), ["prompts"], "no export, no temp file");
    }

    // ---- import: what counts as an export ----

    #[test]
    fn a_file_that_isnt_a_zip_isnt_an_export() {
        assert_eq!(error(read(b"just some subtitles".to_vec())), NOT_AN_EXPORT);
        assert_eq!(error(read(Vec::new())), NOT_AN_EXPORT);
    }

    #[test]
    fn a_file_over_20_mb_isnt_an_export() {
        let root = tempfile::tempdir().unwrap();
        let zip = export(&configured(), &root.path().join("prompts"));
        // A valid export behind a run of padding the zip reader skips (sparse:
        // nothing is written), so only the file's size decides.
        let padded = |total: u64| {
            let path = root.path().join(format!("{total}.zip"));
            let mut file = fs::File::create(&path).unwrap();
            file.set_len(total - zip.len() as u64).unwrap();
            file.seek(io::SeekFrom::End(0)).unwrap();
            file.write_all(&zip).unwrap();
            path
        };
        read_file(&padded(MAX_FILE_BYTES)).unwrap();
        assert_eq!(read_file(&padded(MAX_FILE_BYTES + 1)).unwrap_err().to_string(), NOT_AN_EXPORT);
    }

    #[test]
    fn a_zip_of_more_entries_than_an_export_could_hold_isnt_an_export() {
        let config = serde_json::to_vec(&json!({ "app": default_config() })).unwrap();
        let names: Vec<String> = (1..MAX_ENTRIES).map(|i| format!("junk/{i}.txt")).collect();
        let mut entries: Vec<(&str, &[u8])> = vec![(CONFIG_ENTRY, &config)];
        entries.extend(names.iter().map(|n| (n.as_str(), &b""[..])));
        assert_eq!(read(zip_of(&entries)).unwrap().skipped.len(), MAX_ENTRIES - 1);
        entries.push(("one-too-many.txt", b""));
        assert_eq!(error(read(zip_of(&entries))), NOT_AN_EXPORT);
    }

    #[test]
    fn a_zip_without_config_json_isnt_an_export() {
        assert_eq!(error(read(zip_of(&[("prompts/verify.txt", b"check the lines")]))), NOT_AN_EXPORT);
        assert_eq!(error(read(zip_of(&[("backup/config.json", b"{}")]))), NOT_AN_EXPORT);
        assert_eq!(error(read(zip_of(&[("config.json/", b"")]))), NOT_AN_EXPORT, "a directory");
    }

    #[test]
    fn config_json_needs_an_app_object_with_a_numeric_version() {
        let docs = [
            json!("not an object"),
            json!([]),
            json!({}),
            json!({ "app": 2 }),
            json!({ "config": { "version": 2 } }),
            json!({ "app": { "version": "2" } }),
            json!({ "app": { "version": null } }),
            // v1 never wrote a version; every export carries one.
            json!({ "app": { "default_source": "zh", "default_target": "en",
                             "active_connection": "anthropic", "connections": {} } }),
        ];
        for doc in docs {
            let bytes = serde_json::to_vec(&doc).unwrap();
            assert_eq!(error(read(zip_of(&[("config.json", &bytes)]))), NOT_AN_EXPORT, "{doc}");
        }
        assert_eq!(error(read(zip_of(&[("config.json", b"{\"app\": ")]))), NOT_AN_EXPORT, "not JSON");
    }

    #[test]
    fn a_newer_export_asks_for_an_update() {
        for version in [json!(CONFIG_VERSION + 1), json!(2.5)] {
            let bytes = serde_json::to_vec(&json!({ "app": { "version": version, "models": [] } })).unwrap();
            assert_eq!(error(read(zip_of(&[("config.json", &bytes)]))), NEWER_EXPORT, "{version}");
        }
    }

    #[test]
    fn the_app_value_loads_like_a_stored_config() {
        // A sparse v2 value: missing fields default, the library gets its steps,
        // a model's key is remembered, and the work folder stays out.
        let sonnet = Model { id: "m1".into(), api_key: "sk-ant".into(), ..sample_model(Provider::Anthropic, "claude-sonnet-5-5") };
        let doc = json!({ "app": { "version": 2, "default_workdir": "/elsewhere", "models": [sonnet] } });
        let cfg = read(zip_of(&[("config.json", &serde_json::to_vec(&doc).unwrap())])).unwrap().config;
        assert_eq!((cfg.default_source.as_str(), cfg.default_target.as_str()), ("zh", "en"));
        assert_eq!(cfg.steps.as_ref().map(|s| s.translate.model.as_str()), Some("m1"));
        assert_eq!(cfg.provider_keys.get("anthropic").map(String::as_str), Some("sk-ant"));
        assert_eq!(cfg.default_workdir, None);
        // A pre-release v2 value (two roles under version 2) is migrated.
        let doc = json!({ "app": { "version": 2, "default_source": "ko", "default_target": "en",
            "translation": { "provider": "anthropic", "driver": "anthropic",
                "base_url": "https://api.anthropic.com", "api_key": "k", "model": "claude-sonnet-5-5" } } });
        let cfg = read(zip_of(&[("config.json", &serde_json::to_vec(&doc).unwrap())])).unwrap().config;
        assert_eq!(cfg.models.iter().map(Model::label).collect::<Vec<_>>(), ["Claude Sonnet 5.5"]);
        assert_eq!(cfg.version, CONFIG_VERSION);
    }

    // ---- import: entries ----

    #[test]
    fn an_entry_over_1_mb_is_refused() {
        let ok = config_document(&default_config()).unwrap();
        let max = vec![b'a'; MAX_ENTRY_BYTES as usize];
        let over = vec![b'a'; MAX_ENTRY_BYTES as usize + 1];
        assert_eq!(error(read(zip_of(&[("config.json", &over)]))), NOT_AN_EXPORT);
        assert_eq!(
            error(read(zip_of(&[("config.json", &ok), ("prompts/verify.txt", &over)]))),
            "The edited \"Verify\" prompt in this export can't be used: it's larger than 1 MB."
        );
        let imported = read(zip_of(&[("config.json", &ok), ("prompts/verify.txt", &max)])).unwrap();
        assert_eq!(imported.prompts[0].1.len(), max.len(), "exactly 1 MB is fine");
    }

    #[test]
    fn an_invalid_edited_prompt_is_named_as_the_prompts_page_names_it() {
        let ok = config_document(&default_config()).unwrap();
        assert_eq!(
            error(read(zip_of(&[("config.json", &ok), ("prompts/translate.zh-en.txt", b"use {TONE}")]))),
            "The edited \"Chinese → English\" prompt in this export can't be used: missing required \
             placeholder: {GLOSSARY}."
        );
        assert_eq!(
            error(read(zip_of(&[("config.json", &ok), ("prompts/tones/wuxia.txt", b" \n ")]))),
            "The edited \"Wuxia\" prompt in this export can't be used: prompt is empty."
        );
    }

    #[test]
    fn an_edited_prompt_that_isnt_utf8_is_refused() {
        let ok = config_document(&default_config()).unwrap();
        assert_eq!(
            error(read(zip_of(&[("config.json", &ok), ("prompts/verify.txt", &[0xff, 0xfe, 0xfd])]))),
            "The edited \"Verify\" prompt in this export can't be used: it isn't UTF-8 text."
        );
    }

    #[test]
    fn a_damaged_entry_cant_be_read() {
        // Stored, so the text sits in the file as is. Flipping the first byte of
        // `needle` leaves valid content ("bheck", "{h") that only the entry's
        // checksum catches.
        let config = config_document(&default_config()).unwrap();
        let damaged = |needle: &[u8]| {
            let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
            let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            for (name, bytes) in [(CONFIG_ENTRY, &config[..]), ("prompts/verify.txt", &b"check the lines"[..])] {
                zip.start_file(name, stored).unwrap();
                zip.write_all(bytes).unwrap();
            }
            let mut bytes = zip.finish().unwrap().into_inner();
            let at = bytes.windows(needle.len()).position(|w| w == needle).unwrap();
            bytes[at] ^= 1;
            bytes
        };
        assert_eq!(
            error(read(damaged(b"check the lines"))),
            "The edited \"Verify\" prompt in this export can't be used: it can't be read."
        );
        assert_eq!(error(read(damaged(b"zh\""))), NOT_AN_EXPORT, "the config, damaged");
    }

    #[test]
    fn unknown_and_traversal_names_are_skipped_and_never_written() {
        let unknown = [
            "../x",
            "/etc/x",
            "prompts/../../x",
            "prompts/tones/../../evil.txt",
            "prompts/unknown.txt",
            "PROMPTS/verify.txt",
            "prompts\\verify.txt",
            "projects.json",
        ];
        let config = config_document(&configured()).unwrap();
        let mut entries: Vec<(&str, &[u8])> =
            vec![("config.json", &config), ("prompts/", b""), ("prompts/tones/", b""), ("prompts/verify.txt", b"check")];
        entries.extend(unknown.iter().map(|name| (*name, &b"{GLOSSARY} {TONE} pwned"[..])));
        let imported = read(zip_of(&entries)).unwrap();
        assert_eq!(imported.skipped, unknown);
        assert_eq!(imported.prompts, owned(&[(Verify, "check")]));
        // Applied deep inside a sandbox, so a `../../` escape would still land
        // where this test looks: only the catalog's file is written.
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("a/b/c/prompts");
        apply(&imported, &default_config(), &dir, |_| Ok(())).unwrap();
        assert_eq!(files_under(root.path()), [dir.join("verify.txt")]);
    }

    // ---- import: applying ----

    #[test]
    fn an_import_replaces_the_edited_prompts_exactly() {
        let (root, dir) = app_data(&[(Verify, "my check"), (TranslateZhEn, "{GLOSSARY} old {TONE}")]);
        // Leftovers of an import that was interrupted.
        write_prompts(&root.path().join("prompts.import"), &owned(&[(ToneFunny, "leftover")])).unwrap();
        fs::create_dir_all(root.path().join("prompts.old")).unwrap();
        let imported = Imported {
            config: default_config(),
            prompts: owned(&[(TranslateZhEn, "{GLOSSARY} new {TONE}"), (ToneXianxia, "my xianxia")]),
            skipped: Vec::new(),
        };
        apply(&imported, &default_config(), &dir, |_| Ok(())).unwrap();
        assert_eq!(files_under(root.path()), [dir.join("tones/xianxia.txt"), dir.join("translate.zh-en.txt")]);
        assert_eq!(prompts::resolve(TranslateZhEn, &dir).unwrap(), "{GLOSSARY} new {TONE}");
        assert_eq!(
            prompts::resolve(Verify, &dir).unwrap(),
            prompts::default_text(Verify),
            "an edit the file lacks goes back to the default"
        );
        assert_eq!(names_in(root.path()), ["prompts"], "no staging or old dir left");
    }

    #[test]
    fn an_import_saves_the_files_config_but_keeps_this_installs_work_folder() {
        let (_root, dir) = app_data(&[]);
        let imported = read(export(&configured(), &dir)).unwrap();
        let current = AppConfig { default_workdir: Some("/here/subs".into()), ..default_config() };
        let mut saved = None;
        apply(&imported, &current, &dir, |cfg| {
            saved = Some(cfg.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(saved, Some(AppConfig { default_workdir: Some("/here/subs".into()), ..configured() }));
    }

    #[test]
    fn a_failed_save_changes_nothing_on_disk() {
        let (root, dir) = app_data(&[(Verify, "my check")]);
        let imported = Imported { config: configured(), prompts: owned(&[(ToneFunny, "funnier")]), skipped: Vec::new() };
        let err = apply(&imported, &default_config(), &dir, |_| Err(AppError::Other("disk full".into())));
        assert_eq!(err.unwrap_err().to_string(), "disk full");
        assert_eq!(files_under(root.path()), [dir.join("verify.txt")]);
        assert_eq!(fs::read_to_string(dir.join("verify.txt")).unwrap(), "my check");
    }

    #[test]
    fn an_import_works_before_any_prompt_was_edited() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("prompts");
        let imported = Imported { config: default_config(), prompts: owned(&[(Verify, "check")]), skipped: Vec::new() };
        apply(&imported, &default_config(), &dir, |_| Ok(())).unwrap();
        assert_eq!(files_under(root.path()), [dir.join("verify.txt")]);
        assert_eq!(names_in(root.path()), ["prompts"]);
    }

    #[test]
    fn a_swap_that_cant_move_the_new_prompts_in_puts_the_old_ones_back() {
        let (root, dir) = app_data(&[(Verify, "my check")]);
        assert!(swap_prompts(&dir, &root.path().join("nowhere")).is_err());
        assert_eq!(files_under(root.path()), [dir.join("verify.txt")]);
        assert_eq!(names_in(root.path()), ["prompts"]);
    }

    // ---- backup before replacing ----

    /// The stored config as the store plugin writes it, with a work folder.
    fn write_config_file(root: &Path, cfg: &AppConfig) -> PathBuf {
        let path = root.join("config.json");
        let doc = json!({ CONFIG_KEY: serde_json::to_value(cfg).unwrap() });
        fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
        path
    }

    #[test]
    fn replacing_a_setup_saves_the_current_one_first_and_the_backup_brings_it_back() {
        let (root, dir) = app_data(&[(Verify, "my check")]);
        let current = AppConfig { default_workdir: Some("/here/subs".into()), ..configured() };
        let config_file = write_config_file(root.path(), &current);
        let incoming = Imported { config: default_config(), prompts: owned(&[(ToneFunny, "funnier")]), skipped: Vec::new() };
        let mut saved = Vec::new();

        let made = replace_with_backup(&incoming, &current, &dir, &config_file, root.path(), |cfg| {
            saved.push(cfg.clone());
            Ok(())
        })
        .unwrap()
        .unwrap();
        let name = made.file_name().unwrap().to_str().unwrap();
        assert!(backup_stamp(name).is_some(), "{name}");
        assert_eq!(made.parent(), Some(root.path()));
        assert_eq!(prompts::resolve(ToneFunny, &dir).unwrap(), "funnier", "the import went through");

        // The backup is what was here: the stored config and the edited prompts.
        let restored = read_file(&made).unwrap();
        assert_eq!(restored.config, AppConfig { default_workdir: None, ..configured() }, "without the work folder");
        assert_eq!(restored.prompts, owned(&[(Verify, "my check")]));

        // Undo: importing the backup puts it all back (and backs up the import in turn).
        let now = saved.last().unwrap().clone();
        let config_file = write_config_file(root.path(), &now);
        let undo_backup = replace_with_backup(&restored, &now, &dir, &config_file, root.path(), |cfg| {
            saved.push(cfg.clone());
            Ok(())
        })
        .unwrap()
        .unwrap();
        assert_ne!(undo_backup, made, "a new backup, the first one kept");
        assert!(made.exists());
        assert_eq!(saved.last().unwrap(), &current, "same models, steps, keys and work folder");
        assert_eq!(prompts::resolve(Verify, &dir).unwrap(), "my check");
        assert_eq!(prompts::resolve(ToneFunny, &dir).unwrap(), prompts::default_text(ToneFunny));
        assert_eq!(read_file(&undo_backup).unwrap().prompts, owned(&[(ToneFunny, "funnier")]));
    }

    #[test]
    fn an_edited_prompt_that_wouldnt_export_doesnt_block_an_import() {
        let (root, dir) = app_data(&[(Verify, "  \n"), (ToneXianxia, "my xianxia")]);
        let config_file = write_config_file(root.path(), &configured());
        let incoming = Imported { config: default_config(), prompts: Vec::new(), skipped: Vec::new() };
        let made = replace_with_backup(&incoming, &configured(), &dir, &config_file, root.path(), |_| Ok(()))
            .unwrap()
            .unwrap();
        // The backup holds the files as they were, the broken one included.
        let mut zip = ZipArchive::new(fs::File::open(&made).unwrap()).unwrap();
        let mut names: Vec<String> = zip.file_names().map(|n| n.unwrap().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["config.json", "prompts/tones/xianxia.txt", "prompts/verify.txt"]);
        let mut verify = String::new();
        zip.by_name("prompts/verify.txt").unwrap().read_to_string(&mut verify).unwrap();
        assert_eq!(verify, "  \n");
        let mut config = Vec::new();
        zip.by_name("config.json").unwrap().read_to_end(&mut config).unwrap();
        assert_eq!(config, fs::read(&config_file).unwrap(), "config.json byte for byte");
    }

    #[test]
    fn only_the_newest_backups_are_kept() {
        let (root, dir) = app_data(&[(Verify, "my check")]);
        let backups = root.path().join("backups");
        let config_file = root.path().join("no-config-yet.json");
        let made: Vec<PathBuf> = (1..=5)
            .map(|at| backup_setup(&backups, &config_file, &configured(), &dir, at * 1000).unwrap())
            .collect();
        let mut left = names_in(&backups);
        left.sort();
        assert_eq!(left, ["setup-backup-3000.zip", "setup-backup-4000.zip", "setup-backup-5000.zip"]);
        // The config came from the loaded one when there was no file.
        assert_eq!(read_file(&made[4]).unwrap().config.models.len(), 2);
        // Two in the same millisecond don't overwrite each other.
        let again = backup_setup(&backups, &config_file, &configured(), &dir, 5000).unwrap();
        assert_eq!(again.file_name().unwrap(), "setup-backup-5001.zip");
        // Other files are left alone.
        fs::write(backups.join("setup-backup.zip"), "an old single backup").unwrap();
        backup_setup(&backups, &config_file, &configured(), &dir, 9000).unwrap();
        assert!(backups.join("setup-backup.zip").exists());
        assert_eq!(backups_in(&backups).len(), BACKUPS_KEPT);
    }

    #[test]
    fn nothing_to_replace_means_no_backup() {
        let (root, dir) = app_data(&[]);
        let incoming = read(export(&configured(), &dir)).unwrap();
        let config_file = root.path().join("config.json");
        let made = replace_with_backup(&incoming, &default_config(), &dir, &config_file, root.path(), |_| Ok(()))
            .unwrap();
        assert_eq!(made, None);
        assert_eq!(names_in(root.path()), ["prompts"]);
    }

    #[test]
    fn a_backup_that_cant_be_written_stops_the_import() {
        let (root, dir) = app_data(&[(Verify, "my check")]);
        let not_a_dir = root.path().join("not-a-dir");
        fs::write(&not_a_dir, "a file").unwrap();
        let config_file = root.path().join("config.json");
        let incoming = Imported { config: default_config(), prompts: Vec::new(), skipped: Vec::new() };
        let mut saved = false;
        let err = replace_with_backup(&incoming, &configured(), &dir, &config_file, &not_a_dir, |_| {
            saved = true;
            Ok(())
        })
        .unwrap_err()
        .to_string();
        assert!(err.starts_with("Couldn't save a backup of your current setup"), "{err}");
        assert!(err.ends_with("so nothing was replaced."), "{err}");
        assert!(!saved, "the config wasn't touched");
        assert_eq!(prompts::resolve(Verify, &dir).unwrap(), "my check");
    }

    // ---- summary ----

    #[test]
    fn the_summary_names_the_models_and_says_whether_it_replaces_a_setup() {
        let (_root, empty) = app_data(&[]);
        let (_edited_root, edited) = app_data(&[(Verify, "mine")]);
        let imported = Imported {
            skipped: vec!["notes.txt".into()],
            ..read(export(&configured(), &edited)).unwrap()
        };
        assert_eq!(
            summary(&imported, &default_config(), &empty),
            SetupSummary {
                models: vec!["Claude Sonnet 5.5".into(), "Gemini 3 Flash".into()],
                edited_prompts: 1,
                skipped: vec!["notes.txt".into()],
                replaces_existing: false,
                backup: None,
            }
        );
        assert!(summary(&imported, &configured(), &empty).replaces_existing, "models here");
        assert!(summary(&imported, &default_config(), &edited).replaces_existing, "an edited prompt here");
    }
}
