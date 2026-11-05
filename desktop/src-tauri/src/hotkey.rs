//! Native shortcut registration and bare-modifier dispatch.
use crate::commands::{cancel_dictation, recover_dictation, retry_dictation};
use crate::dictation::{start_recording, stop_and_transcribe};
use crate::locks::MutexExt;
use crate::tray::refresh_menu;
use crate::{bindings, config, AppState};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// Bare modifier keys usable as push-to-talk hotkeys. Like Fn, macOS can't
/// register a lone modifier as a global shortcut, so these are watched by the
/// same low-level flagsChanged listener. Returns the CGEventFlags bit.
pub(crate) fn bare_modifier_flag(accel: &str) -> Option<u64> {
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
pub(crate) const MAX_SHORTCUTS: usize = 3;

pub(crate) fn register_binding(
    app: &AppHandle,
    shortcut: Shortcut,
    dispatch: bindings::Dispatch,
) -> Result<(), String> {
    app.global_shortcut()
        .on_shortcut(shortcut, move |app, shortcut, event| {
            let action = dispatch.action(shortcut);
            if action == Some(bindings::Action::Dictate) {
                on_shortcut(app, event.state());
                return;
            }
            if event.state() != ShortcutState::Pressed {
                return;
            }
            match action {
                Some(bindings::Action::Cancel) => cancel_dictation(app.clone()),
                Some(bindings::Action::Retry) => {
                    if let Err(e) = retry_dictation(app.clone()) {
                        log::warn!("{e}");
                    }
                }
                Some(bindings::Action::Recover) => {
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
                _ => {}
            }
        })
        .map_err(|e| e.to_string())
}

/// Parse a single configuration snapshot before touching any registrations.
/// Native registration failures preserve the working dispatch table and flags.
pub(crate) fn install_shortcuts(app: &AppHandle, cfg: &config::Config) -> Result<(), String> {
    let plan = bindings::Plan::validate(cfg)?;
    let state = app.state::<AppState>();
    let mut bindings = state.bindings.lock_recover();
    let dispatch = bindings.dispatch.clone();
    bindings.apply(
        &plan,
        |shortcut| register_binding(app, shortcut, dispatch.clone()),
        |shortcut| {
            app.global_shortcut()
                .unregister(shortcut)
                .map_err(|e| e.to_string())
        },
    )?;
    drop(bindings);
    state.watched_flags.store(plan.watched, Ordering::SeqCst);
    *state.shortcuts.lock_recover() = cfg.shortcuts.clone();
    let mut current = state.cfg.lock_recover();
    current.shortcuts = cfg.shortcuts.clone();
    current.cancel_shortcut = cfg.cancel_shortcut.clone();
    current.retry_shortcut = cfg.retry_shortcut.clone();
    current.recovery_shortcut = cfg.recovery_shortcut.clone();
    Ok(())
}

/// Register and persist a new dictation-key list only after every new binding
/// succeeds. Fn/bare modifiers remain handled by the existing flags listener.
pub(crate) fn apply_shortcuts(app: &AppHandle, list: Vec<String>) -> Result<(), String> {
    let mut cfg = config::Config::load();
    cfg.shortcuts = list;
    install_shortcuts(app, &cfg)?;
    cfg.save();
    log::info!("shortcuts: {}", cfg.shortcuts.join(", "));
    refresh_menu(app);
    Ok(())
}

/// Add one push-to-talk shortcut (up to MAX_SHORTCUTS), rejecting duplicates.
pub(crate) fn add_shortcut(app: &AppHandle, accel: &str) -> Result<(), String> {
    let mut list = app.state::<AppState>().shortcuts.lock_recover().clone();
    if list.iter().any(|a| a == accel) {
        return Err("That shortcut is already added".into());
    }
    if list.len() >= MAX_SHORTCUTS {
        return Err(format!("You can have at most {MAX_SHORTCUTS} shortcuts"));
    }
    list.push(accel.to_string());
    apply_shortcuts(app, list)
}

pub(crate) fn change_shortcut(app: &AppHandle, old: &str, new: &str) -> Result<(), String> {
    if old == new {
        return Ok(());
    }
    let mut list = app.state::<AppState>().shortcuts.lock_recover().clone();
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
pub(crate) fn remove_shortcut(app: &AppHandle, accel: &str) {
    let mut list = app.state::<AppState>().shortcuts.lock_recover().clone();
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
pub(crate) fn spawn_fn_listener(app: AppHandle) {
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
pub(crate) fn on_shortcut(app: &AppHandle, state_event: ShortcutState) {
    let state = app.state::<AppState>();
    let hold_mode = state.cfg.lock_recover().hotkey_mode == "hold";
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
