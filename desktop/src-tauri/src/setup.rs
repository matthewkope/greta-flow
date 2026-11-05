//! Startup, model lifecycle, permissions and existing window builders.
use crate::hotkey::{install_shortcuts, register_binding, spawn_fn_listener};
use crate::locks::MutexExt;
use crate::setup_status;
use crate::tray::{refresh_menu, set_tray_state, TrayState, ICON_APP};
use crate::{bindings, config, engine, history, model, AppState};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

/// The model recommended to first-time users (best accuracy/size balance).
pub(crate) const RECOMMENDED_MODEL: &str = config::DEFAULT_MODEL;

/// Hugging Face page listing every downloadable ggml whisper.cpp model.
pub(crate) const MODELS_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/tree/main";

pub(crate) fn set_model(app: &AppHandle, name: &str) {
    // Reject anything that isn't an absolute path to a local model file or a
    // valid ggml model name, before it's persisted or used to build a URL/path.
    if !name.starts_with('/') && !model::is_valid_model_name(name) {
        log::error!("ignoring invalid model name: {name:?}");
        return;
    }
    let state = app.state::<AppState>();
    let (previous, generation, reuse_active) = {
        let mut current = state.current_model.lock_recover();
        let previous = state.active_model.lock_recover().clone();
        let (generation, reuse_active) = select_model(
            &mut current,
            &previous,
            state.engine_ready.load(Ordering::SeqCst),
            &state.model_generation,
            name,
        );
        (previous, generation, reuse_active)
    };
    let mut cfg = config::Config::load();
    cfg.model = name.to_string();
    cfg.save();
    if reuse_active {
        state.cfg.lock_recover().model = name.to_string();
        setup_status::phase(app, generation, name, "ready");
        set_tray_state(app, TrayState::Idle);
        refresh_menu(app);
        return;
    }
    setup_status::phase(app, generation, name, "queued");
    refresh_menu(app);

    let app = app.clone();
    let name = name.to_string();
    std::thread::spawn(move || {
        set_tray_state(&app, TrayState::Downloading);
        let loaded = setup_status::load(&app, &cfg, generation);
        let state = app.state::<AppState>();
        let mut engine_slot = state.engine.lock_recover();
        let mut current = state.current_model.lock_recover();
        if state.model_generation.load(Ordering::SeqCst) != generation {
            return;
        }
        match loaded {
            Ok(engine) => {
                *engine_slot = Some(engine);
                *state.active_model.lock_recover() = cfg.model.clone();
                *state.cfg.lock_recover() = cfg.clone();
                state.engine_ready.store(true, Ordering::SeqCst);
                setup_status::phase(&app, generation, &cfg.model, "ready");
                log::info!("switched model to {name}");
            }
            Err(e) => {
                setup_status::publish(
                    &app,
                    generation,
                    &cfg.model,
                    model::Progress::phase("failed"),
                    Some(e.clone()),
                );
                log::error!("model switch to {name} failed, keeping {previous}: {e}");
                *current = previous.clone();
                let mut cfg = config::Config::load();
                cfg.model = previous;
                cfg.save();
            }
        }
        drop(current);
        drop(engine_slot);
        // Reflect the result: ready, or still no model.
        let idle = if state.engine_ready.load(Ordering::SeqCst) {
            TrayState::Idle
        } else {
            TrayState::NeedsModel
        };
        set_tray_state(&app, idle);
        refresh_menu(&app);
    });
}

/// Open (or focus) the small "press your shortcut" recorder window.
/// On startup, ask macOS for Accessibility + Microphone so Greta Flow shows
/// up in those Privacy lists (to be toggled on) instead of only appearing after
/// the first dictation. Input Monitoring already self-registers via the Fn tap.
pub(crate) fn request_startup_permissions(_app: &AppHandle) {
    prompt_accessibility();
    request_microphone();
}

