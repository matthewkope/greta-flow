//! Gretchen Flow — menu-bar push-to-talk dictation.
//!
//! The menu bar ¿ shows the app state: quiet template glyph when idle,
//! bold glowing red while recording, amber while transcribing, with a ↓
//! suffix while the model downloads.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

mod audio;
mod bindings;
mod config;
mod engine;
mod history;
mod inject;
mod lists;
mod locks;
mod maintenance;
mod metrics;
mod model;
mod parakeet;
mod profile;
mod self_test;
mod session;
mod streaming;
mod transcribe;

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

mod commands;
mod dictation;
mod hotkey;
mod setup;
mod setup_status;
mod tray;

struct AppState {
    recorder: audio::Recorder,
    sessions: Mutex<session::Sessions>,
    bindings: Mutex<bindings::Bindings>,
    engine: Arc<Mutex<Option<engine::Engine>>>,
    recording: AtomicBool,
    engine_ready: AtomicBool,
    model_generation: AtomicU64,
    model_status: Mutex<Option<setup_status::ModelStatus>>,
    processing: AtomicBool,
    /// Active push-to-talk shortcuts (up to 3). "Fn" plus any registered
    /// accelerators. Editable from the menu/window.
    shortcuts: Mutex<Vec<String>>,
    /// Bitmask of CGEventFlags for bare-modifier hotkeys currently active
    /// (Fn, Ctrl, Cmd, Alt, Shift) — driven by the low-level flags listener.
    watched_flags: AtomicU64,
    /// When the recorder window is open to change an existing shortcut, this
    /// holds the accelerator being replaced (None means "add a new one").
    pending_replace: Mutex<Option<String>>,
    /// The active Whisper model name (changeable from the menu).
    current_model: Mutex<String>,
    active_model: Mutex<String>,
    /// Live config; replaced wholesale by "Reload Config".
    cfg: Mutex<config::Config>,
    /// Full texts behind the tray's "Recent" items, newest first.
    history_items: Mutex<Vec<String>>,
}

fn main() {
    if self_test::run_if_requested() {
        return;
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cfg = config::Config::load();
    history::configure(cfg.history.clone());
    log::info!(
        "Gretchen Flow starting (model: {}, shortcuts: {}, mode: {})",
        cfg.model,
        cfg.shortcuts.join(", "),
        cfg.hotkey_mode
    );

    let startup_plan = bindings::Plan::startup(&cfg);

    // Surface panics in the log — an unwind across a system callback aborts
    // the process with no crash report otherwise.
    std::panic::set_hook(Box::new(|info| {
        log::error!("PANIC: {info}");
    }));

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            setup_status::setup_model_status,
            setup_status::setup_disk_space,
            setup_status::setup_permissions,
            setup_status::setup_saved_step,
            setup_status::setup_save_step,
            setup_status::setup_relaunch,
            commands::apply_custom_hotkey,
            commands::cancel_custom_hotkey,
            commands::open_models_page,
            commands::open_privacy,
            commands::prime_microphone,
            commands::download_recommended_model,
            commands::close_setup,
            commands::menu_state,
            commands::menu_choose_model,
            commands::menu_model_from_file,
            commands::menu_remove_hotkey,
            commands::menu_record_hotkey,
            commands::menu_change_hotkey,
            commands::hotkey_replace_target,
            commands::remove_pending_hotkey,
            commands::cancel_dictation,
            commands::recover_dictation,
            commands::retry_dictation,
            commands::menu_reload_config,
            commands::menu_open_setup,
            commands::menu_copy_recent,
            commands::menu_clear_history,
            commands::menu_quit,
            commands::menu_close
        ])
        .manage(AppState {
            recorder: audio::Recorder::spawn(),
            sessions: Mutex::new(session::Sessions::default()),
            bindings: Mutex::new(bindings::Bindings::default()),
            engine: Arc::new(Mutex::new(None)),
            recording: AtomicBool::new(false),
            engine_ready: AtomicBool::new(false),
            model_generation: AtomicU64::new(0),
            model_status: Mutex::new(None),
            processing: AtomicBool::new(false),
            shortcuts: Mutex::new(cfg.shortcuts.clone()),
            watched_flags: AtomicU64::new(startup_plan.watched),
            pending_replace: Mutex::new(None),
            current_model: Mutex::new(cfg.model.clone()),
            active_model: Mutex::new(cfg.model.clone()),
            cfg: Mutex::new(cfg),
            history_items: Mutex::new(Vec::new()),
        })
        .setup(move |app| setup::initialize(app, startup_plan))
        .build(tauri::generate_context!());
    let app = match app {
        Ok(app) => app,
        Err(error) => {
            log::error!("error while building Gretchen Flow: {error}");
            std::process::exit(1);
        }
    };
    app.run(|app, event| match event {
        // Keep running with zero windows; only exit via the Quit menu item.
        tauri::RunEvent::ExitRequested { code, api, .. } => {
            log::info!("exit requested (code: {code:?})");
            if code.is_none() {
                api.prevent_exit();
            }
        }
        // Clicking the app icon (Finder/Dock/Launchpad) while it's already
        // running re-opens the standalone window.
        tauri::RunEvent::Reopen { .. } => setup::open_main_window(app),
        tauri::RunEvent::Exit => log::info!("exiting"),
        _ => {}
    });
}
