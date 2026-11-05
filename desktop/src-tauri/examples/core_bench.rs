//! Offline release-path proxy. Uses real preparation, inference and session
//! delivery, but an in-memory sink: NO microphone, AX insertion or history writes.
#![allow(dead_code)]
#[path = "../src/audio.rs"]
mod audio;
#[path = "../src/config.rs"]
mod config;
#[path = "../src/engine.rs"]
mod engine;
#[path = "../src/history.rs"]
mod history;
#[path = "../src/inject.rs"]
mod inject;
#[path = "../src/lists.rs"]
mod lists;
#[path = "../src/locks.rs"]
mod locks;
#[path = "../src/metrics.rs"]
mod metrics;
#[path = "../src/parakeet.rs"]
mod parakeet;
#[path = "../src/profile.rs"]
mod profile;
#[path = "../src/session.rs"]
mod session;
#[path = "../src/streaming.rs"]
mod streaming;
#[path = "../src/transcribe.rs"]
mod transcribe;

use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<_> = std::env::args().collect();
    if !(4..=5).contains(&args.len()) {
        return Err("core_bench MODEL MANIFEST REPEATS [IDLE_SECONDS]".into());
    }
    let repeats: usize = args[3].parse()?;
    if repeats == 0 {
        return Err("REPEATS must be positive".into());
    }
    let clips: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    if clips.is_empty() {
        return Err("empty manifest".into());
    }
    let cfg = config::Config::default().for_app(None);
    let load = Instant::now();
    let engine = std::sync::Mutex::new(engine::Engine::load(&args[1], &cfg)?);
    println!(
        "{}",
        serde_json::json!({"event":"load", "seconds":load.elapsed().as_secs_f64(), "pid":std::process::id()})
    );
    if let Some(seconds) = args.get(4) {
        std::thread::sleep(Duration::from_secs(seconds.parse()?));
    }
    let mut sessions = session::Sessions::default();
    for pass in 0..=repeats {
        for (index, clip) in clips.iter().enumerate() {
            if pass == 0 && index > 0 {
                break;
            }
            let bytes = std::fs::read(clip["path"].as_str().ok_or("missing path")?)?;
            if bytes.len() % 4 != 0 {
                return Err("invalid PCM".into());
            }
            let samples: Vec<f32> = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            // Match a 48 kHz callback buffer without a new acoustic resampler:
            // repeating each 16 kHz sample lets production decimation recover it.
            let recording = audio::Recording {
                samples: samples.iter().flat_map(|s| [*s; 3]).collect(),
                sample_rate: 48_000,
                last_speech: None,
            };
            let id = sessions.begin();
            let release = Instant::now();
            let mut timing = metrics::Timing::new(release);
            let prepared = streaming::prepare(&audio::resample_to_16k(&recording));
            sessions.retain_audio(id, &prepared);
            timing.stage("resample_gate_gain_retain", release);
            let wait = Instant::now();
            let engine = engine.lock().map_err(|_| "benchmark engine poisoned")?;
            timing.stage("engine_wait", wait);
            let inference = Instant::now();
            let text = engine.transcribe(&prepared, &cfg)?;
            timing.stage("recognition_and_format", inference);
            drop(engine);
            timing.mark("release_to_final_text");
            let insertion = Instant::now();
            let mut sink = String::new();
            sessions.deliver(id, text.clone(), |_, text| {
                sink.push_str(text);
                Ok(())
            })?;
            timing.stage("offline_delivery", insertion);
            timing.mark("offline_release_to_sink");
            timing.emit("offline_sink");
            println!(
                "{}",
                serde_json::json!({"event":"offline_sink", "id":clip["id"], "pass":pass, "text":text, "stages_ms":timing.stages_ms})
            );
        }
    }
    Ok(())
}
