//! Offline production-path accuracy and latency harness. No mic, insertion, or history writes.
#![allow(dead_code)]
#[path = "../src/config.rs"]
mod config;
#[path = "../src/engine.rs"]
mod engine;
#[path = "../src/history.rs"]
mod history;
#[path = "../src/lists.rs"]
mod lists;
#[path = "../src/parakeet.rs"]
mod parakeet;
#[path = "../src/profile.rs"]
mod profile;
#[path = "../src/streaming.rs"]
mod streaming;
#[path = "../src/transcribe.rs"]
mod transcribe;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 3 {
        return Err("review_bench MODEL_DIR MANIFEST [paced]".into());
    }
    let cfg = config::Config::default().for_app(None);
    let started = Instant::now();
    let engine = Arc::new(Mutex::new(Some(engine::Engine::load(&args[1], &cfg)?)));
    println!(
        "{}",
        serde_json::json!({"event":"load","seconds":started.elapsed().as_secs_f64()})
    );
    let clips: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    for pass in 0..=2 {
        for (i, clip) in clips.iter().enumerate() {
            if pass == 0 && i > 0 {
                break;
            }
            let bytes = std::fs::read(clip["path"].as_str().ok_or("missing path")?)?;
            let samples: Vec<f32> = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            let start = Instant::now();
            let prepared = streaming::prepare(&samples);
            let prep = start.elapsed().as_secs_f64();
            let start = Instant::now();
            let text = if prepared.is_empty() {
                String::new()
            } else {
                engine
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .transcribe(&prepared, &cfg)?
            };
            println!(
                "{}",
                serde_json::json!({"event":"batch","id":clip["id"],"pass":pass,"seconds":start.elapsed().as_secs_f64(),"prepare_seconds":prep,"text":text,"condition":clip["condition"]})
            );
            if pass == 0 {
                continue;
            }
            if args.get(3).is_some_and(|s| s == "paced") {
                let (input, worker) = streaming::Worker::spawn(engine.clone(), cfg.clone());
                let capture = Instant::now();
                for (index, chunk) in samples.chunks(1600).enumerate() {
                    let deadline = capture
                        + Duration::from_secs_f64(
                            ((index + 1) * 1600).min(samples.len()) as f64 / 16000.,
                        );
                    if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
                        std::thread::sleep(wait);
                    }
                    input.send(chunk.to_vec(), 16000);
                }
                let release = Instant::now();
                drop(input);
                let result = worker.finish();
                let fallback = result.is_err();
                let text = result.unwrap_or_else(|_| {
                    engine
                        .lock()
                        .unwrap()
                        .as_ref()
                        .unwrap()
                        .transcribe(&prepared, &cfg)
                        .unwrap()
                });
                println!(
                    "{}",
                    serde_json::json!({"event":"paced","id":clip["id"],"pass":pass,"seconds":release.elapsed().as_secs_f64(),"text":text,"fallback":fallback})
                );
            } else {
                let mut segmenter = streaming::Segmenter::default();
                let mut parts = vec![];
                let mut raw = cfg.clone();
                raw.profile.mode = profile::Mode::Literal;
                let mut fallback = false;
                let start = Instant::now();
                for chunk in samples.chunks(1600) {
                    match segmenter.push(chunk) {
                        Ok(segments) => {
                            for segment in segments {
                                parts.push(
                                    engine
                                        .lock()
                                        .unwrap()
                                        .as_ref()
                                        .unwrap()
                                        .transcribe(&streaming::prepare(&segment), &raw)?,
                                );
                            }
                        }
                        Err(_) => {
                            fallback = true;
                            break;
                        }
                    }
                }
                if !fallback {
                    if let Some(tail) = segmenter.finish() {
                        parts.push(
                            engine
                                .lock()
                                .unwrap()
                                .as_ref()
                                .unwrap()
                                .transcribe(&streaming::prepare(&tail), &raw)?,
                        );
                    }
                }
                let text = if fallback {
                    engine
                        .lock()
                        .unwrap()
                        .as_ref()
                        .unwrap()
                        .transcribe(&prepared, &cfg)?
                } else {
                    cfg.profile
                        .format(&parts.join(" "), true, cfg.remove_fillers, cfg.auto_lists)
                };
                println!(
                    "{}",
                    serde_json::json!({"event":"segmented_offline","id":clip["id"],"pass":pass,"seconds":start.elapsed().as_secs_f64(),"text":text,"fallback":fallback,"condition":clip["condition"]})
                );
            }
        }
    }
    Ok(())
}
