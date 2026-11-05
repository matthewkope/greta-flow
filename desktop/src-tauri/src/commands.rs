//! Stable Tauri command surface for the existing windows.
use crate::dictation::deliver_result;
use crate::hotkey::{add_shortcut, change_shortcut, remove_shortcut, MAX_SHORTCUTS};
use crate::locks::MutexExt;
use crate::metrics;
use crate::setup::{
    open_hotkey_window, open_setup_window, open_url, pick_model_file, reload_config, set_model,
    MODELS_URL, RECOMMENDED_MODEL,
};
use crate::tray::{refresh_menu, set_tray_state, TrayState, HISTORY_MENU_ITEMS, MODEL_CHOICES};
use crate::{history, model, AppState};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};

#[tauri::command]
pub(crate) fn cancel_dictation(app: AppHandle) {
    let state = app.state::<AppState>();
    {
        let mut sessions = state.sessions.lock_repair(|sessions| sessions.cancel());
        sessions.cancel();
        if state.recording.swap(false, Ordering::SeqCst) {
            let _ = state.recorder.stop_async();
        }
    }
    set_tray_state(&app, TrayState::Idle);
}

#[tauri::command]
pub(crate) fn recover_dictation(app: AppHandle) -> Option<String> {
    app.state::<AppState>()
        .sessions
        .lock_repair(|sessions| sessions.cancel())
        .recovery()
}

#[tauri::command]
pub(crate) fn retry_dictation(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.recording.load(Ordering::SeqCst) || state.processing.swap(true, Ordering::SeqCst) {
        return Err("Dictation is busy".into());
    }
    let retry = state
        .sessions
        .lock_repair(|sessions| sessions.cancel())
        .retry_audio();
    let Some((id, samples)) = retry else {
        state.processing.store(false, Ordering::SeqCst);
        return Err("No unexpired, unattempted recording to retry".into());
    };
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let cfg = state
            .sessions
            .lock_repair(|sessions| sessions.cancel())
            .get(id)
            .map(|j| j.cfg.clone())
            .unwrap_or_default();
        let result = state
            .engine
            .lock_recover()
            .as_ref()
            .ok_or("No engine".into())
            .and_then(|e| e.transcribe(&samples, &cfg));
        if let Ok(text) = result {
            deliver_result(
                &app,
                id,
                text,
                &mut metrics::Timing::new(std::time::Instant::now()),
            );
        }
        state.processing.store(false, Ordering::SeqCst);
        set_tray_state(&app, TrayState::Idle);
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// Standalone NES-style window mirroring the tray menu
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
pub(crate) struct MenuChoice {
    id: String,
    label: String,
    active: bool,
    note: String,
}

#[derive(serde::Serialize)]
pub(crate) struct MenuState {
    has_engine: bool,
    status: String,
    models: Vec<MenuChoice>,
    custom_model: Option<String>,
    /// Active push-to-talk shortcuts, in order (first is the default Fn).
    shortcuts: Vec<String>,
    /// Whether another shortcut can still be added (fewer than MAX_SHORTCUTS).
    can_add_shortcut: bool,
    recent: Vec<String>,
}

/// Snapshot of everything the window renders.
#[tauri::command]
pub(crate) fn menu_state(app: AppHandle) -> MenuState {
    let state = app.state::<AppState>();
    let current_model = state.current_model.lock_recover().clone();
    let shortcuts = state.shortcuts.lock_recover().clone();
    let hotkey_mode = state.cfg.lock_recover().hotkey_mode.clone();
    let has_engine = state.engine_ready.load(Ordering::SeqCst);
    let recent = history::recent(HISTORY_MENU_ITEMS);

    let models = MODEL_CHOICES
        .iter()
        .map(|(name, label)| MenuChoice {
            id: (*name).to_string(),
            label: (*label).to_string(),
            active: *name == current_model,
            note: if model::model_path(name).exists() {
                String::new()
            } else {
                "needs download".to_string()
            },
        })
        .collect();
    let model_listed = MODEL_CHOICES.iter().any(|(n, _)| *n == current_model);
    let custom_model = if model_listed || current_model.is_empty() {
        None
    } else if current_model.starts_with('/') {
        std::path::Path::new(&current_model)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
    } else {
        Some(current_model.clone())
    };

    MenuState {
        has_engine,
        status: if has_engine {
            format!("READY · {} · {hotkey_mode}", shortcuts.join(" / "))
        } else {
            "NO MODEL — PICK ONE BELOW".to_string()
        },
        models,
        custom_model,
        can_add_shortcut: shortcuts.len() < MAX_SHORTCUTS,
        shortcuts,
        recent,
    }
}

#[tauri::command]
pub(crate) fn menu_choose_model(app: AppHandle, name: String) {
    set_model(&app, &name);
}

#[tauri::command]
pub(crate) fn menu_model_from_file(app: AppHandle) {
    pick_model_file(&app);
}

/// Open the Hugging Face model list in the default browser so the user can
/// download any ggml model, then load it via "from file…".
#[tauri::command]
pub(crate) fn open_models_page() {
    open_url(MODELS_URL);
}

/// Open a URL in the user's default browser.
/// Open a specific macOS Privacy & Security pane so the user can grant a
/// permission Greta Flow needs.
#[tauri::command]
pub(crate) fn open_privacy(pane: String) {
    let anchor = match pane.as_str() {
        "microphone" => "Privacy_Microphone",
        "accessibility" => "Privacy_Accessibility",
        "input" => "Privacy_ListenEvent", // Input Monitoring (for the Fn key)
        _ => return,
    };
    open_url(&format!(
        "x-apple.systempreferences:com.apple.preference.security?{anchor}"
    ));
}

/// Briefly open the microphone so macOS shows its permission prompt during
/// setup (cpal triggers the TCC prompt the first time the input device opens).
#[tauri::command]
pub(crate) fn prime_microphone(app: AppHandle) {
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        state.recorder.start();
        std::thread::sleep(std::time::Duration::from_millis(250));
        let _ = state.recorder.stop();
    });
}

