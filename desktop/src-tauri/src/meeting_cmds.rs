//! Tauri commands and window for meeting notes. Kept separate from the
//! dictation command surface; see `meeting.rs` for the capture pipeline.
use crate::locks::MutexExt;
use crate::meeting::{MeetingListItem, MeetingRecord, SourceMode, StateView, Store, Transcriber};
use crate::tray::{set_tray_state, TrayState};
use crate::{meeting_notes, system_audio, AppState};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

pub(crate) const WINDOW: &str = "meetings";

#[tauri::command]
pub(crate) fn meeting_state(app: AppHandle) -> StateView {
    app.state::<AppState>().meeting.view()
}

/// Start a meeting capturing `source`: "microphone" (default), "system", or
/// "both". Refused while dictation is recording or processing; dictation is
/// refused while a meeting is active.
#[tauri::command(async)]
pub(crate) fn meeting_start(app: AppHandle, source: Option<String>) -> Result<(), String> {
    let mode = SourceMode::parse(source.as_deref())?;
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
    let transcriber = || -> Transcriber {
        let engine = Arc::clone(&state.engine);
        let cfg = cfg.clone();
        Box::new(move |samples: &[f32]| {
            engine
                .lock_recover()
                .as_ref()
                .ok_or_else(|| "model not loaded".to_string())?
                .transcribe(samples, &cfg)
        })
    };
    let open_system = |input| {
        let helper = system_audio::helper_path()?;
        system_audio::Capture::spawn(
            std::process::Command::new(helper),
            input,
            system_audio::HEADER_TIMEOUT,
        )
    };
    state.meeting.begin_sources(
        mode,
        Box::new(|input| state.recorder.start_meeting(input)),
        Box::new(open_system),
        transcriber,
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

/// Summarize a saved meeting with the local model. The result is saved into
/// the meeting record only after grounding checks; failures are saved with
/// their real reason and the transcript is untouched.
#[tauri::command]
pub(crate) async fn meeting_summarize(
    app: AppHandle,
    id: String,
) -> Result<meeting_notes::MeetingSummary, String> {
    tauri::async_runtime::spawn_blocking(move || summarize_blocking(&app, &id))
        .await
        .map_err(|e| e.to_string())?
}

fn summarize_blocking(app: &AppHandle, id: &str) -> Result<meeting_notes::MeetingSummary, String> {
    use crate::summary::{self, SummaryState};
    let state = app.state::<AppState>();
    if state.meeting.view().id.as_deref() == Some(id) {
        return Err("Stop the meeting before summarizing it.".into());
    }
    let cfg = state.cfg.lock_recover().clone();
    let status = summary::runtime_status(&cfg);
    match status.state {
        SummaryState::Ready | SummaryState::Failed => {}
        SummaryState::Loading => return Err("A summary is already being generated.".into()),
        _ => return Err(status.detail),
    }
    let _claim = summary::claim_run(id)?;
    let (Some(path), Some(model)) = (status.path, status.model) else {
        return Err("No summary model is set up.".into());
    };
    let helper = summary::helper_path()?;
    let store = Store::default_store();
    let mut record = store.get(id)?.ok_or("Meeting not found")?;
    record.summary = meeting_notes::SummaryStatus::generating();
    store.save(&record)?;
    let result = summary::summarize(&helper, &path, &record.segments, summary::DEFAULT_TIMEOUT);
    // Re-read so a concurrent rename/delete is respected.
    let Some(mut record) = store.get(id)? else {
        return Err("Meeting was deleted while summarizing.".into());
    };
    match result {
        Ok(raw) => {
            let (bullets, dropped_b) =
                meeting_notes::validate_model_grounding(&record.segments, raw.bullets);
            let bullets = bullets
                .into_iter()
                .map(|b| meeting_notes::SummaryItem { owner: None, ..b })
                .collect();
            let (action_items, dropped_a) =
                meeting_notes::validate_model_grounding(&record.segments, raw.action_items);
            let model_label = if model.pinned {
                summary::manifest().label()
            } else {
                std::path::Path::new(&model.path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or(model.model_id)
            };
            let result = meeting_notes::MeetingSummary {
                bullets,
                action_items,
                model: model_label,
                generated_ms: crate::meeting::now_ms(),
                dropped_ungrounded: dropped_b + dropped_a,
                sections: raw.chunks,
                failed_chunks: raw.failed_chunks,
            };
            record.summary = meeting_notes::SummaryStatus::generated(result.clone());
            store.save(&record)?;
            Ok(result)
        }
        Err(e) => {
            record.summary = meeting_notes::SummaryStatus::failed(format!("Summary failed: {e}"));
            summary::set_run_state(Some((
                SummaryState::Failed,
                format!("Last run failed: {e}"),
            )));
            let _ = store.save(&record);
            Err(e)
        }
    }
}

#[tauri::command]
pub(crate) fn summary_status(app: AppHandle) -> crate::summary::SummaryRuntimeStatus {
    let cfg = app.state::<AppState>().cfg.lock_recover().clone();
    crate::summary::runtime_status(&cfg)
}

fn save_summary_model(app: &AppHandle, path: String) {
    let state = app.state::<AppState>();
    let mut cfg = state.cfg.lock_recover();
    cfg.summary_model = Some(path);
    cfg.save();
    crate::summary::set_run_state(None);
}

#[tauri::command(async)]
pub(crate) fn summary_model_set_path(
    app: AppHandle,
    path: String,
) -> Result<crate::summary::SummaryModelInfo, String> {
    let info = crate::summary::validate_model_dir(std::path::Path::new(path.trim()))?;
    save_summary_model(&app, info.path.clone());
    Ok(info)
}

/// Download the pinned summary model. Only called on an explicit click.
#[tauri::command]
pub(crate) async fn summary_model_download(app: AppHandle) -> Result<(), String> {
    crate::summary::begin_download()?;
    let result = tauri::async_runtime::spawn_blocking(|| {
        crate::summary::download_pinned(&crate::summary::default_model_dir())
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    crate::summary::end_download(&result);
    let path = result?;
    save_summary_model(&app, path.display().to_string());
    Ok(())
}

#[tauri::command]
pub(crate) fn summary_model_download_state() -> crate::summary::DownloadState {
    crate::summary::download_state()
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
