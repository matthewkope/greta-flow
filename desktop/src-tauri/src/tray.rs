//! Tray state and menus. Existing labels and presentation are preserved.
use crate::commands::open_models_page;
use crate::hotkey::MAX_SHORTCUTS;
use crate::locks::MutexExt;
use crate::setup::{open_hotkey_window, pick_model_file, reload_config, set_model, show_setup};
use crate::{config, history, inject, model, AppState};
use std::sync::atomic::Ordering;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

pub(crate) const TRAY_ID: &str = "gf";
pub(crate) const ICON_APP: &[u8] = include_bytes!("../icons/icon.png");
/// Menu-bar icons: plain white Gretchen outline when idle, white Gretchen on a
/// black badge while recording (shown alongside macOS's orange mic indicator).
pub(crate) const ICON_IDLE: &[u8] = include_bytes!("../icons/tray/idle.png");
pub(crate) const ICON_RECORDING: &[u8] = include_bytes!("../icons/tray/recording.png");

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum TrayState {
    Downloading,
    Idle,
    Recording,
    Transcribing,
    NeedsModel,
    Error,
}

/// Whisper model choices offered in the tray menu: (ggml name, display label).
pub(crate) const MODEL_CHOICES: &[(&str, &str)] = &[
    (
        config::DEFAULT_MODEL,
        "Parakeet v2 — fast English (default)",
    ),
    ("large-v3-turbo-q5_0", "Large v3 Turbo quantized — 547 MB"),
    ("large-v3-turbo", "Large v3 Turbo — 1.6 GB, max accuracy"),
    ("small", "Small — 466 MB, lighter"),
    ("base", "Base — 142 MB, fastest"),
];

/// Apply a tray state on the main thread (AppKit UI is not thread-safe).
pub(crate) fn set_tray_state(app: &AppHandle, state: TrayState) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || set_tray_state_on_main(&handle, state));
}

thread_local! {
    /// Last applied (state, dark) — skip no-op tray updates so an open
    /// dropdown isn't disturbed by redundant icon/title writes.
    static LAST_TRAY: std::cell::Cell<Option<(TrayState, bool)>> =
        const { std::cell::Cell::new(None) };
}

pub(crate) fn set_tray_state_on_main(app: &AppHandle, state: TrayState) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    if LAST_TRAY.with(|c| c.get()) == Some((state, false)) {
        return;
    }
    LAST_TRAY.with(|c| c.set(Some((state, false))));
    let (bytes, title) = match state {
        TrayState::Idle => (ICON_IDLE, ""),
        TrayState::Downloading => (ICON_IDLE, "↓"),
        TrayState::Recording => (ICON_RECORDING, ""),
        TrayState::Transcribing => (ICON_IDLE, ""),
        TrayState::NeedsModel => (ICON_IDLE, "!"),
        TrayState::Error => (ICON_IDLE, "✕"),
    };
    let _ = tray.set_icon(Image::from_bytes(bytes).ok());
    let _ = tray.set_icon_as_template(false);
    // Always set an explicit title: clearing with None doesn't reliably
    // remove the previous text on macOS.
    let _ = tray.set_title(Some(title));
}

/// Human-friendly label for a shortcut accelerator (e.g. the Fn/Globe key).
pub(crate) fn hotkey_label(accel: &str) -> String {
    match accel {
        "Fn" => "Fn  (🌐 Globe key)".to_string(),
        "Ctrl" => "⌃ Control  (held alone)".to_string(),
        "Cmd" => "⌘ Command  (held alone)".to_string(),
        "Alt" => "⌥ Option  (held alone)".to_string(),
        "Shift" => "⇧ Shift  (held alone)".to_string(),
        _ => accel.to_string(),
    }
}

/// Replace one push-to-talk shortcut with another, preserving its position.
pub(crate) const HISTORY_MENU_ITEMS: usize = 5;

/// Handles to the menu items that change at runtime. The menu is built once
/// and mutated in place — replacing a tray menu (or its submenus) while macOS
/// has shown it once makes the dropdown glitch on the next hover.
pub(crate) struct MenuHandles {
    menu: Menu<tauri::Wry>,
    status: MenuItem<tauri::Wry>,
    hist_header: MenuItem<tauri::Wry>,
    hist: Vec<MenuItem<tauri::Wry>>,
    model_items: Vec<CheckMenuItem<tauri::Wry>>,
    model_custom: CheckMenuItem<tauri::Wry>,
    /// Up to MAX_SHORTCUTS slots, each showing an active shortcut (click to
    /// remove) or "(empty)" when unused.
    hotkey_slots: Vec<MenuItem<tauri::Wry>>,
    hotkey_add: MenuItem<tauri::Wry>,
}

