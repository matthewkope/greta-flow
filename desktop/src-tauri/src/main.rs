//! Gretchen Flow — menu-bar push-to-talk dictation.
//!
//! The menu bar ¿ shows the app state: quiet template glyph when idle,
//! bold glowing red while recording, amber while transcribing, with a ↓
//! suffix while the model downloads.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod config;
mod engine;
mod history;
mod inject;
mod lists;
mod metrics;
mod model;
mod parakeet;
mod profile;
mod self_test;
mod session;
mod streaming;
mod transcribe;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

const TRAY_ID: &str = "gf";
const ICON_APP: &[u8] = include_bytes!("../icons/icon.png");
/// Menu-bar icons: plain white Gretchen outline when idle, white Gretchen on a
/// black badge while recording (shown alongside macOS's orange mic indicator).
const ICON_IDLE: &[u8] = include_bytes!("../icons/tray/idle.png");
const ICON_RECORDING: &[u8] = include_bytes!("../icons/tray/recording.png");

#[derive(Clone, Copy, PartialEq)]
enum TrayState {
    Downloading,
    Idle,
    Recording,
    Transcribing,
    NeedsModel,
    Error,
}

struct AppState {
    recorder: audio::Recorder,
    sessions: Mutex<session::Sessions>,
    engine: Arc<Mutex<Option<engine::Engine>>>,
    recording: AtomicBool,
    engine_ready: AtomicBool,
    model_generation: AtomicU64,
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

/// Bare modifier keys usable as push-to-talk hotkeys. Like Fn, macOS can't
/// register a lone modifier as a global shortcut, so these are watched by the
/// same low-level flagsChanged listener. Returns the CGEventFlags bit.
fn bare_modifier_flag(accel: &str) -> Option<u64> {
    match accel {
        "Fn" => Some(1 << 23),    // kCGEventFlagMaskSecondaryFn
        "Ctrl" => Some(1 << 18),  // kCGEventFlagMaskControl
        "Cmd" => Some(1 << 20),   // kCGEventFlagMaskCommand
        "Alt" => Some(1 << 19),   // kCGEventFlagMaskAlternate
        "Shift" => Some(1 << 17), // kCGEventFlagMaskShift
        _ => None,
    }
}

/// Maximum number of simultaneous push-to-talk shortcuts.
const MAX_SHORTCUTS: usize = 3;

/// The model recommended to first-time users (best accuracy/size balance).
const RECOMMENDED_MODEL: &str = config::DEFAULT_MODEL;

/// Hugging Face page listing every downloadable ggml whisper.cpp model.
const MODELS_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/tree/main";

/// Whisper model choices offered in the tray menu: (ggml name, display label).
const MODEL_CHOICES: &[(&str, &str)] = &[
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
fn set_tray_state(app: &AppHandle, state: TrayState) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || set_tray_state_on_main(&handle, state));
}

thread_local! {
    /// Last applied (state, dark) — skip no-op tray updates so an open
    /// dropdown isn't disturbed by redundant icon/title writes.
    static LAST_TRAY: std::cell::Cell<Option<(TrayState, bool)>> =
        const { std::cell::Cell::new(None) };
}

