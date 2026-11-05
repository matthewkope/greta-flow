//! Capture, recognition and one-shot delivery of a dictation session.
use crate::history;
use crate::locks::MutexExt;
use crate::setup::show_setup;
use crate::tray::{refresh_menu, set_tray_state, TrayState};
use crate::{audio, inject, metrics, session, streaming, AppState};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};

pub(crate) fn start_recording(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.processing.load(Ordering::SeqCst) || state.recording.load(Ordering::SeqCst) {
        return;
    }
    if !state.engine_ready.load(Ordering::SeqCst) {
        log::warn!("no model loaded; opening setup");
        show_setup(app);
        return;
    }
    let snapshot = state.cfg.lock_recover().clone();
    let (context_tx, context_rx) = std::sync::mpsc::channel();
    let (config_tx, config_rx) = std::sync::mpsc::channel();
    {
        let mut sessions = state.sessions.lock_repair(|sessions| sessions.cancel());
        if state.processing.load(Ordering::SeqCst) || state.recording.swap(true, Ordering::SeqCst) {
            return;
        }
        let id = sessions.begin();
        let Some(job) = sessions.get(id) else {
            state.recording.store(false, Ordering::SeqCst);
            return;
        };
        job.cfg = snapshot.for_app(None);
        job.pending_context = Some(context_rx);
        // Queue microphone start before any potentially slow Accessibility calls.
        if snapshot.streaming {
            let (input, worker) = if snapshot.app_profiles.is_empty() {
                streaming::Worker::spawn(state.engine.clone(), job.cfg.clone())
            } else {
                streaming::Worker::spawn_pending(state.engine.clone(), config_rx)
            };
            job.stream = Some(worker);
            state.recorder.start_stream(input);
        } else {
            state.recorder.start();
        }
    }
    std::thread::spawn(move || {
        let target = inject::Target::capture().ok();
        let bundle = target
            .as_ref()
            .and_then(|t| {
                objc2_app_kit::NSRunningApplication::runningApplicationWithProcessIdentifier(t.pid)
            })
            .and_then(|a| a.bundleIdentifier())
            .map(|s| s.to_string());
        let cfg = snapshot.for_app(bundle.as_deref());
        let context = session::CaptureContext {
            target,
            cfg: cfg.clone(),
            completed: std::time::Instant::now(),
        };
        let _ = context_tx.send(context);
        let _ = config_tx.send(cfg);
    });
    set_tray_state(app, TrayState::Recording);
}

pub(crate) fn stop_and_transcribe(app: &AppHandle) {
    let release = std::time::Instant::now();
    let state = app.state::<AppState>();
    // Serialize start/stop commands with session changes; never wait for the
    // microphone or Accessibility on the hotkey callback.
    let (id, recording) = {
        let mut sessions = state.sessions.lock_repair(|sessions| sessions.cancel());
        if !state.recording.swap(false, Ordering::SeqCst) {
            return;
        }
        state.processing.store(true, Ordering::SeqCst);
        let id = sessions.id().unwrap_or(0);
        let recording = state.recorder.stop_async();
        sessions.resolve_capture(id, release);
        (id, recording)
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
        state
            .sessions
            .lock_repair(|sessions| sessions.cancel())
            .retain_audio(id, &samples);
        let cfg = match state
            .sessions
            .lock_repair(|sessions| sessions.cancel())
            .get(id)
        {
            Some(job) => job.cfg.clone(),
            None => return,
        };
        let stream = state
            .sessions
            .lock_repair(|sessions| sessions.cancel())
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
                if state
                    .sessions
                    .lock_repair(|sessions| sessions.cancel())
                    .get(id)
                    .is_none()
                {
                    return;
                }
                let wait_start = std::time::Instant::now();
                let guard = state.engine.lock_recover();
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

pub(crate) fn deliver_result(
    app: &AppHandle,
    id: u64,
    text: String,
    timing: &mut metrics::Timing,
) -> &'static str {
    let insertion = std::time::Instant::now();
    let state = app.state::<AppState>();
    let (result, retained) = {
        let mut sessions = state.sessions.lock_repair(|sessions| sessions.cancel());
        let result = sessions.deliver(id, text, |target, text| {
            target
                .ok_or("No safe destination captured; use recovery text")?
                .insert(text)
        });
        let retained = if matches!(result, Ok(false)) {
            None
        } else {
            sessions.take_history(id)
        };
        (result, retained)
    };
    timing.stage("insertion", insertion);
    if matches!(result, Ok(true)) {
        timing.mark("release_to_inserted");
        timing.inserted();
    }
    if let Some(text) = retained {
        let persistence = std::time::Instant::now();
        history::append(&text);
        timing.stage("persistence_enqueue", persistence);
        let handle = app.clone();
        std::thread::spawn(move || refresh_menu(&handle));
    }
    match result {
        Ok(true) => "inserted",
        Ok(false) => "cancelled_or_stale",
        Err(e) => {
            log::warn!("{e}");
            "recoverable"
        }
    }
}
