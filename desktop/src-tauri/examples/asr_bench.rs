//! Offline engine benchmark; never captures audio or injects text.
#![allow(dead_code)]
#[path = "../src/config.rs"]
mod config;
#[path = "../src/history.rs"]
mod history;
#[path = "../src/lists.rs"]
mod lists;
#[path = "../src/profile.rs"]
mod profile;
#[path = "../src/transcribe.rs"]
mod transcribe;

use std::time::Instant;

#[derive(serde::Deserialize)]
struct Clip {
    id: String,
    path: String,
    seconds: f64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("usage: asr_bench MODEL MANIFEST REPEATS".into());
    }
    let clips: Vec<Clip> = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    if clips.is_empty() {
        return Err("empty manifest".into());
    }
    let repeats: usize = args[3].parse()?;
    let audio: Vec<Vec<f32>> = clips
        .iter()
        .map(|c| {
            let data = std::fs::read(&c.path)?;
            if data.len() % 4 != 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "unaligned f32 PCM",
                ));
            }
            Ok(data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect())
        })
        .collect::<Result<_, std::io::Error>>()?;
    // Explicit product defaults, independent of subsequent user config changes.
    let cfg = config::Config::default();
    let start = Instant::now();
    let engine = transcribe::Engine::load(&args[1], &cfg)?;
    println!(
        "{}",
        serde_json::json!({"event":"load","model":"whisper-turbo-current","seconds":start.elapsed().as_secs_f64()})
    );
    for pass in 0..=repeats {
        for (i, clip) in clips
            .iter()
            .enumerate()
            .take(if pass == 0 { 1 } else { clips.len() })
        {
            let start = Instant::now();
            let text = engine.transcribe(&audio[i])?;
            println!(
                "{}",
                serde_json::json!({"event":"transcribe","model":"whisper-turbo-current","id":clip.id,"pass":pass,"seconds":start.elapsed().as_secs_f64(),"audio_seconds":clip.seconds,"text":text})
            );
        }
    }
    Ok(())
}