/// Ask macOS for microphone access via the canonical API. This registers the
/// app under Privacy ▸ Microphone and shows the prompt (when "not determined"),
/// and is a no-op once already granted/denied.
pub(crate) fn request_microphone() {
    use block2::RcBlock;
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, Bool};
    use objc2_foundation::NSString;
    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {
        static AVMediaTypeAudio: *const NSString;
    }
    let Some(cls) = AnyClass::get(c"AVCaptureDevice") else {
        log::warn!("AVCaptureDevice unavailable; cannot request microphone");
        return;
    };
    let Some(media) = (unsafe { AVMediaTypeAudio.as_ref() }) else {
        return;
    };
    // requestAccess copies the block, so the RcBlock can drop after this call.
    let handler = RcBlock::new(|granted: Bool| {
        log::info!(
            "microphone access {}",
            if granted.as_bool() {
                "granted"
            } else {
                "denied"
            }
        );
    });
    let _: () =
        unsafe { msg_send![cls, requestAccessForMediaType: media, completionHandler: &*handler] };
}

/// Trigger the Accessibility trust prompt, which also lists the app under
/// Privacy ▸ Accessibility. No-op once already trusted.
pub(crate) fn prompt_accessibility() {
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::string::{CFString, CFStringRef};
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
        static kAXTrustedCheckOptionPrompt: CFStringRef;
    }
    unsafe {
        if AXIsProcessTrusted() {
            return;
        }
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        let opts = CFDictionary::from_CFType_pairs(&[(
            key.as_CFType(),
            CFBoolean::true_value().as_CFType(),
        )]);
        AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef());
    }
}

pub(crate) fn open_hotkey_window(app: &AppHandle) {
    activate_app();
    if let Some(window) = app.get_webview_window("hotkey") {
        let _ = window.set_focus();
        return;
    }
    let result = tauri::WebviewWindowBuilder::new(
        app,
        "hotkey",
        tauri::WebviewUrl::App("hotkey.html".into()),
    )
    .title("Set Hotkey — Greta Flow")
    .inner_size(380.0, 230.0)
    .resizable(false)
    .always_on_top(true)
    .build();
    if let Err(e) = result {
        log::error!("couldn't open hotkey window: {e}");
    }
}

/// Activate the app so panels (file dialogs) center on screen — without
/// this, an accessory app's dialogs appear in odd positions.
pub(crate) fn activate_app() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    if let Some(mtm) = MainThreadMarker::new() {
        let ns_app = NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        ns_app.activateIgnoringOtherApps(true);
    }
}

/// Set the Dock icon to the Greta artwork at runtime, so it's never the
/// generic executable icon (e.g. in dev builds, or before the bundle icon
/// is associated). Main thread only.
pub(crate) fn set_dock_icon() {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(ICON_APP);
    if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
        let ns_app = NSApplication::sharedApplication(mtm);
        unsafe { ns_app.setApplicationIconImage(Some(&image)) };
    }
}

/// Native, screen-centered open panel for choosing a Whisper model file from
/// anywhere on disk. Runs on the main thread (where menu events fire) so the
/// modal panel can present and center correctly for an accessory app.
pub(crate) fn pick_model_file(app: &AppHandle) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::{NSArray, NSString};

    activate_app();
    let Some(mtm) = MainThreadMarker::new() else {
        log::error!("model picker must run on the main thread");
        return;
    };

    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    let exts =
        NSArray::from_retained_slice(&[NSString::from_str("bin"), NSString::from_str("gguf")]);
    #[allow(deprecated)] // setAllowedContentTypes needs UTType; extensions are simpler here
    panel.setAllowedFileTypes(Some(&exts));
    panel.center();

    if panel.runModal() == NSModalResponseOK {
        if let Some(url) = panel.URLs().firstObject() {
            if let Some(path) = url.path() {
                set_model(app, &path.to_string());
            }
        }
    }
}

pub(crate) fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    if let Err(e) = std::process::Command::new("open").arg(url).spawn() {
        log::error!("couldn't open {url}: {e}");
    }
}