fn set_tray_state_on_main(app: &AppHandle, state: TrayState) {
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

fn start_recording(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.processing.load(Ordering::SeqCst) || state.recording.load(Ordering::SeqCst) {
        return;
    }
    if !state.engine_ready.load(Ordering::SeqCst) {
        log::warn!("no model loaded; opening setup");
        show_setup(app);
        return;
    }
    state.recording.store(true, Ordering::SeqCst);
    let id = state.sessions.lock().unwrap().begin();
    let handle = app.clone();
    std::thread::spawn(move || {
        let target = inject::Target::capture().ok();
        let state = handle.state::<AppState>();
        let mut sessions = state.sessions.lock().unwrap();
        if let Some(job) = sessions.get(id) {
            if state.recording.load(Ordering::SeqCst) {
                let bundle = target.as_ref().and_then(|t| objc2_app_kit::NSRunningApplication::runningApplicationWithProcessIdentifier(t.pid)).and_then(|a| a.bundleIdentifier()).map(|s| s.to_string());
                job.cfg = state.cfg.lock().unwrap().for_app(bundle.as_deref());
                job.target = target;
                if job.cfg.streaming {
                    let (input, worker) =
                        streaming::Worker::spawn(state.engine.clone(), job.cfg.clone());
                    job.stream = Some(worker);
                    state.recorder.start_stream(input);
                } else {
                    state.recorder.start();
                }
            }
        }
    });
    set_tray_state(app, TrayState::Recording);
}

fn stop_and_transcribe(app: &AppHandle) {
    let release = std::time::Instant::now();
    let state = app.state::<AppState>();
    if !state.recording.swap(false, Ordering::SeqCst) {
        return;
    }
    state.processing.store(true, Ordering::SeqCst);
    // Queue stop now; waiting for the audio device must never block a hotkey callback.
    let (id, recording) = {
        let mut sessions = state.sessions.lock().unwrap();
        (sessions.id().unwrap_or(0), state.recorder.stop_async())
    };
    set_tray_state(app, TrayState::Transcribing);
    let app = app.clone();
    std::thread::spawn(move || {
        struct Finished(AppHandle);
        impl Drop for Finished {
            fn drop(&mut self) {
                set_tray_state(&self.0, TrayState::Idle);
                self.0
                    .state::<AppState>()
                    .processing
                    .store(false, Ordering::SeqCst);
            }
        }
        let _finished = Finished(app.clone());
        let mut timing = metrics::Timing::new(release);
        let recording = match recording.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(recording) => recording,
            Err(e) => {
                log::error!("microphone stop failed: {e}");
                return;
            }
        };
        timing.stage("capture_stop", release);
        timing.speech(recording.last_speech);
        let prep_start = std::time::Instant::now();
        let seconds = recording.samples.len() as f32 / recording.sample_rate as f32;
        if seconds < 0.3 {
            return;
        }
        if recording.samples.iter().any(|v| !v.is_finite()) {
            timing.emit("capture_limit_or_invalid_audio");
            return;
        }
        let samples = streaming::prepare(&audio::resample_to_16k(&recording));
        if samples.is_empty() {
            timing.emit("silence");
            return;
        }
        timing.stage("resample_gate_gain", prep_start);
        let state = app.state::<AppState>();
        state.sessions.lock().unwrap().retain_audio(id, &samples);
        let cfg = match state.sessions.lock().unwrap().get(id) {
            Some(job) => job.cfg.clone(),
            None => return,
        };
        let stream = state
            .sessions
            .lock()
            .unwrap()
            .get(id)
            .and_then(|j| j.stream.take());
        let stream_start = std::time::Instant::now();
        let streamed = stream.map(|worker| worker.finish());
        timing.stage("stream_tail_wait", stream_start);
        let result = match streamed {
            Some(Ok(text)) => Ok(text),
            fallback => {
                if fallback.is_some() {
                    timing.stages_ms.insert("whole_clip_fallback_count", 1.);
                }
                if state.sessions.lock().unwrap().get(id).is_none() {
                    return;
                }
                let wait_start = std::time::Instant::now();
                let guard = state.engine.lock().unwrap();
                timing.stage("engine_wait", wait_start);
                let inference_start = std::time::Instant::now();
                let result = match guard.as_ref() {
                    Some(engine) => engine.transcribe(&samples, &cfg),
                    None => Err("model not loaded".into()),
                };
                timing.stage("recognition_and_format", inference_start);
                result
            }
        };
        timing.mark("release_to_final_text");
        match result {
            Ok(text) if !text.is_empty() => {
                let outcome = deliver_result(&app, id, text, &mut timing);
                timing.emit(outcome);
            }
            Ok(_) => log::info!("no speech detected"),
            Err(e) => {
                timing.emit("recognition_failed");
                log::error!("transcription failed: {e}");
            }
        }
        set_tray_state(&app, TrayState::Idle);
    });
}

fn deliver_result(
    app: &AppHandle,
    id: u64,
    text: String,
    timing: &mut metrics::Timing,
) -> &'static str {
    let insertion = std::time::Instant::now();
    let state = app.state::<AppState>();
    let result = state
        .sessions
        .lock()
        .unwrap()
        .deliver(id, text.clone(), |target, text| {
            target
                .ok_or("No safe destination captured; use recovery text")?
                .insert(text)
        });
    timing.stage("insertion", insertion);
    match result {
        Ok(true) => {
            timing.mark("release_to_inserted");
            timing.inserted();
            let persistence = std::time::Instant::now();
            history::append(&text);
            timing.stage("persistence_enqueue", persistence);
            let handle = app.clone();
            std::thread::spawn(move || refresh_menu(&handle));
            "inserted"
        }
        Ok(false) => "cancelled_or_stale",
        Err(e) => {
            log::warn!("{e}");
            "recoverable"
        }
    }
}

#[tauri::command]
fn cancel_dictation(app: AppHandle) {
    let state = app.state::<AppState>();
    state.sessions.lock().unwrap().cancel();
    if state.recording.swap(false, Ordering::SeqCst) {
        let _ = state.recorder.stop_async();
    }
    set_tray_state(&app, TrayState::Idle);
}