#[tauri::command]
pub(crate) fn menu_remove_hotkey(app: AppHandle, accel: String) {
    remove_shortcut(&app, &accel);
}

/// Open the recorder to ADD a new shortcut.
#[tauri::command]
pub(crate) fn menu_record_hotkey(app: AppHandle) {
    *app.state::<AppState>().pending_replace.lock_recover() = None;
    open_hotkey_window(&app);
}

/// Open the recorder to CHANGE an existing shortcut to anything else.
#[tauri::command]
pub(crate) fn menu_change_hotkey(app: AppHandle, accel: String) {
    *app.state::<AppState>().pending_replace.lock_recover() = Some(accel);
    open_hotkey_window(&app);
}

/// The shortcut the recorder is currently set to replace (None = add mode).
#[tauri::command]
pub(crate) fn hotkey_replace_target(app: AppHandle) -> Option<String> {
    app.state::<AppState>()
        .pending_replace
        .lock_recover()
        .clone()
}

#[tauri::command]
pub(crate) fn menu_reload_config(app: AppHandle) {
    reload_config(&app);
}

#[tauri::command]
pub(crate) fn menu_open_setup(app: AppHandle) {
    open_setup_window(&app);
}

/// Copy a recent transcription to the system clipboard.
#[tauri::command]
pub(crate) fn menu_copy_recent(index: usize) {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    use objc2_foundation::NSString;
    let recent = history::recent(HISTORY_MENU_ITEMS);
    let Some(text) = recent.get(index) else {
        return;
    };
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    let ok =
        unsafe { pasteboard.setString_forType(&NSString::from_str(text), NSPasteboardTypeString) };
    if !ok {
        log::error!("failed to copy recent dictation to clipboard");
    }
}

/// Erase all stored dictation history and refresh the menu/window.
#[tauri::command]
pub(crate) fn menu_clear_history(app: AppHandle) {
    history::clear();
    refresh_menu(&app);
}

#[tauri::command]
pub(crate) fn menu_quit(app: AppHandle) {
    app.exit(0);
}

/// Open (or focus) the standalone NES-style main window. Stays a menu-bar
/// (Accessory) app — no Dock icon — and surfaces the window explicitly with
#[tauri::command]
pub(crate) fn menu_close(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.close();
    }
}

/// Show the first-run / setup guide on the main thread.
#[tauri::command]
pub(crate) fn download_recommended_model(app: AppHandle) {
    set_model(&app, RECOMMENDED_MODEL);
    if let Some(window) = app.get_webview_window("setup") {
        let _ = window.close();
    }
}

#[tauri::command]
pub(crate) fn close_setup(app: AppHandle) {
    if let Some(window) = app.get_webview_window("setup") {
        let _ = window.close();
    }
}

#[tauri::command]
pub(crate) fn apply_custom_hotkey(app: AppHandle, accel: String) -> Result<(), String> {
    let target = app
        .state::<AppState>()
        .pending_replace
        .lock_recover()
        .take();
    match target {
        Some(old) => change_shortcut(&app, &old, &accel)?,
        None => add_shortcut(&app, &accel)?,
    }
    if let Some(window) = app.get_webview_window("hotkey") {
        let _ = window.close();
    }
    Ok(())
}

/// Recorder "Remove" button: drop the shortcut being changed.
#[tauri::command]
pub(crate) fn remove_pending_hotkey(app: AppHandle) {
    let target = app
        .state::<AppState>()
        .pending_replace
        .lock_recover()
        .take();
    if let Some(accel) = target {
        remove_shortcut(&app, &accel);
    }
    if let Some(window) = app.get_webview_window("hotkey") {
        let _ = window.close();
    }
}

#[tauri::command]
pub(crate) fn cancel_custom_hotkey(app: AppHandle) {
    *app.state::<AppState>().pending_replace.lock_recover() = None;
    if let Some(window) = app.get_webview_window("hotkey") {
        let _ = window.close();
    }
}