/// show()/set_focus() after centering it.
pub(crate) fn open_main_window(app: &AppHandle) {
    activate_app();
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    // Native window with an overlay (transparent) title bar: keeps the dark
    // themed look while giving real traffic-light controls (close / minimize /
    // fullscreen) and native title-bar dragging.
    let result =
        tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("menu.html".into()))
            .title("Greta Flow")
            .inner_size(440.0, 620.0)
            .resizable(false)
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true)
            .focused(true)
            .build();
    match result {
        Ok(window) => {
            let _ = window.center();
            let _ = window.show();
            let _ = window.set_focus();
        }
        Err(e) => log::error!("couldn't open main window: {e}"),
    }
}

pub(crate) fn show_setup(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || open_setup_window(&handle));
}

/// Open (or focus) the setup window explaining how to download a model and
/// grant permissions.
pub(crate) fn open_setup_window(app: &AppHandle) {
    activate_app();
    if let Some(window) = app.get_webview_window("setup") {
        let _ = window.set_focus();
        return;
    }
    let result =
        tauri::WebviewWindowBuilder::new(app, "setup", tauri::WebviewUrl::App("setup.html".into()))
            .title("Welcome to Greta Flow")
            .inner_size(520.0, 580.0)
            .resizable(false)
            .center()
            .build();
    if let Err(e) = result {
        log::error!("couldn't open setup window: {e}");
    }
}

/// Load the configured model at launch only when already on disk.
/// A missing model opens setup; startup never auto-downloads.
pub(crate) fn load_engine_async(app: AppHandle) {
    let generation = {
        let state = app.state::<AppState>();
        let _current = state.current_model.lock_recover();
        state.model_generation.fetch_add(1, Ordering::SeqCst) + 1
    };
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let cfg = state.cfg.lock_recover().clone();

        let available = if cfg.model.is_empty() {
            false
        } else if cfg.model.starts_with('/') {
            std::path::Path::new(&cfg.model).exists()
        } else {
            model::model_path(&cfg.model).exists()
        };
        if !available {
            setup_status::phase(&app, generation, &cfg.model, "needs_model");
            log::info!("no model downloaded yet; opening setup");
            set_tray_state(&app, TrayState::NeedsModel);
            show_setup(&app);
            return;
        }

        set_tray_state(&app, TrayState::Downloading);
        let loaded = setup_status::load(&app, &cfg, generation);
        let mut engine_slot = state.engine.lock_recover();
        let current = state.current_model.lock_recover();
        if state.model_generation.load(Ordering::SeqCst) != generation {
            return;
        }
        match loaded {
            Ok(engine) => {
                *engine_slot = Some(engine);
                *state.active_model.lock_recover() = cfg.model.clone();
                state.engine_ready.store(true, Ordering::SeqCst);
                setup_status::phase(&app, generation, &cfg.model, "ready");
                log::info!("engine ready (model: {})", cfg.model);
                set_tray_state(&app, TrayState::Idle);
            }
            Err(e) => {
                setup_status::publish(
                    &app,
                    generation,
                    &cfg.model,
                    model::Progress::phase("failed"),
                    Some(e.clone()),
                );
                log::error!("engine load failed: {e}");
                set_tray_state(&app, TrayState::Error);
            }
        }
        drop(current);
        drop(engine_slot);
        // Update the menu/window now that the engine state has settled.
        refresh_menu(&app);
    });
}

