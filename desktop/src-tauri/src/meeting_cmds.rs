//! Tauri commands and window for meeting notes. Kept separate from the
//! dictation command surface; see `meeting.rs` for the capture pipeline.
use crate::locks::MutexExt;
use crate::meeting::{MeetingListItem, MeetingRecord, StateView, Store, Transcriber};
use crate::tray::{set_tray_state, TrayState};
use crate::{meeting_notes, AppState};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

pub(crate) const WINDOW: &str = "meetings";

#[tauri::command]
pub(crate) fn meeting_state(app: AppHandle) -> StateView {
    app.state::<AppState>().meeting.view()
}

/// Start a microphone meeting. Refused while dictation is recording or
/// processing; dictation is refused while a meeting is active.
#[tauri::command(async)]
pub(crate) fn meeting_start(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    if !state.engine_ready.load(Ordering::SeqCst) {
        return Err("No speech model is loaded yet. Finish setup first.".into());
    }
    {
        // Same lock dictation holds while it claims the microphone.
        let _sessions = state.sessions.lock_repair(|sessions| sessions.cancel());
        if state.recording.load(Ordering::SeqCst) || state.processing.load(Ordering::SeqCst) {
            return Err("Dictation is in progress. Try again when it finishes.".into());
        }
        state.meeting.reserve()?;
    }
    let mut cfg = state.cfg.lock_recover().for_app(None);
    cfg.profile.mode = crate::profile::Mode::Literal;
    let engine = Arc::clone(&state.engine);
    let transcribe: Transcriber = Box::new(move |samples: &[f32]| {
        engine
            .lock_recover()
            .as_ref()
            .ok_or_else(|| "model not loaded".to_string())?
            .transcribe(samples, &cfg)
    });
    state.meeting.begin(
        |input| state.recorder.start_meeting(input),
        transcribe,
        Arc::new(Store::default_store()),
    )?;
    set_tray_state(&app, TrayState::Recording);
    Ok(())
}

#[derive(serde::Serialize)]
pub(crate) struct Stopped {
    id: String,
}

/// Stop capture, recognize the remaining audio, and save the meeting.
#[tauri::command(async)]
pub(crate) fn meeting_stop(app: AppHandle) -> Result<Stopped, String> {
    let record = finish(&app)?;
    Ok(Stopped { id: record.id })
}

fn finish(app: &AppHandle) -> Result<MeetingRecord, String> {
    let state = app.state::<AppState>();
    set_tray_state(app, TrayState::Transcribing);
    let result = state.meeting.finish(
        || {
            // Dropping the input stream drops the meeting sink and ends capture.
            let _ = state.recorder.stop_async();
        },
        &Store::default_store(),
    );
    set_tray_state(app, TrayState::Idle);
    result
}

/// Save an in-progress meeting before the process exits.
pub(crate) fn finish_on_exit(app: &AppHandle) {
    if app.state::<AppState>().meeting.is_active() {
        match finish(app) {
            Ok(record) => log::info!("meeting saved on exit ({} segments)", record.segments.len()),
            Err(e) => log::warn!("meeting not saved on exit: {e}"),
        }
    }
}

#[tauri::command]
pub(crate) fn meeting_list() -> Vec<MeetingListItem> {
    Store::default_store().list()
}

#[tauri::command]
pub(crate) fn meeting_get(id: String) -> Result<Option<MeetingRecord>, String> {
    Store::default_store().get(&id)
}

#[tauri::command]
pub(crate) fn meeting_delete(app: AppHandle, id: String) -> Result<(), String> {
    if app.state::<AppState>().meeting.view().id.as_deref() == Some(id.as_str()) {
        return Err("Stop the meeting before deleting it.".into());
    }
    Store::default_store().delete(&id)
}

/// Copy a saved meeting's notes (Markdown) to the clipboard. Runs on the
/// main thread like other pasteboard writes.
#[tauri::command]
pub(crate) fn meeting_copy(id: String) -> Result<(), String> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    use objc2_foundation::NSString;
    let record = Store::default_store()
        .get(&id)?
        .ok_or("Meeting not found")?;
    let text = meeting_notes::markdown(&record);
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    let ok =
        unsafe { pasteboard.setString_forType(&NSString::from_str(&text), NSPasteboardTypeString) };
    if ok {
        Ok(())
    } else {
        Err("Couldn't copy to the clipboard".into())
    }
}

#[tauri::command]
pub(crate) fn meeting_open_window(app: AppHandle) {
    open_window(&app);
}

pub(crate) fn open_window(app: &AppHandle) {
    crate::setup::activate_app();
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let result = tauri::WebviewWindowBuilder::new(
        app,
        WINDOW,
        tauri::WebviewUrl::App("meeting.html".into()),
    )
    .title("Meeting Notes — Greta Flow")
    .inner_size(520.0, 680.0)
    .min_inner_size(380.0, 480.0)
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
        Err(e) => log::error!("couldn't open meeting window: {e}"),
    }
}
