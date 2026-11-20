//! Opt-in smoke test for the meeting summarizer. NOT a default test; CI must
//! not run it with a model.
//!
//!   cargo run --example summary_smoke -- --download
//!       Fetch the pinned model into the app's normal cache with the app's
//!       own download + verify path, then print verified sizes and hashes.
//!   GRETA_SUMMARY_MODEL_DIR=<dir> cargo run --example summary_smoke
//!       Summarize the synthetic fixture transcript with the helper
//!       (GRETA_SUMMARIZER_HELPER or the staged path) and check grounding.
//!       Without GRETA_SUMMARY_MODEL_DIR it prints "skipped" and exits 0.
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
#[path = "../src/meeting.rs"]
mod meeting;
#[path = "../src/meeting_notes.rs"]
mod meeting_notes;
#[path = "../src/metrics.rs"]
mod metrics;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/parakeet.rs"]
mod parakeet;
#[path = "../src/profile.rs"]
mod profile;
#[path = "../src/session.rs"]
mod session;
#[path = "../src/streaming.rs"]
mod streaming;
#[path = "../src/summary.rs"]
mod summary;
#[path = "../src/system_audio.rs"]
mod system_audio;
#[path = "../src/transcribe.rs"]
mod transcribe;

use std::path::{Path, PathBuf};
use std::time::Instant;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(serde::Deserialize)]
struct FixtureSegment {
    index: usize,
    start_ms: u64,
    text: String,
}

fn main() -> Res<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    if std::env::args().nth(1).as_deref() == Some("--download") {
        return download();
    }
    let Some(model_dir) = std::env::var_os("GRETA_SUMMARY_MODEL_DIR") else {
        println!("skipped: GRETA_SUMMARY_MODEL_DIR is not set");
        return Ok(());
    };
    smoke(Path::new(&model_dir))
}

fn download() -> Res<()> {
    let dest = summary::default_model_dir();
    let manifest = summary::manifest();
    println!("model: {}", manifest.label());
    println!("dest:  {}", dest.display());
    summary::begin_download()?;
    let result = summary::download_pinned(&dest);
    summary::end_download(&result);
    let dir = result?;
    summary::verify_manifest(&dir, &manifest)?;
    for name in manifest.files.keys() {
        let path = dir.join(name);
        let size = std::fs::metadata(&path)?.len();
        let sha = model::file_sha256(&path)?;
        println!("verified {name}: {size} B sha256 {sha}");
    }
    Ok(())
}

fn smoke(model_dir: &Path) -> Res<()> {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meeting_transcript.json");
    let raw: Vec<FixtureSegment> = serde_json::from_slice(&std::fs::read(&fixture)?)?;
    let mut segments = Vec::with_capacity(raw.len());
    for (i, s) in raw.iter().enumerate() {
        if s.index != i {
            return Err(format!("fixture index {} out of order at {i}", s.index).into());
        }
        let end_ms = raw.get(i + 1).map_or(s.start_ms + 5000, |n| n.start_ms);
        segments.push(meeting::Segment {
            start_ms: s.start_ms,
            end_ms,
            text: s.text.clone(),
            source: Default::default(),
        });
    }
    let info = summary::validate_model_dir(model_dir)?;
    println!("model dir: {} ({info:?})", model_dir.display());
    let helper = summary::helper_path()?;
    println!("helper:    {}", helper.display());
    println!("segments:  {}", segments.len());

    let started = Instant::now();
    let model_dir_str = model_dir.to_str().ok_or("model dir is not UTF-8")?;
    let output = summary::summarize(&helper, model_dir_str, &segments, summary::DEFAULT_TIMEOUT)
        .map_err(|e| {
            format!("summarize failed (raw output did not parse or helper failed): {e}")
        })?;
    let elapsed = started.elapsed();

    let (bullets, dropped_b) =
        meeting_notes::validate_model_grounding(&segments, output.bullets.clone());
    let (actions, dropped_a) =
        meeting_notes::validate_model_grounding(&segments, output.action_items.clone());

    println!(
        "\nraw: {} bullets, {} action items",
        output.bullets.len(),
        output.action_items.len()
    );
    print_dropped("bullet", &segments, &output.bullets);
    print_dropped("action", &segments, &output.action_items);
    println!("\nkept bullets ({}):", bullets.len());
    for item in &bullets {
        check(&segments, item)?;
        println!(
            "  [{}] {}  <- \"{}\"",
            item.segment_index, item.text, item.quote
        );
    }
    println!("kept action items ({}):", actions.len());
    for item in &actions {
        check(&segments, item)?;
        println!(
            "  [{}] {} (owner: {})  <- \"{}\"",
            item.segment_index,
            item.text,
            item.owner.as_deref().unwrap_or("-"),
            item.quote
        );
    }
    println!("dropped: {dropped_b} bullets, {dropped_a} action items");
    if bullets.is_empty() && actions.is_empty() {
        return Err("no grounded items survived".into());
    }
    println!("elapsed: {:.2}s", elapsed.as_secs_f64());
    println!("ok");
    Ok(())
}

fn check(segments: &[meeting::Segment], item: &meeting_notes::SummaryItem) -> Res<()> {
    let segment = segments
        .get(item.segment_index)
        .ok_or("kept item cites a missing segment")?;
    if item.quote.trim().is_empty() || !segment.text.contains(&item.quote) {
        return Err(format!("kept item failed grounding: {item:?}").into());
    }
    if let Some(owner) = &item.owner {
        if !item.quote.contains(owner.as_str()) {
            return Err(format!("kept owner not in quote: {item:?}").into());
        }
    }
    Ok(())
}

fn print_dropped(kind: &str, segments: &[meeting::Segment], items: &[meeting_notes::ModelItem]) {
    for item in items {
        let grounded = segments
            .get(item.segment_index)
            .is_some_and(|s| !item.quote.trim().is_empty() && s.text.contains(&item.quote));
        if !grounded {
            println!(
                "  dropped {kind} [{}] {}  <- \"{}\"",
                item.segment_index, item.text, item.quote
            );
        }
    }
}