/// Force-reload the transcription engine from the current config, keeping the
/// existing engine until the new one is ready. Picks up model and
/// transcription-setting changes (language, vocabulary, punctuation, etc.).
pub(crate) fn reload_engine(app: AppHandle, cfg: config::Config, generation: u64) {
    let model = cfg.model.clone();
    setup_status::phase(&app, generation, &model, "queued");
    std::thread::spawn(move || {
        set_tray_state(&app, TrayState::Downloading);
        let loaded = setup_status::load(&app, &cfg, generation);
        let state = app.state::<AppState>();
        let mut engine_slot = state.engine.lock_recover();
        let current = state.current_model.lock_recover();
        if state.model_generation.load(Ordering::SeqCst) != generation {
            return;
        }
        match loaded {
            Ok(engine) => {
                *engine_slot = Some(engine);
                *state.active_model.lock_recover() = cfg.model.clone();
                state.engine_ready.store(true, Ordering::SeqCst);
                setup_status::phase(&app, generation, &cfg.model, "ready");
                log::info!("engine reloaded (model: {model})");
            }
            Err(e) => {
                setup_status::publish(
                    &app,
                    generation,
                    &cfg.model,
                    model::Progress::phase("failed"),
                    Some(e.clone()),
                );
                log::error!("engine reload failed, keeping current engine: {e}");
            }
        }
        drop(current);
        drop(engine_slot);
        set_tray_state(&app, TrayState::Idle);
    });
}

/// Menu action: re-read config.json and apply everything live — hotkey,
/// icon theme, model, and transcription settings — without restarting.
pub(crate) fn reload_config(app: &AppHandle) {
    let mut fresh = config::Config::load();
    history::configure(fresh.history.clone());
    let state = app.state::<AppState>();

    // Re-register the shortcuts if they changed.
    let current = state.shortcuts.lock_recover().clone();
    let old = state.cfg.lock_recover().clone();
    if fresh.shortcuts != current
        || fresh.cancel_shortcut != old.cancel_shortcut
        || fresh.retry_shortcut != old.retry_shortcut
        || fresh.recovery_shortcut != old.recovery_shortcut
    {
        if let Err(e) = install_shortcuts(app, &fresh) {
            log::error!("reload config: {e}");
            // Keep the live snapshot honest; the invalid file can be corrected
            // and retried on the next reload without losing working shortcuts.
            fresh.shortcuts = current;
            fresh.cancel_shortcut = old.cancel_shortcut;
            fresh.retry_shortcut = old.retry_shortcut;
            fresh.recovery_shortcut = old.recovery_shortcut;
        }
    }

    // Swap in the new config (covers hotkey_mode and any future fields).
    let generation = {
        let mut current = state.current_model.lock_recover();
        let generation = reload_generation(
            &state.model_generation,
            &current,
            &state.active_model.lock_recover(),
            &fresh.model,
        );
        *current = fresh.model.clone();
        *state.cfg.lock_recover() = fresh.clone();
        generation
    };

    refresh_menu(app);
    log::info!("config reloaded");

    // Formatting and recognition settings use per-session snapshots; only a
    // model change needs new weights.
    if let Some(generation) = generation {
        reload_engine(app.clone(), fresh, generation);
    }
}