#[tauri::command]
fn recover_dictation(app: AppHandle) -> Option<String> {
    app.state::<AppState>().sessions.lock().unwrap().recovery()
}

#[tauri::command]
fn retry_dictation(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.recording.load(Ordering::SeqCst) || state.processing.swap(true, Ordering::SeqCst) {
        return Err("Dictation is busy".into());
    }
    let retry = state.sessions.lock().unwrap().retry_audio();
    let Some((id, samples)) = retry else {
        state.processing.store(false, Ordering::SeqCst);
        return Err("No unexpired, unattempted recording to retry".into());
    };
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let cfg = state
            .sessions
            .lock()
            .unwrap()
            .get(id)
            .map(|j| j.cfg.clone())
            .unwrap_or_default();
        let result = state
            .engine
            .lock()
            .unwrap()
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

fn register_recovery_shortcuts(app: &AppHandle, cfg: &config::Config) -> Result<(), String> {
    for (kind, key) in [
        (0, &cfg.cancel_shortcut),
        (1, &cfg.retry_shortcut),
        (2, &cfg.recovery_shortcut),
    ] {
        let Some(key) = key else {
            continue;
        };
        let shortcut = key.parse::<Shortcut>().map_err(|e| e.to_string())?;
        app.global_shortcut()
            .on_shortcut(shortcut, move |app, _, event| {
                if event.state() != ShortcutState::Pressed {
                    return;
                }
                match kind {
                    0 => cancel_dictation(app.clone()),
                    1 => {
                        if let Err(e) = retry_dictation(app.clone()) {
                            log::warn!("{e}");
                        }
                    }
                    _ => {
                        if let Some(text) = recover_dictation(app.clone()) {
                            let _ = app.run_on_main_thread(move || {
                                use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
                                use objc2_foundation::NSString;
                                let pb = NSPasteboard::generalPasteboard();
                                pb.clearContents();
                                unsafe {
                                    pb.setString_forType(
                                        &NSString::from_str(&text),
                                        NSPasteboardTypeString,
                                    );
                                }
                            });
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Register exactly the given set of push-to-talk shortcuts, persist them, and
/// refresh the UI. "Fn" drives the low-level Fn/Globe listener; every other
/// entry is registered as a normal global shortcut. All accelerators are
/// validated before anything is unregistered, so a bad entry leaves the
/// previous set intact.
fn apply_shortcuts(app: &AppHandle, list: Vec<String>) -> Result<(), String> {
    // Validate recovery bindings and collisions before replacing live shortcuts.
    let cfg = config::Config::load();
    let mut seen = Vec::new();
    for key in list.iter().chain(
        [
            &cfg.cancel_shortcut,
            &cfg.retry_shortcut,
            &cfg.recovery_shortcut,
        ]
        .into_iter()
        .filter_map(|s| s.as_ref()),
    ) {
        if bare_modifier_flag(key).is_some() {
            continue;
        }
        let parsed = key.parse::<Shortcut>().map_err(|e| e.to_string())?;
        if seen.contains(&parsed) {
            return Err("Shortcut bindings must be distinct".into());
        }
        seen.push(parsed);
    }
    // Validate every accelerator up front.
    for accel in &list {
        if bare_modifier_flag(accel).is_none() {
            accel
                .parse::<Shortcut>()
                .map_err(|e| format!("\"{accel}\" isn't a usable shortcut: {e}"))?;
        }
    }

    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let mut watched: u64 = 0;
    for accel in &list {
        if let Some(flag) = bare_modifier_flag(accel) {
            watched |= flag;
            continue;
        }
        if let Ok(sc) = accel.parse::<Shortcut>() {
            if let Err(e) = gs.on_shortcut(sc, |app, _sc, event| on_shortcut(app, event.state())) {
                log::error!("couldn't register \"{accel}\": {e}");
            }
        }
    }

    register_recovery_shortcuts(app, &config::Config::load())?;
    let state = app.state::<AppState>();
    state.watched_flags.store(watched, Ordering::SeqCst);
    *state.shortcuts.lock().unwrap() = list.clone();
    let mut cfg = config::Config::load();
    cfg.shortcuts = list.clone();
    cfg.save();
    log::info!("shortcuts: {}", list.join(", "));
    refresh_menu(app);
    Ok(())
}

/// Add one push-to-talk shortcut (up to MAX_SHORTCUTS), rejecting duplicates.
fn add_shortcut(app: &AppHandle, accel: &str) -> Result<(), String> {
    let mut list = app.state::<AppState>().shortcuts.lock().unwrap().clone();
    if list.iter().any(|a| a == accel) {
        return Err("That shortcut is already added".into());
    }
    if list.len() >= MAX_SHORTCUTS {
        return Err(format!("You can have at most {MAX_SHORTCUTS} shortcuts"));
    }
    list.push(accel.to_string());
    apply_shortcuts(app, list)
}

/// Human-friendly label for a shortcut accelerator (e.g. the Fn/Globe key).
fn hotkey_label(accel: &str) -> String {
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
fn change_shortcut(app: &AppHandle, old: &str, new: &str) -> Result<(), String> {
    if old == new {
        return Ok(());
    }
    let mut list = app.state::<AppState>().shortcuts.lock().unwrap().clone();
    if list.iter().any(|a| a == new) {
        return Err("That shortcut is already added".into());
    }
    match list.iter().position(|a| a == old) {
        Some(pos) => list[pos] = new.to_string(),
        None => {
            // Old entry gone (e.g. config changed underneath) — add instead.
            if list.len() >= MAX_SHORTCUTS {
                return Err(format!("You can have at most {MAX_SHORTCUTS} shortcuts"));
            }
            list.push(new.to_string());
        }
    }
    apply_shortcuts(app, list)
}

/// Remove a push-to-talk shortcut, keeping at least one active.
fn remove_shortcut(app: &AppHandle, accel: &str) {
    let mut list = app.state::<AppState>().shortcuts.lock().unwrap().clone();
    list.retain(|a| a != accel);
    if list.is_empty() {
        log::info!("refusing to remove the last shortcut");
        return;
    }
    if let Err(e) = apply_shortcuts(app, list) {
        log::error!("remove shortcut: {e}");
    }
}

/// Watch bare-modifier hotkeys (Fn/Globe, Ctrl, Cmd, Alt, Shift) globally —
/// none of them can be registered shortcuts. A minimal CGEventTap on
/// flagsChanged events only — no keyboard-layout lookups, so it's safe off
/// the main thread. Always running; only acts on flags in `watched_flags`.
fn spawn_fn_listener(app: AppHandle) {
    use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
    use core_graphics::event::{
        CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    };
    use std::cell::Cell;

    std::thread::spawn(move || {
        let prev_flags = Cell::new(0u64);
        let tap = CGEventTap::new(
            CGEventTapLocation::Session,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            vec![CGEventType::FlagsChanged],
            move |_proxy, _etype, event| {
                let now = event.get_flags().bits();
                let before = prev_flags.get();
                prev_flags.set(now);
                let watched = app.state::<AppState>().watched_flags.load(Ordering::SeqCst);
                let changed = (before ^ now) & watched;
                if changed != 0 {
                    // A watched modifier toggled. Pressed if any watched bit
                    // is newly down; released when it comes back up.
                    let shortcut_state = if now & changed != 0 {
                        ShortcutState::Pressed
                    } else {
                        ShortcutState::Released
                    };
                    on_shortcut(&app, shortcut_state);
                }
                None
            },
        );
        let Ok(tap) = tap else {
            log::error!("couldn't create Fn event tap (check Input Monitoring permission)");
            return;
        };
        let Ok(source) = tap.mach_port.create_runloop_source(0) else {
            log::error!("couldn't create run loop source for the Fn tap");
            return;
        };
        let run_loop = CFRunLoop::get_current();
        unsafe {
            run_loop.add_source(&source, kCFRunLoopCommonModes);
        }
        tap.enable();
        CFRunLoop::run_current();
    });
}

/// Menu action: switch the Whisper model. Downloads it if needed and swaps
/// the engine in the background; on failure the previous model stays active.
fn set_model(app: &AppHandle, name: &str) {
    // Reject anything that isn't an absolute path to a local model file or a
    // valid ggml model name, before it's persisted or used to build a URL/path.
    if !name.starts_with('/') && !model::is_valid_model_name(name) {
        log::error!("ignoring invalid model name: {name:?}");
        return;
    }
    let state = app.state::<AppState>();
    let (previous, generation) = {
        let mut current = state.current_model.lock().unwrap();
        // Skip only if this model is already active and loaded; otherwise
        // (re)load so a missing/never-downloaded model still gets fetched.
        if *current == name && state.engine_ready.load(Ordering::SeqCst) {
            return;
        }
        let previous = state.active_model.lock().unwrap().clone();
        *current = name.to_string();
        (
            previous,
            state.model_generation.fetch_add(1, Ordering::SeqCst) + 1,
        )
    };
    let mut cfg = config::Config::load();
    cfg.model = name.to_string();
    cfg.save();
    refresh_menu(app);

    let app = app.clone();
    let name = name.to_string();
    std::thread::spawn(move || {
        set_tray_state(&app, TrayState::Downloading);
        let loaded = model::ensure_model(&name)
            .and_then(|path| engine::Engine::load(&path.to_string_lossy(), &cfg));
        let state = app.state::<AppState>();
        let mut engine_slot = state.engine.lock().unwrap();
        let mut current = state.current_model.lock().unwrap();
        if state.model_generation.load(Ordering::SeqCst) != generation {
            return;
        }
        match loaded {
            Ok(engine) => {
                *engine_slot = Some(engine);
                *state.active_model.lock().unwrap() = cfg.model.clone();
                *state.cfg.lock().unwrap() = cfg.clone();
                state.engine_ready.store(true, Ordering::SeqCst);
                log::info!("switched model to {name}");
            }
            Err(e) => {
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
/// On startup, ask macOS for Accessibility + Microphone so Gretchen Flow shows
/// up in those Privacy lists (to be toggled on) instead of only appearing after
/// the first dictation. Input Monitoring already self-registers via the Fn tap.
fn request_startup_permissions(_app: &AppHandle) {
    prompt_accessibility();
    request_microphone();
}

/// Ask macOS for microphone access via the canonical API. This registers the
/// app under Privacy ▸ Microphone and shows the prompt (when "not determined"),
/// and is a no-op once already granted/denied.
fn request_microphone() {
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
fn prompt_accessibility() {
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

fn open_hotkey_window(app: &AppHandle) {
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
    .title("Set Hotkey — Gretchen Flow")
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
fn activate_app() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    if let Some(mtm) = MainThreadMarker::new() {
        let ns_app = NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        ns_app.activateIgnoringOtherApps(true);
    }
}

/// Set the Dock icon to the Gretchen artwork at runtime, so it's never the
/// generic executable icon (e.g. in dev builds, or before the bundle icon
/// is associated). Main thread only.
fn set_dock_icon() {
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
fn pick_model_file(app: &AppHandle) {
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

// ---------------------------------------------------------------------------
// Standalone NES-style window mirroring the tray menu
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
struct MenuChoice {
    id: String,
    label: String,
    active: bool,
    note: String,
}

#[derive(serde::Serialize)]
struct MenuState {
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
fn menu_state(app: AppHandle) -> MenuState {
    let state = app.state::<AppState>();
    let current_model = state.current_model.lock().unwrap().clone();
    let shortcuts = state.shortcuts.lock().unwrap().clone();
    let hotkey_mode = state.cfg.lock().unwrap().hotkey_mode.clone();
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
fn menu_choose_model(app: AppHandle, name: String) {
    set_model(&app, &name);
}

#[tauri::command]
fn menu_model_from_file(app: AppHandle) {
    pick_model_file(&app);
}

/// Open the Hugging Face model list in the default browser so the user can
/// download any ggml model, then load it via "from file…".
#[tauri::command]
fn open_models_page() {
    open_url(MODELS_URL);
}

/// Open a URL in the user's default browser.
fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    if let Err(e) = std::process::Command::new("open").arg(url).spawn() {
        log::error!("couldn't open {url}: {e}");
    }
}

/// Open a specific macOS Privacy & Security pane so the user can grant a
/// permission Gretchen Flow needs.
#[tauri::command]
fn open_privacy(pane: String) {
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
fn prime_microphone(app: AppHandle) {
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        state.recorder.start();
        std::thread::sleep(std::time::Duration::from_millis(250));
        let _ = state.recorder.stop();
    });
}

#[tauri::command]
fn menu_remove_hotkey(app: AppHandle, accel: String) {
    remove_shortcut(&app, &accel);
}

/// Open the recorder to ADD a new shortcut.
#[tauri::command]
fn menu_record_hotkey(app: AppHandle) {
    *app.state::<AppState>().pending_replace.lock().unwrap() = None;
    open_hotkey_window(&app);
}

/// Open the recorder to CHANGE an existing shortcut to anything else.
#[tauri::command]
fn menu_change_hotkey(app: AppHandle, accel: String) {
    *app.state::<AppState>().pending_replace.lock().unwrap() = Some(accel);
    open_hotkey_window(&app);
}

/// The shortcut the recorder is currently set to replace (None = add mode).
#[tauri::command]
fn hotkey_replace_target(app: AppHandle) -> Option<String> {
    app.state::<AppState>()
        .pending_replace
        .lock()
        .unwrap()
        .clone()
}

#[tauri::command]
fn menu_reload_config(app: AppHandle) {
    reload_config(&app);
}

#[tauri::command]
fn menu_open_setup(app: AppHandle) {
    open_setup_window(&app);
}

/// Copy a recent transcription to the system clipboard.
#[tauri::command]
fn menu_copy_recent(index: usize) {
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
fn menu_clear_history(app: AppHandle) {
    history::clear();
    refresh_menu(&app);
}

#[tauri::command]
fn menu_quit(app: AppHandle) {
    app.exit(0);
}

/// Open (or focus) the standalone NES-style main window. Stays a menu-bar
/// (Accessory) app — no Dock icon — and surfaces the window explicitly with
/// show()/set_focus() after centering it.
fn open_main_window(app: &AppHandle) {
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
            .title("Gretchen Flow")
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

#[tauri::command]
fn menu_close(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.close();
    }
}

/// Show the first-run / setup guide on the main thread.
fn show_setup(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || open_setup_window(&handle));
}

/// Open (or focus) the setup window explaining how to download a model and
/// grant permissions.
fn open_setup_window(app: &AppHandle) {
    activate_app();
    if let Some(window) = app.get_webview_window("setup") {
        let _ = window.set_focus();
        return;
    }
    let result =
        tauri::WebviewWindowBuilder::new(app, "setup", tauri::WebviewUrl::App("setup.html".into()))
            .title("Welcome to Gretchen Flow")
            .inner_size(520.0, 580.0)
            .resizable(false)
            .center()
            .build();
    if let Err(e) = result {
        log::error!("couldn't open setup window: {e}");
    }
}

#[tauri::command]
fn download_recommended_model(app: AppHandle) {
    set_model(&app, RECOMMENDED_MODEL);
    if let Some(window) = app.get_webview_window("setup") {
        let _ = window.close();
    }
}

#[tauri::command]
fn close_setup(app: AppHandle) {
    if let Some(window) = app.get_webview_window("setup") {
        let _ = window.close();
    }
}

#[tauri::command]
fn apply_custom_hotkey(app: AppHandle, accel: String) -> Result<(), String> {
    let target = app
        .state::<AppState>()
        .pending_replace
        .lock()
        .unwrap()
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
fn remove_pending_hotkey(app: AppHandle) {
    let target = app
        .state::<AppState>()
        .pending_replace
        .lock()
        .unwrap()
        .take();
    if let Some(accel) = target {
        remove_shortcut(&app, &accel);
    }
    if let Some(window) = app.get_webview_window("hotkey") {
        let _ = window.close();
    }
}

#[tauri::command]
fn cancel_custom_hotkey(app: AppHandle) {
    *app.state::<AppState>().pending_replace.lock().unwrap() = None;
    if let Some(window) = app.get_webview_window("hotkey") {
        let _ = window.close();
    }
}

fn on_shortcut(app: &AppHandle, state_event: ShortcutState) {
    let state = app.state::<AppState>();
    let hold_mode = state.cfg.lock().unwrap().hotkey_mode == "hold";
    let recording = state.recording.load(Ordering::SeqCst);

    match state_event {
        ShortcutState::Pressed => {
            if hold_mode {
                if !recording {
                    start_recording(app);
                }
            } else if recording {
                stop_and_transcribe(app);
            } else {
                start_recording(app);
            }
        }
        ShortcutState::Released => {
            if hold_mode && recording {
                stop_and_transcribe(app);
            }
        }
    }
}

const HISTORY_MENU_ITEMS: usize = 5;

/// Handles to the menu items that change at runtime. The menu is built once
/// and mutated in place — replacing a tray menu (or its submenus) while macOS
/// has shown it once makes the dropdown glitch on the next hover.
struct MenuHandles {
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
fn build_menu(app: &AppHandle) -> tauri::Result<MenuHandles> {
    let menu = Menu::new(app)?;

    let status = MenuItem::with_id(app, "status", "Gretchen Flow", false, None::<&str>)?;
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
        "Getting Started…",
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
        "Quit Gretchen Flow",
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
fn refresh_menu(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || refresh_menu_on_main(&handle));
}

fn refresh_menu_on_main(app: &AppHandle) {
    let state = app.state::<AppState>();
    let recent = history::recent(HISTORY_MENU_ITEMS);
    let shortcuts = state.shortcuts.lock().unwrap().clone();
    let current_model = state.current_model.lock().unwrap().clone();
    let hotkey_mode = state.cfg.lock().unwrap().hotkey_mode.clone();
    let has_engine = state.engine_ready.load(Ordering::SeqCst);

    MENU_HANDLES.with(|handles| {
        let handles = handles.borrow();
        let Some(h) = handles.as_ref() else { return };

        let _ = h.status.set_text(if has_engine {
            format!("Gretchen Flow — {} ({hotkey_mode})", shortcuts.join(" / "))
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

    *state.history_items.lock().unwrap() = recent;

    // Keep the standalone window in sync if it's open.
    use tauri::Emitter;
    let _ = app.emit("menu-updated", ());
}

fn on_menu_event(app: &AppHandle, id: &str) {
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
            .lock()
            .unwrap()
            .get(idx)
            .cloned();
        if let Some(accel) = accel {
            *app.state::<AppState>().pending_replace.lock().unwrap() = Some(accel);
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
        let items = state.history_items.lock().unwrap();
        items.get(idx).cloned()
    };
    if let Some(text) = text {
        std::thread::spawn(move || {
            // Give the menu a beat to close and focus to return to the app
            // the user was in.
            std::thread::sleep(std::time::Duration::from_millis(300));
            if let Err(e) = inject::type_text(&text) {
                log::error!("{e}");
            }
        });
    }
}

/// Load the configured model on startup IF it's already on disk. The app
/// ships with no model and never auto-downloads — a missing model opens the
/// setup guide instead.
fn load_engine_async(app: AppHandle) {
    let generation = {
        let state = app.state::<AppState>();
        let _current = state.current_model.lock().unwrap();
        state.model_generation.fetch_add(1, Ordering::SeqCst) + 1
    };
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let cfg = state.cfg.lock().unwrap().clone();

        let available = if cfg.model.is_empty() {
            false
        } else if cfg.model.starts_with('/') {
            std::path::Path::new(&cfg.model).exists()
        } else {
            model::model_path(&cfg.model).exists()
        };
        if !available {
            log::info!("no model downloaded yet; opening setup");
            set_tray_state(&app, TrayState::NeedsModel);
            show_setup(&app);
            return;
        }

        set_tray_state(&app, TrayState::Downloading);
        let loaded = model::ensure_model(&cfg.model)
            .and_then(|path| engine::Engine::load(&path.to_string_lossy(), &cfg));
        let mut engine_slot = state.engine.lock().unwrap();
        let current = state.current_model.lock().unwrap();
        if state.model_generation.load(Ordering::SeqCst) != generation {
            return;
        }
        match loaded {
            Ok(engine) => {
                *engine_slot = Some(engine);
                *state.active_model.lock().unwrap() = cfg.model.clone();
                *state.cfg.lock().unwrap() = cfg.clone();
                state.engine_ready.store(true, Ordering::SeqCst);
                log::info!("engine ready (model: {})", cfg.model);
                set_tray_state(&app, TrayState::Idle);
            }
            Err(e) => {
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
fn reload_engine(app: AppHandle, cfg: config::Config, generation: u64) {
    let model = cfg.model.clone();
    std::thread::spawn(move || {
        set_tray_state(&app, TrayState::Downloading);
        let loaded = model::ensure_model(&model)
            .and_then(|path| engine::Engine::load(&path.to_string_lossy(), &cfg));
        let state = app.state::<AppState>();
        let mut engine_slot = state.engine.lock().unwrap();
        let current = state.current_model.lock().unwrap();
        if state.model_generation.load(Ordering::SeqCst) != generation {
            return;
        }
        match loaded {
            Ok(engine) => {
                *engine_slot = Some(engine);
                *state.active_model.lock().unwrap() = cfg.model.clone();
                *state.cfg.lock().unwrap() = cfg.clone();
                state.engine_ready.store(true, Ordering::SeqCst);
                log::info!("engine reloaded (model: {model})");
            }
            Err(e) => log::error!("engine reload failed, keeping current engine: {e}"),
        }
        drop(current);
        drop(engine_slot);
        set_tray_state(&app, TrayState::Idle);
    });
}

/// Menu action: re-read config.json and apply everything live — hotkey,
/// icon theme, model, and transcription settings — without restarting.
fn reload_config(app: &AppHandle) {
    let fresh = config::Config::load();
    history::configure(fresh.history.clone());
    let state = app.state::<AppState>();

    // Re-register the shortcuts if they changed.
    let current = state.shortcuts.lock().unwrap().clone();
    let old = state.cfg.lock().unwrap().clone();
    if fresh.shortcuts != current
        || fresh.cancel_shortcut != old.cancel_shortcut
        || fresh.retry_shortcut != old.retry_shortcut
        || fresh.recovery_shortcut != old.recovery_shortcut
    {
        if let Err(e) = apply_shortcuts(app, fresh.shortcuts.clone()) {
            log::error!("reload config: {e}");
        }
    }

    // Swap in the new config (covers hotkey_mode and any future fields).
    let model_changed = fresh.model != *state.active_model.lock().unwrap();
    let model = fresh.model.clone();
    *state.cfg.lock().unwrap() = fresh.clone();
    let generation = {
        let mut current = state.current_model.lock().unwrap();
        *current = model;
        state.model_generation.fetch_add(1, Ordering::SeqCst) + 1
    };

    refresh_menu(app);
    log::info!("config reloaded");

    // Always reload the engine so language/vocabulary/punctuation edits apply
    // even when the model name is unchanged.
    if model_changed {
        reload_engine(app.clone(), fresh, generation);
    }
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

    // Parse the registrable shortcuts (everything except "Fn", which the
    // low-level listener handles). Unparseable entries are dropped with a warning.
    let startup_shortcuts: Vec<Shortcut> = cfg
        .shortcuts
        .iter()
        .filter(|a| bare_modifier_flag(a).is_none())
        .filter_map(|a| match a.parse::<Shortcut>() {
            Ok(sc) => Some(sc),
            Err(e) => {
                log::error!("ignoring invalid shortcut \"{a}\": {e}");
                None
            }
        })
        .collect();
    let startup_watched: u64 = cfg
        .shortcuts
        .iter()
        .filter_map(|a| bare_modifier_flag(a))
        .fold(0, |acc, f| acc | f);

    // Surface panics in the log — an unwind across a system callback aborts
    // the process with no crash report otherwise.
    std::panic::set_hook(Box::new(|info| {
        log::error!("PANIC: {info}");
    }));

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            apply_custom_hotkey,
            cancel_custom_hotkey,
            open_models_page,
            open_privacy,
            prime_microphone,
            download_recommended_model,
            close_setup,
            menu_state,
            menu_choose_model,
            menu_model_from_file,
            menu_remove_hotkey,
            menu_record_hotkey,
            menu_change_hotkey,
            hotkey_replace_target,
            remove_pending_hotkey,
            cancel_dictation,
            recover_dictation,
            retry_dictation,
            menu_reload_config,
            menu_open_setup,
            menu_copy_recent,
            menu_clear_history,
            menu_quit,
            menu_close
        ])
        .manage(AppState {
            recorder: audio::Recorder::spawn(),
            sessions: Mutex::new(session::Sessions::default()),
            engine: Arc::new(Mutex::new(None)),
            recording: AtomicBool::new(false),
            engine_ready: AtomicBool::new(false),
            model_generation: AtomicU64::new(0),
            processing: AtomicBool::new(false),
            shortcuts: Mutex::new(cfg.shortcuts.clone()),
            watched_flags: AtomicU64::new(startup_watched),
            pending_replace: Mutex::new(None),
            current_model: Mutex::new(cfg.model.clone()),
            active_model: Mutex::new(cfg.model.clone()),
            cfg: Mutex::new(cfg),
            history_items: Mutex::new(Vec::new()),
        })
        .setup(move |app| {
            // Regular app: appears in the Dock (clicking it opens the window),
            // and also keeps a menu-bar tray icon for recording status.
            #[cfg(target_os = "macos")]
            {
                app.set_activation_policy(tauri::ActivationPolicy::Regular);
                set_dock_icon();
            }

            let expiry_app = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(5));
                expiry_app
                    .state::<AppState>()
                    .sessions
                    .lock()
                    .unwrap()
                    .expire();
            });
            let handles = build_menu(app.handle())?;
            TrayIconBuilder::with_id(TRAY_ID)
                .icon(Image::from_bytes(ICON_IDLE)?)
                .icon_as_template(false)
                .menu(&handles.menu)
                .on_menu_event(|app, event| on_menu_event(app, event.id().as_ref()))
                .build(app)?;
            MENU_HANDLES.with(|cell| *cell.borrow_mut() = Some(handles));
            refresh_menu(app.handle());

            for shortcut in &startup_shortcuts {
                app.global_shortcut()
                    .on_shortcut(*shortcut, |app, _sc, event| on_shortcut(app, event.state()))?;
            }
            register_recovery_shortcuts(
                app.handle(),
                &app.state::<AppState>().cfg.lock().unwrap(),
            )?;
            spawn_fn_listener(app.handle().clone());

            // Proactively ask for Accessibility + Microphone so the app shows up
            // in those Privacy lists for the user to toggle on, rather than only
            // appearing after first use.
            request_startup_permissions(app.handle());

            load_engine_async(app.handle().clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Gretchen Flow")
        .run(|app, event| match event {
            // Keep running with zero windows; only exit via the Quit menu item.
            tauri::RunEvent::ExitRequested { code, api, .. } => {
                log::info!("exit requested (code: {code:?})");
                if code.is_none() {
                    api.prevent_exit();
                }
            }
            // Clicking the app icon (Finder/Dock/Launchpad) while it's already
            // running re-opens the standalone window.
            tauri::RunEvent::Reopen { .. } => open_main_window(app),
            tauri::RunEvent::Exit => log::info!("exiting"),
            _ => {}
        });
}