thread_local! {
    /// Menu handles live on the main thread only (AppKit menu items are not
    /// thread-safe); all access goes through run_on_main_thread.
    static MENU_HANDLES: std::cell::RefCell<Option<MenuHandles>> =
        const { std::cell::RefCell::new(None) };
}

/// Build the full tray menu once, with fixed slots for everything dynamic.
pub(crate) fn build_menu(app: &AppHandle) -> tauri::Result<MenuHandles> {
    let menu = Menu::new(app)?;

    let status = MenuItem::with_id(app, "status", "Greta Flow", false, None::<&str>)?;
    menu.append(&status)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;

    let hist_header =
        MenuItem::with_id(app, "hist-header", "No dictations yet", false, None::<&str>)?;
    menu.append(&hist_header)?;
    let mut hist = Vec::new();
    for i in 0..HISTORY_MENU_ITEMS {
        let item = MenuItem::with_id(app, format!("hist-{i}"), "—", false, None::<&str>)?;
        menu.append(&item)?;
        hist.push(item);
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;

    let model_menu = Submenu::with_id(app, "model-menu", "Model", true)?;
    let mut model_items = Vec::new();
    for (name, label) in MODEL_CHOICES {
        let item = CheckMenuItem::with_id(
            app,
            format!("model-{name}"),
            *label,
            true,
            false,
            None::<&str>,
        )?;
        model_menu.append(&item)?;
        model_items.push(item);
    }
    let model_custom =
        CheckMenuItem::with_id(app, "model-custom-slot", "—", false, false, None::<&str>)?;
    model_menu.append(&model_custom)?;
    model_menu.append(&PredefinedMenuItem::separator(app)?)?;
    model_menu.append(&MenuItem::with_id(
        app,
        "browse-models",
        "Browse Models on Hugging Face…",
        true,
        None::<&str>,
    )?)?;
    model_menu.append(&MenuItem::with_id(
        app,
        "model-file",
        "Model from File…",
        true,
        None::<&str>,
    )?)?;
    menu.append(&model_menu)?;

    let hotkey_menu = Submenu::with_id(app, "hotkey-menu", "Shortcuts", true)?;
    let mut hotkey_slots = Vec::new();
    for i in 0..MAX_SHORTCUTS {
        let item = MenuItem::with_id(app, format!("hotkey-slot-{i}"), "—", false, None::<&str>)?;
        hotkey_menu.append(&item)?;
        hotkey_slots.push(item);
    }
    hotkey_menu.append(&PredefinedMenuItem::separator(app)?)?;
    let hotkey_add = MenuItem::with_id(app, "add-hotkey", "Add Shortcut…", true, None::<&str>)?;
    hotkey_menu.append(&hotkey_add)?;
    menu.append(&hotkey_menu)?;

    menu.append(&MenuItem::with_id(
        app,
        "setup",
        "Run Setup Again…",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "reload-config",
        "Reload Config",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "quit",
        "Quit Greta Flow",
        true,
        None::<&str>,
    )?)?;

    Ok(MenuHandles {
        menu,
        status,
        hist_header,
        hist,
        model_items,
        model_custom,
        hotkey_slots,
        hotkey_add,
    })
}

/// Update the tray menu in place on the main thread.
pub(crate) fn refresh_menu(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || refresh_menu_on_main(&handle));
}