pub(crate) fn initialize(
    app: &mut tauri::App,
    startup_plan: bindings::Plan,
) -> Result<(), Box<dyn std::error::Error>> {
    // Regular app: appears in the Dock (clicking it opens the window),
    // and also keeps a menu-bar tray icon for recording status.
    #[cfg(target_os = "macos")]
    {
        app.set_activation_policy(tauri::ActivationPolicy::Regular);
        set_dock_icon();
    }

    let (tick_tx, tick_rx) = std::sync::mpsc::sync_channel(1);
    let recovery_app = app.handle().clone();
    std::thread::spawn(move || {
        let mut recovery = crate::maintenance::Recovery::default();
        let mut last_worker: Option<std::sync::Weak<crate::parakeet::Engine>> = None;
        for () in tick_rx {
            let state = recovery_app.state::<AppState>();
            if state.recording.load(Ordering::SeqCst) || state.processing.load(Ordering::SeqCst) {
                continue;
            }
            let engine = match state.engine.try_lock() {
                Ok(engine) => engine,
                Err(std::sync::TryLockError::WouldBlock) => continue,
                Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                    log::warn!("recovering poisoned engine slot");
                    state.engine.clear_poison();
                    poisoned.into_inner()
                }
            };
            let worker = engine.as_ref().and_then(engine::Engine::maintenance_worker);
            drop(engine); // Model swaps must not wait for a slow helper restart.
            if let Some(worker) = worker {
                let current = Arc::downgrade(&worker);
                if last_worker.as_ref().is_none_or(|old| !old.ptr_eq(&current)) {
                    // A new instance was installed after a model load. Do not
                    // carry a failed old helper's cooldown into the new one.
                    recovery.succeeded();
                    last_worker = Some(current);
                }
                if !recovery.due(
                    state.model_generation.load(Ordering::SeqCst),
                    std::time::Instant::now(),
                ) {
                    continue;
                }
                match worker.ensure_ready() {
                    Ok(()) => recovery.succeeded(),
                    Err(error) => {
                        let delay = recovery.failed(std::time::Instant::now());
                        log::warn!(
                            "idle helper recovery failed; retry in {}s: {error}",
                            delay.as_secs()
                        );
                    }
                }
            }
        }
    });
    let expiry_app = app.handle().clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(5));
        expiry_app
            .state::<AppState>()
            .sessions
            .lock_repair(|sessions| sessions.cancel())
            .expire();
        // Reuse the expiry timer, but a helper timeout must never delay audio
        // expiration. At most one tick queues behind an in-progress restart.
        let _ = tick_tx.try_send(());
    });
    crate::tray::initialize(app.handle())?;

    // Optional malformed, colliding, or unavailable recovery controls
    // must not prevent the application (or its dictation key) starting.
    {
        let state = app.state::<AppState>();
        let mut bindings = state.bindings.lock_recover();
        let dispatch = bindings.dispatch.clone();
        bindings.startup(&startup_plan, |shortcut| {
            register_binding(app.handle(), shortcut, dispatch.clone())
        });
    }
    spawn_fn_listener(app.handle().clone());

    // Proactively ask for Accessibility + Microphone so the app shows up
    // in those Privacy lists for the user to toggle on, rather than only
    // appearing after first use.
    request_startup_permissions(app.handle());

    if setup_status::setup_saved_step().ok().flatten().is_some() {
        open_setup_window(app.handle());
    }
    load_engine_async(app.handle().clone());
    Ok(())
}

// Every explicit selection supersedes pending work, even when it returns to the
// working engine. Requested/current is not evidence that an engine loaded.
fn select_model(
    current: &mut String,
    active: &str,
    ready: bool,
    generation: &std::sync::atomic::AtomicU64,
    name: &str,
) -> (u64, bool) {
    *current = name.into();
    (
        generation.fetch_add(1, Ordering::SeqCst) + 1,
        ready && active == name,
    )
}

fn reload_generation(
    generation: &std::sync::atomic::AtomicU64,
    current: &str,
    active: &str,
    next: &str,
) -> Option<u64> {
    // A changed request must also supersede a pending switch back to a model
    // that happens to still be active. A failed load remains retryable.
    (next != current || next != active).then(|| generation.fetch_add(1, Ordering::SeqCst) + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    #[test]
    fn failed_reload_can_retry_and_returning_to_active_supersedes_pending_work() {
        let generation = AtomicU64::new(4);
        let mut current = "failed".to_string();
        let (retry, reuse) = select_model(&mut current, "working", true, &generation, "failed");
        assert_eq!(retry, 5);
        assert!(!reuse);
        let (back, reuse) = select_model(&mut current, "working", true, &generation, "working");
        assert_eq!(back, 6);
        assert!(reuse);
        assert_eq!(current, "working");
        assert_ne!(retry, generation.load(Ordering::SeqCst));
        let (_, reuse) = select_model(&mut current, "working", false, &generation, "working");
        assert!(!reuse);
    }

    #[test]
    fn unchanged_config_does_not_supersede_startup() {
        let generation = AtomicU64::new(1);
        assert_eq!(
            reload_generation(&generation, "model", "model", "model"),
            None
        );
        assert_eq!(generation.load(Ordering::SeqCst), 1);
        assert_eq!(
            reload_generation(&generation, "pending", "model", "model"),
            Some(2)
        );
        assert_eq!(generation.load(Ordering::SeqCst), 2);
    }
}
