//! polygluttony — Tauri shell.
//!
//! This crate hosts both the desktop shell (Tauri commands + event emission)
//! and the translation engine. The engine modules below are pure Rust and exist
//! solely to serve the webview through the commands in [`commands`].


mod commands;
mod config;
mod error;
mod events;
mod models;

// Translation engine. Ported from the original Python `subs_translator` package;
// implementation lands here incrementally.
mod ass;
mod utils;
mod glossary;
mod llm;
mod prompts;
mod translation;
mod validation;

/// Work around a GTK3/NVIDIA crash on Wayland.
///
/// NVIDIA's EGL driver negotiates explicit sync (`wp_linux_drm_syncobj`) on the
/// toplevel surface, but GTK3 then attaches buffers and commits without setting
/// an acquire point. The compositor rejects that as a protocol violation and
/// kills us: the window paints once, blank, then dies with
/// `Error 71 (Protocol error) dispatching to Wayland display`.
///
/// Opting out of explicit sync falls back to implicit sync and renders fine.
/// The variable is NVIDIA-specific and ignored by other drivers, and we never
/// override an existing value so it stays overridable from the environment.
#[cfg(target_os = "linux")]
fn disable_nvidia_explicit_sync() {
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var_os("__NV_DISABLE_EXPLICIT_SYNC").is_none()
    {
        std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
    }
}

/// Entry point invoked from `main.rs` (and the mobile entry point).
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Must happen before Tauri initialises GTK and brings up the EGL surface.
    #[cfg(target_os = "linux")]
    disable_nvidia_explicit_sync();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .manage(translation::run::RunState::default())
        .manage(translation::live::LiveBoard::default())
        .manage(glossary::run::GlossaryRunState::default())
        .manage(glossary::watch::GlossaryWatchState::default())
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::get_connections,
            commands::list_providers,
            commands::model_capabilities,
            commands::save_model,
            commands::remove_model,
            commands::set_step,
            commands::set_shared_steps,
            commands::show_every_step,
            commands::use_one_model,
            commands::first_run_status,
            commands::test_connection,
            commands::list_models,
            commands::list_languages,
            commands::list_recents,
            commands::remove_recent,
            commands::clear_recents,
            commands::save_folder_prefs,
            commands::set_default_languages,
            commands::open_folder,
            commands::start_translation,
            commands::cancel_translation,
            commands::episode_snapshot,
            commands::open_episode_window,
            commands::load_glossary,
            commands::save_glossary,
            commands::start_glossary_build,
            commands::cancel_glossary_build,
            commands::normalize_glossary,
            commands::import_reference_files,
            commands::reference_status,
            commands::clear_reference,
            commands::load_reference,
            commands::save_reference,
            commands::import_glossary,
            commands::glossary_backup_status,
            commands::export_glossary,
            commands::open_glossary_editor,
            commands::watch_glossary,
            commands::unwatch_glossary,
            commands::web_search_status,
            commands::list_prompts,
            commands::get_prompt,
            commands::save_prompt,
            commands::reset_prompt,
            commands::read_prompt_file,
        ])
        // Tauri quits only once every window is gone, so an open episode
        // window would keep the app (and its run) alive with no way back to
        // Cancel. Closing the main window quits, as it did before.
        .on_window_event(|window, event| {
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                tauri::Manager::app_handle(window).exit(0);
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