pub(crate) fn refresh_menu_on_main(app: &AppHandle) {
    let state = app.state::<AppState>();
    let recent = history::recent(HISTORY_MENU_ITEMS);
    let shortcuts = state.shortcuts.lock_recover().clone();
    let current_model = state.current_model.lock_recover().clone();
    let hotkey_mode = state.cfg.lock_recover().hotkey_mode.clone();
    let has_engine = state.engine_ready.load(Ordering::SeqCst);

    MENU_HANDLES.with(|handles| {
        let handles = handles.borrow();
        let Some(h) = handles.as_ref() else { return };

        let _ = h.status.set_text(if has_engine {
            format!("Greta Flow — {} ({hotkey_mode})", shortcuts.join(" / "))
        } else {
            "No model — open Model ▸ to download one".to_string()
        });

        let _ = h.hist_header.set_text(if recent.is_empty() {
            "No dictations yet"
        } else {
            "Recent — click to type again"
        });
        for (i, item) in h.hist.iter().enumerate() {
            match recent.get(i) {
                Some(text) => {
                    let label: String = if text.chars().count() > 45 {
                        format!("{}…", text.chars().take(45).collect::<String>())
                    } else {
                        text.clone()
                    };
                    let _ = item.set_text(label);
                    let _ = item.set_enabled(true);
                }
                None => {
                    let _ = item.set_text("—");
                    let _ = item.set_enabled(false);
                }
            }
        }

        let mut model_listed = false;
        for ((name, label), item) in MODEL_CHOICES.iter().zip(&h.model_items) {
            let checked = *name == current_model;
            model_listed |= checked;
            let text = if model::model_path(name).exists() {
                (*label).to_string()
            } else {
                format!("{label}  (needs download)")
            };
            let _ = item.set_text(text);
            let _ = item.set_checked(checked);
        }
        if model_listed || current_model.is_empty() {
            // No custom model in use (or none selected at all).
            let _ = h.model_custom.set_text("—");
            let _ = h.model_custom.set_enabled(false);
            let _ = h.model_custom.set_checked(false);
        } else {
            // File-path models display as their file name.
            let label = if current_model.starts_with('/') {
                std::path::Path::new(&current_model)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| current_model.clone())
            } else {
                current_model.clone()
            };
            let _ = h.model_custom.set_text(label);
            let _ = h.model_custom.set_enabled(true);
            let _ = h.model_custom.set_checked(true);
        }

        for (i, item) in h.hotkey_slots.iter().enumerate() {
            match shortcuts.get(i) {
                Some(accel) => {
                    // Clicking a slot opens the recorder to change/remove it.
                    let _ = item.set_text(format!("{}  ▸ change…", hotkey_label(accel)));
                    let _ = item.set_enabled(true);
                }
                None => {
                    let _ = item.set_text("(empty)");
                    let _ = item.set_enabled(false);
                }
            }
        }
        let _ = h.hotkey_add.set_enabled(shortcuts.len() < MAX_SHORTCUTS);
    });

    *state.history_items.lock_recover() = recent;

    // Keep the standalone window in sync if it's open.
    use tauri::Emitter;
    let _ = app.emit("menu-updated", ());
}

pub(crate) fn on_menu_event(app: &AppHandle, id: &str) {
    log::info!("menu event: {id}");
    if id == "quit" {
        app.exit(0);
        return;
    }
    if id == "model-file" {
        pick_model_file(app);
        return;
    }
    if id == "reload-config" {
        reload_config(app);
        return;
    }
    if id == "setup" {
        show_setup(app);
        return;
    }
    if id == "add-hotkey" {
        open_hotkey_window(app);
        return;
    }
    if id == "browse-models" {
        open_models_page();
        return;
    }
    if id == "model-custom-slot" {
        return; // already-active custom entry — nothing to do
    }
    // Clicking a filled shortcut slot opens the recorder to change it
    // (the recorder also offers a Remove button).
    if let Some(idx) = id
        .strip_prefix("hotkey-slot-")
        .and_then(|s| s.parse::<usize>().ok())
    {
        let accel = app
            .state::<AppState>()
            .shortcuts
            .lock_recover()
            .get(idx)
            .cloned();
        if let Some(accel) = accel {
            *app.state::<AppState>().pending_replace.lock_recover() = Some(accel);
            open_hotkey_window(app);
        }
        return;
    }
    if let Some(name) = id.strip_prefix("model-") {
        if name != "menu" {
            set_model(app, name);
        }
        return;
    }
    let Some(idx) = id
        .strip_prefix("hist-")
        .and_then(|s| s.parse::<usize>().ok())
    else {
        return;
    };
    let text = {
        let state = app.state::<AppState>();
        let items = state.history_items.lock_recover();
        items.get(idx).cloned()
    };
    if let Some(text) = text {
        let app = app.clone();
        std::thread::spawn(move || {
            // Give the menu a beat to close and focus to return to the app
            // the user was in.
            std::thread::sleep(std::time::Duration::from_millis(300));
            if let Err(e) = inject::type_text(&app, &text) {
                log::error!("{e}");
            }
        });
    }
}

pub(crate) fn initialize(app: &AppHandle) -> tauri::Result<()> {
    let handles = build_menu(app)?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(ICON_IDLE)?)
        .icon_as_template(false)
        .menu(&handles.menu)
        .on_menu_event(|app, event| on_menu_event(app, event.id().as_ref()))
        .build(app)?;
    MENU_HANDLES.with(|cell| *cell.borrow_mut() = Some(handles));
    refresh_menu(app);
    Ok(())
}
