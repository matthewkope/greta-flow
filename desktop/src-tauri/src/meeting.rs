//! Meeting notes: explicit Start/Stop capture with sample-timestamped segments.
//!
//! Capture → `Input` (bounded, never blocks the audio callback) → prep thread
//! (gap accounting, 16 kHz resampling, silence/length segmentation) → bounded
//! decode queue → decoder thread (existing ASR engine). Every timestamp is
//! derived from sample offsets on the 16 kHz timeline, never from wall clocks.
//! Lost audio is recorded as an explicit gap; the timeline stays aligned.
//!
//! A meeting captures the default microphone, system audio (through the
//! `greta-system-audio` helper, see `system_audio.rs`), or both. Each source
//! runs its own pipeline and sample timeline; segments carry a source label
//! (not a speaker identity) and are merged by start time when saved.
use crate::locks::MutexExt;
use crate::meeting_notes::{self, ActionItem, SummaryStatus};
use crate::streaming::{prepare, Resampler};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const RATE: u64 = 16_000;
const FRAME: usize = 160;
const MAX_SEGMENT: usize = 30 * RATE as usize;
const SILENCE: usize = RATE as usize;
const PAD: usize = RATE as usize / 5;
/// Continuous speech is cut at the quietest frame within this trailing window.
const CUT_WINDOW: usize = 3 * RATE as usize;
/// Hard bound on one meeting; capture beyond it is ignored and disclosed.
pub const MAX_MEETING_MS: u64 = 4 * 60 * 60 * 1000;
/// Device blocks waiting for the prep thread (~2–5 s of audio).
const CAPTURE_QUEUE: usize = 256;
/// Segments waiting for the decoder (at most ~6 × 30 s × 4 B × 16 kHz ≈ 11.5 MB).
const DECODE_QUEUE: usize = 6;
const MAX_BLOCK: usize = 192_000;
const CHECKPOINT: Duration = Duration::from_secs(60);
/// Segments returned to a polling UI; the full list is in the saved record.
const LIVE_VIEW_SEGMENTS: usize = 300;
pub const SOURCE_MICROPHONE: &str = "microphone";
pub const SOURCE_SYSTEM: &str = "system";
pub const SOURCE_BOTH: &str = "both";

fn ms(samples_16k: u64) -> u64 {
    samples_16k * 1000 / RATE
}

/// Where a segment's audio came from. A source label only: it says nothing
/// about who spoke. Records saved before system audio default to microphone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioSource {
    #[default]
    Microphone,
    System,
}

/// Which sources a meeting captures.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SourceMode {
    #[default]
    Microphone,
    System,
    Both,
}
impl SourceMode {
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.unwrap_or(SOURCE_MICROPHONE) {
            SOURCE_MICROPHONE => Ok(Self::Microphone),
            SOURCE_SYSTEM => Ok(Self::System),
            SOURCE_BOTH => Ok(Self::Both),
            other => Err(format!("unknown meeting source: {other}")),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Microphone => SOURCE_MICROPHONE,
            Self::System => SOURCE_SYSTEM,
            Self::Both => SOURCE_BOTH,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    #[serde(default)]
    pub source: AudioSource,
}

/// A span of the meeting timeline with no transcript, and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gap {
    pub start_ms: u64,
    pub end_ms: u64,
    pub reason: String,
    #[serde(default)]
    pub source: AudioSource,
}

// ---------------------------------------------------------------------------
// Capture sink
// ---------------------------------------------------------------------------

struct Block {
    /// Index of the first sample on the device-rate timeline.
    offset: u64,
    samples: Vec<f32>,
    rate: u32,
}

/// Audio-callback side of a meeting. Never blocks; a full queue drops the
/// block, which the prep thread reports as a gap from the offset jump.
#[derive(Clone)]
pub struct Input {
    tx: SyncSender<Block>,
    captured: Arc<AtomicU64>,
}
impl Input {
    pub fn send(&self, samples: Vec<f32>, rate: u32) {
        let offset = self
            .captured
            .fetch_add(samples.len() as u64, Ordering::SeqCst);
        if samples.len() > MAX_BLOCK {
            return;
        }
        let _ = self.tx.try_send(Block {
            offset,
            samples,
            rate,
        });
    }
}

// ---------------------------------------------------------------------------
// Segmentation on the 16 kHz timeline
// ---------------------------------------------------------------------------

/// Silence-boundary segmenter that keeps the absolute 16 kHz offset of every
/// emitted segment. Unlike dictation's segmenter, continuous speech never
/// fails: it is cut at the quietest recent frame once it reaches 30 s.
#[derive(Default)]
pub(crate) struct Chunker {
    buffer: Vec<f32>,
    origin: u64,
    tail: usize,
    voiced: bool,
    frame: Vec<f32>,
}
fn energy(frame: &[f32]) -> f32 {
    frame.iter().map(|v| v * v).sum::<f32>() / frame.len().max(1) as f32
}
fn is_voice(frame: &[f32]) -> bool {
    energy(frame) > 0.0001f32.powi(2)
}
impl Chunker {
    pub(crate) fn push(&mut self, samples: &[f32]) -> Vec<(u64, Vec<f32>)> {
        let mut out = vec![];
        for &s in samples {
            self.frame.push(s);
            if self.frame.len() < FRAME {
                continue;
            }
            let voice = is_voice(&self.frame);
            self.buffer.append(&mut self.frame);
            if voice {
                self.voiced = true;
                self.tail = 0;
            } else {
                self.tail += FRAME;
            }
            if self.voiced && self.tail >= SILENCE {
                let end = self.buffer.len() - self.tail + PAD;
                out.push((self.origin, self.buffer[..end].to_vec()));
                let keep = self.buffer.len() - PAD;
                self.buffer.drain(..keep);
                self.origin += keep as u64;
                self.voiced = false;
                self.tail = PAD;
            } else if !self.voiced && self.buffer.len() > PAD {
                let drop = self.buffer.len() - PAD;
                self.buffer.drain(..drop);
                self.origin += drop as u64;
            }
            if self.buffer.len() >= MAX_SEGMENT {
                let cut = self.quietest_cut();
                out.push((self.origin, self.buffer[..cut].to_vec()));
                self.buffer.drain(..cut);
                self.origin += cut as u64;
                self.tail = 0;
            }
        }
        out
    }
    /// Frame-aligned cut point at the start of the quietest frame in the
    /// trailing window; always leaves a non-empty segment.
    fn quietest_cut(&self) -> usize {
        let len = self.buffer.len() - self.buffer.len() % FRAME;
        let first = len.saturating_sub(CUT_WINDOW).max(FRAME);
        (first..len)
            .step_by(FRAME)
            .min_by(|a, b| {
                energy(&self.buffer[*a..*a + FRAME])
                    .total_cmp(&energy(&self.buffer[*b..*b + FRAME]))
            })
            .unwrap_or(len)
    }
    pub(crate) fn finish(&mut self) -> Option<(u64, Vec<f32>)> {
        self.buffer.append(&mut self.frame);
        if self.voiced || self.buffer.iter().any(|v| v.abs() > 0.0001) {
            Some((self.origin, std::mem::take(&mut self.buffer)))
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Live state shared by the pipeline threads and the UI
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Live {
    source: AudioSource,
    segments: Vec<Segment>,
    gaps: Vec<Gap>,
    /// Processed 16 kHz timeline length.
    timeline: u64,
    limit_reached: bool,
    error: Option<String>,
}
impl Live {
    fn gap(&mut self, start: u64, end: u64, reason: &str) {
        if end > start {
            self.gaps.push(Gap {
                start_ms: ms(start),
                end_ms: ms(end),
                reason: reason.into(),
                source: self.source,
            });
        }
    }
    fn for_source(source: AudioSource) -> Self {
        Self {
            source,
            ..Self::default()
        }
    }
}

/// One view over several sources: segments and gaps ordered by start time
/// (stable, so microphone precedes system on ties), timeline = longest source.
fn merge(lives: &[Arc<Mutex<Live>>]) -> Live {
    let mut out = Live::default();
    for live in lives {
        let live = live.lock_recover();
        out.segments.extend(live.segments.iter().cloned());
        out.gaps.extend(live.gaps.iter().cloned());
        out.timeline = out.timeline.max(live.timeline);
        out.limit_reached |= live.limit_reached;
        if out.error.is_none() {
            out.error = live.error.clone();
        }
    }
    out.segments.sort_by_key(|s| s.start_ms);
    out.gaps.sort_by_key(|g| (g.start_ms, g.end_ms));
    out
}

struct Pending {
    start: u64,
    samples: Vec<f32>,
}

pub(crate) type Transcriber = Box<dyn Fn(&[f32]) -> Result<String, String> + Send>;

/// Device-rate blocks → 16 kHz segments. Returns when capture stops.
fn run_prep(
    rx: Receiver<Block>,
    captured: Arc<AtomicU64>,
    decode: SyncSender<Pending>,
    live: Arc<Mutex<Live>>,
    limit_ms: u64,
) {
    let mut resampler = Resampler::default();
    let mut chunker = Chunker::default();
    let mut expected = 0u64;
    let mut rate = 0u32;
    let limit = limit_ms * RATE / 1000;
    let send = |pending: Pending, live: &Mutex<Live>| {
        let (start, len) = (pending.start, pending.samples.len() as u64);
        match decode.try_send(pending) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                live.lock_recover()
                    .gap(start, start + len, "transcriber fell behind");
            }
        }
    };
    let mut stopped = false;
    let mut feed = |samples: &[f32], block_rate: u32, stopped: &mut bool| {
        if *stopped {
            return;
        }
        let out = match resampler.push(samples, block_rate) {
            Ok(out) => out,
            Err(e) => {
                live.lock_recover().error = Some(format!("capture stopped: {e}"));
                *stopped = true;
                return;
            }
        };
        let room = limit.saturating_sub(live.lock_recover().timeline) as usize;
        let take = out.len().min(room);
        for (start, samples) in chunker.push(&out[..take]) {
            send(Pending { start, samples }, &live);
        }
        let mut state = live.lock_recover();
        state.timeline += take as u64;
        if take < out.len() {
            state.limit_reached = true;
            *stopped = true;
        }
    };
    for block in rx {
        if stopped {
            continue; // keep draining so the callback never sees a full queue
        }
        if rate == 0 {
            rate = block.rate;
        }
        if block.offset > expected {
            // Dropped blocks: keep the timeline aligned with silence and say so.
            let start = expected * RATE / rate as u64;
            let end = block.offset * RATE / rate as u64;
            live.lock_recover()
                .gap(start, end, "audio dropped (capture queue full)");
            let silence = vec![0.; rate as usize];
            while expected < block.offset && !stopped {
                let n = (block.offset - expected).min(rate as u64) as usize;
                feed(&silence[..n], rate, &mut stopped);
                expected += n as u64;
            }
        }
        expected = block.offset + block.samples.len() as u64;
        feed(&block.samples, block.rate, &mut stopped);
    }
    if !stopped {
        let tail = resampler.finish();
        let room = limit.saturating_sub(live.lock_recover().timeline) as usize;
        let take = tail.len().min(room);
        for (start, samples) in chunker.push(&tail[..take]) {
            send(Pending { start, samples }, &live);
        }
        live.lock_recover().timeline += take as u64;
    }
    // Speech already segmented up to a limit or error is still recognized.
    if let Some((start, samples)) = chunker.finish() {
        send(Pending { start, samples }, &live);
    }
    let total = captured.load(Ordering::SeqCst);
    if rate != 0 && total > expected && !stopped {
        let start = expected * RATE / rate as u64;
        let end = total * RATE / rate as u64;
        let mut state = live.lock_recover();
        state.gap(start, end, "audio dropped (capture queue full)");
        state.timeline = state.timeline.max(end);
    }
}

/// Recognize queued segments in order. `checkpoint` runs at most once per
/// `CHECKPOINT` so an interrupted meeting keeps its transcript.
fn run_decoder(
    rx: Receiver<Pending>,
    transcribe: Transcriber,
    live: Arc<Mutex<Live>>,
    checkpoint: impl Fn(),
) {
    let mut saved = Instant::now();
    for pending in rx {
        let end = pending.start + pending.samples.len() as u64;
        let prepared = prepare(&pending.samples);
        if prepared.is_empty() {
            continue; // below the noise gate: silence, not a failure
        }
        match transcribe(&prepared) {
            Ok(text) if text.trim().is_empty() => {}
            Ok(text) => {
                let mut live = live.lock_recover();
                let source = live.source;
                live.segments.push(Segment {
                    start_ms: ms(pending.start),
                    end_ms: ms(end),
                    text: text.trim().to_string(),
                    source,
                })
            }
            Err(e) => {
                log::warn!("meeting segment recognition failed: {e}");
                live.lock_recover()
                    .gap(pending.start, end, "transcription failed");
            }
        }
        if saved.elapsed() >= CHECKPOINT {
            checkpoint();
            saved = Instant::now();
        }
    }
}

// ---------------------------------------------------------------------------
// Records and local storage
// ---------------------------------------------------------------------------

/// One meeting, shaped as a superset of the shared history `Entry` (v2):
/// `{id, kind:"meeting", started_ms, ended_ms, audio_ms, speech_ms, words,
/// text, segments, title, tz_offset_min}` plus meeting-only fields.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MeetingRecord {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub started_ms: u64,
    pub ended_ms: u64,
    /// Length of the captured sample timeline.
    pub audio_ms: u64,
    /// Total span of segments that produced text.
    pub speech_ms: u64,
    pub words: usize,
    pub text: String,
    pub segments: Vec<Segment>,
    pub gaps: Vec<Gap>,
    pub source: String,
    pub tz_offset_min: i32,
    pub action_items: Vec<ActionItem>,
    pub summary: SummaryStatus,
    /// False for a checkpoint of a meeting that has not been stopped.
    pub complete: bool,
    #[serde(default)]
    pub limit_reached: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct MeetingListItem {
    pub id: String,
    pub title: String,
    pub started_ms: u64,
    pub ended_ms: u64,
    pub audio_ms: u64,
    pub words: usize,
    pub segments: usize,
    pub action_items: usize,
    pub complete: bool,
    pub preview: String,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn local_tm(ms: u64) -> libc::tm {
    let secs = (ms / 1000) as libc::time_t;
    // SAFETY: localtime_r writes only into the provided, zero-initialized tm.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&secs, &mut tm);
        tm
    }
}
pub fn tz_offset_min(ms: u64) -> i32 {
    (local_tm(ms).tm_gmtoff / 60) as i32
}
fn default_title(started_ms: u64) -> String {
    let tm = local_tm(started_ms);
    format!(
        "Meeting {:04}-{:02}-{:02} {:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}

pub(crate) struct Meta {
    id: String,
    title: String,
    started_ms: u64,
    tz_offset_min: i32,
    source: &'static str,
}
impl Meta {
    fn with_source(started_ms: u64, mode: SourceMode) -> Self {
        Self {
            source: mode.label(),
            ..Self::new(started_ms)
        }
    }
    pub(crate) fn new(started_ms: u64) -> Self {
        Self {
            id: format!("m{started_ms}"),
            title: default_title(started_ms),
            started_ms,
            tz_offset_min: tz_offset_min(started_ms),
            source: SOURCE_MICROPHONE,
        }
    }
}

fn build_record(meta: &Meta, live: &Live, ended_ms: u64, complete: bool) -> MeetingRecord {
    let segments = live.segments.clone();
    let mut gaps = live.gaps.clone();
    gaps.sort_by_key(|g| (g.start_ms, g.end_ms));
    let text = segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    // Every stored item must quote its cited segment; the same gate applies to
    // any future summarizer's output.
    let action_items = meeting_notes::validate_grounding(
        meeting_notes::candidate_action_items(&segments),
        &segments,
    );
    MeetingRecord {
        id: meta.id.clone(),
        kind: "meeting".into(),
        title: meta.title.clone(),
        started_ms: meta.started_ms,
        ended_ms,
        audio_ms: ms(live.timeline),
        speech_ms: segments.iter().map(|s| s.end_ms - s.start_ms).sum(),
        words: text.split_whitespace().count(),
        text,
        segments,
        gaps,
        source: meta.source.into(),
        tz_offset_min: meta.tz_offset_min,
        action_items,
        summary: SummaryStatus::unavailable(),
        complete,
        limit_reached: live.limit_reached,
    }
}

/// Private per-meeting JSON files; transcripts never leave this directory.
pub struct Store {
    dir: PathBuf,
}
pub fn meetings_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(crate::config::storage_name())
        .join("meetings")
}
/// Ids are generated as `m<epoch ms>`; anything else is rejected before it
/// can name a path.
pub fn valid_id(id: &str) -> bool {
    id.len() > 1
        && id.len() <= 21
        && id.starts_with('m')
        && id[1..].bytes().all(|b| b.is_ascii_digit())
}
const MAX_RECORD_BYTES: u64 = 64 * 1024 * 1024;
impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
    pub fn default_store() -> Self {
        Self::new(meetings_dir())
    }
    fn path(&self, id: &str) -> Result<PathBuf, String> {
        if !valid_id(id) {
            return Err("invalid meeting id".into());
        }
        Ok(self.dir.join(format!("{id}.json")))
    }
    pub fn save(&self, record: &MeetingRecord) -> Result<(), String> {
        use std::io::Write;
        // Two source pipelines may checkpoint at once; the temp name is per process.
        static SAVING: Mutex<()> = Mutex::new(());
        let _saving = SAVING.lock_recover();
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        let path = self.path(&record.id)?;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)
            .map_err(|e| format!("couldn't create meetings folder: {}", e.kind()))?;
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        let result = (|| -> std::io::Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .mode(0o600)
                .open(&temp)?;
            serde_json::to_writer(&mut file, record)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            std::fs::rename(&temp, &path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result.map_err(|e| format!("couldn't save meeting: {}", e.kind()))
    }
    fn read(path: &Path) -> Option<MeetingRecord> {
        let size = std::fs::metadata(path).ok()?.len();
        if size > MAX_RECORD_BYTES {
            return None;
        }
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    }
    pub fn get(&self, id: &str) -> Result<Option<MeetingRecord>, String> {
        let path = self.path(id)?;
        Ok(Self::read(&path).filter(|r| r.id == id))
    }
    pub fn delete(&self, id: &str) -> Result<(), String> {
        match std::fs::remove_file(self.path(id)?) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("couldn't delete meeting: {}", e.kind()))
            }
            _ => Ok(()),
        }
    }
    /// All saved meetings, newest first.
    pub fn list(&self) -> Vec<MeetingListItem> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return vec![];
        };
        let mut items: Vec<MeetingListItem> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                let id = name.strip_suffix(".json")?;
                if !valid_id(id) {
                    return None;
                }
                let r = Self::read(&entry.path()).filter(|r| r.id == id)?;
                let preview: String = r.text.chars().take(140).collect();
                Some(MeetingListItem {
                    id: r.id,
                    title: r.title,
                    started_ms: r.started_ms,
                    ended_ms: r.ended_ms,
                    audio_ms: r.audio_ms,
                    words: r.words,
                    segments: r.segments.len(),
                    action_items: r.action_items.len(),
                    complete: r.complete,
                    preview,
                })
            })
            .collect();
        items.sort_by_key(|item| std::cmp::Reverse(item.started_ms));
        items
    }
}

// ---------------------------------------------------------------------------
// Controller
// ---------------------------------------------------------------------------

/// One source's pipeline: prep and decoder threads over one `Live`.
struct Running {
    #[allow(dead_code)]
    meta: Arc<Meta>,
    #[allow(dead_code)]
    started: Instant,
    #[allow(dead_code)]
    live: Arc<Mutex<Live>>,
    prep: std::thread::JoinHandle<()>,
    decoder: std::thread::JoinHandle<()>,
}

struct SystemRun {
    pipeline: Running,
    capture: crate::system_audio::Capture,
}

/// A recording meeting: one pipeline per captured source.
struct Session {
    meta: Arc<Meta>,
    started: Instant,
    /// Microphone first, then system; the merge order for ties.
    lives: Vec<Arc<Mutex<Live>>>,
    mic: Option<Running>,
    system: Option<SystemRun>,
    system_live: Option<Arc<Mutex<Live>>>,
    /// System audio failed to start in Both mode; the meeting is mic-only.
    system_notice: Option<String>,
}

#[derive(Default)]
enum Phase {
    #[default]
    Idle,
    Reserved,
    Recording(Box<Session>),
    Finishing,
}

/// Owns at most one meeting. `active` stays true from reservation until the
/// last segment is recognized, so dictation cannot share the microphone or
/// engine with a meeting.
#[derive(Default)]
pub struct Controller {
    active: AtomicBool,
    phase: Mutex<Phase>,
    last_error: Mutex<Option<String>>,
}

#[derive(Serialize)]
pub struct StateView {
    pub active: bool,
    pub finishing: bool,
    pub id: Option<String>,
    pub started_ms: Option<u64>,
    pub elapsed_ms: u64,
    /// Length of audio processed so far (sample-based).
    pub audio_ms: u64,
    pub source: &'static str,
    pub segments: Vec<Segment>,
    pub segment_count: usize,
    pub gaps: Vec<Gap>,
    pub summary: SummaryStatus,
    pub limit_reached: bool,
    pub error: Option<String>,
    pub disclosure: &'static str,
}
pub const DISCLOSURE: &str = "Source labels (mic / system) say where audio came from, not who \
spoke; there is no speaker identification. Microphone captures whatever reaches your mic. System \
audio captures what your Mac plays (for example remote Zoom, Teams or Meet participants) and needs \
macOS 14.2 or later plus the System Audio Recording permission. In Both mode, remote audio played \
on speakers also reaches the microphone, so the same words can appear twice; use headphones to \
avoid this. System-audio capture has not yet been tested on real Zoom or Teams calls.";

/// Opens the microphone with the given sink; returns the device rate.
pub(crate) type MicOpen<'a> = Box<dyn FnOnce(Input) -> Result<u32, String> + 'a>;
/// Starts system-audio capture feeding the given sink; returns the helper.
pub(crate) type SystemOpen<'a> =
    Box<dyn FnOnce(Input) -> Result<(crate::system_audio::Capture, u32), String> + 'a>;

impl Controller {
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }
    /// Claim the single meeting slot. The caller must hold the dictation
    /// session lock and have checked that dictation is idle.
    pub fn reserve(&self) -> Result<(), String> {
        if self.active.swap(true, Ordering::SeqCst) {
            return Err("A meeting is already recording".into());
        }
        *self.phase.lock_recover() = Phase::Reserved;
        *self.last_error.lock_recover() = None;
        Ok(())
    }
    fn release(&self, error: Option<String>) {
        *self.phase.lock_recover() = Phase::Idle;
        *self.last_error.lock_recover() = error;
        self.active.store(false, Ordering::SeqCst);
    }
    /// Start a microphone-only meeting after `reserve`. `open` starts capture
    /// with the given sink and returns the device rate.
    #[allow(dead_code)]
    pub(crate) fn begin(
        &self,
        open: impl FnOnce(Input) -> Result<u32, String>,
        transcribe: Transcriber,
        store: Arc<Store>,
    ) -> Result<(), String> {
        let mut transcribe = Some(transcribe);
        self.begin_sources(
            SourceMode::Microphone,
            Box::new(open),
            Box::new(|_| Err("system audio was not requested".into())),
            move || {
                transcribe
                    .take()
                    .unwrap_or_else(|| Box::new(|_| Err("no transcriber".into())))
            },
            store,
        )
    }
    /// Start the pipelines for `mode` after `reserve`. A failed microphone
    /// fails the start; failed system audio fails a System-only start and
    /// leaves a Both meeting recording the microphone with a recorded notice.
    pub(crate) fn begin_sources(
        &self,
        mode: SourceMode,
        open_mic: MicOpen<'_>,
        open_system: SystemOpen<'_>,
        mut transcriber: impl FnMut() -> Transcriber,
        store: Arc<Store>,
    ) -> Result<(), String> {
        if !matches!(*self.phase.lock_recover(), Phase::Reserved) {
            return Err("meeting slot was not reserved".into());
        }
        let meta = Arc::new(Meta::with_source(now_ms(), mode));
        let new_live = |source| Arc::new(Mutex::new(Live::for_source(source)));
        let mic_live = (mode != SourceMode::System).then(|| new_live(AudioSource::Microphone));
        let system_live = (mode != SourceMode::Microphone).then(|| new_live(AudioSource::System));
        let lives: Vec<_> = mic_live.iter().chain(system_live.iter()).cloned().collect();
        let mut mic = None;
        if let Some(live) = &mic_live {
            let (running, input) = spawn_pipeline_in(
                Arc::clone(&meta),
                Arc::clone(live),
                lives.clone(),
                transcriber(),
                Arc::clone(&store),
                MAX_MEETING_MS,
            );
            match open_mic(input) {
                Ok(rate) => {
                    log::info!("meeting capture started source=microphone rate={rate}");
                    mic = Some(running);
                }
                Err(e) => {
                    drop(running);
                    self.release(Some(e.clone()));
                    return Err(e);
                }
            }
        }
        let mut system = None;
        let mut system_notice = None;
        if let Some(live) = &system_live {
            let (running, input) = spawn_pipeline_in(
                Arc::clone(&meta),
                Arc::clone(live),
                lives.clone(),
                transcriber(),
                Arc::clone(&store),
                MAX_MEETING_MS,
            );
            match open_system(input) {
                Ok((capture, rate)) => {
                    log::info!("meeting capture started source=system rate={rate}");
                    system = Some(SystemRun {
                        pipeline: running,
                        capture,
                    });
                }
                Err(e) if mode == SourceMode::System => {
                    drop(running);
                    self.release(Some(e.clone()));
                    return Err(e);
                }
                Err(e) => {
                    log::warn!("system audio unavailable; recording microphone only: {e}");
                    drop(running);
                    live.lock_recover().error = Some(format!(
                        "System audio unavailable; recording microphone only: {e}"
                    ));
                    system_notice = Some(e);
                }
            }
        }
        *self.phase.lock_recover() = Phase::Recording(Box::new(Session {
            meta,
            started: Instant::now(),
            lives,
            mic,
            system,
            system_live,
            system_notice,
        }));
        Ok(())
    }
    /// Stop capture (via `stop`, which must drop the microphone sink; system
    /// audio is stopped here), recognize the queued tail, and save the record.
    pub(crate) fn finish(
        &self,
        stop: impl FnOnce(),
        store: &Store,
    ) -> Result<MeetingRecord, String> {
        let mut session = {
            let mut phase = self.phase.lock_recover();
            match std::mem::replace(&mut *phase, Phase::Finishing) {
                Phase::Recording(session) => session,
                other => {
                    *phase = other;
                    return Err("No meeting is recording".into());
                }
            }
        };
        stop();
        let mut system_ended = None;
        let mut pipelines = vec![];
        if let Some(mut run) = session.system.take() {
            system_ended = run.capture.ended();
            run.capture.stop();
            pipelines.push(run.pipeline);
        }
        pipelines.extend(session.mic.take());
        let mut panicked = false;
        for running in pipelines {
            panicked |= running.prep.join().is_err();
            panicked |= running.decoder.join().is_err();
        }
        let merged = merge(&session.lives);
        let mut record = build_record(&session.meta, &merged, now_ms(), true);
        let system_end = session
            .system_live
            .as_ref()
            .map(|live| ms(live.lock_recover().timeline))
            .unwrap_or(0);
        let notice = session
            .system_notice
            .as_ref()
            .map(|e| (0, format!("system audio unavailable: {e}")))
            .or_else(|| system_ended.map(|e| (system_end, format!("system audio stopped: {e}"))));
        if let Some((start_ms, reason)) = notice {
            record.gaps.push(Gap {
                start_ms,
                end_ms: record.audio_ms.max(start_ms),
                reason,
                source: AudioSource::System,
            });
            record.gaps.sort_by_key(|g| (g.start_ms, g.end_ms));
        }
        if panicked {
            record.gaps.push(Gap {
                start_ms: 0,
                end_ms: record.audio_ms,
                reason: "meeting worker stopped unexpectedly; transcript may be incomplete".into(),
                source: AudioSource::Microphone,
            });
        }
        let saved = store.save(&record);
        self.release(saved.as_ref().err().cloned());
        saved.map(|()| record)
    }
    pub fn view(&self) -> StateView {
        let phase = self.phase.lock_recover();
        let error = self.last_error.lock_recover().clone();
        let mut view = StateView {
            active: self.is_active(),
            finishing: matches!(*phase, Phase::Finishing),
            id: None,
            started_ms: None,
            elapsed_ms: 0,
            audio_ms: 0,
            source: SOURCE_MICROPHONE,
            segments: vec![],
            segment_count: 0,
            gaps: vec![],
            summary: SummaryStatus::unavailable(),
            limit_reached: false,
            error,
            disclosure: DISCLOSURE,
        };
        if let Phase::Recording(session) = &*phase {
            let live = merge(&session.lives);
            view.id = Some(session.meta.id.clone());
            view.started_ms = Some(session.meta.started_ms);
            view.source = session.meta.source;
            view.elapsed_ms = session.started.elapsed().as_millis() as u64;
            view.audio_ms = ms(live.timeline);
            view.segment_count = live.segments.len();
            let skip = live.segments.len().saturating_sub(LIVE_VIEW_SEGMENTS);
            view.segments = live.segments[skip..].to_vec();
            view.gaps = live.gaps;
            view.limit_reached = live.limit_reached;
            if view.error.is_none() {
                view.error = session
                    .system
                    .as_ref()
                    .and_then(|run| run.capture.ended())
                    .map(|e| format!("System audio stopped: {e}"))
                    .or(live.error);
            }
        }
        view
    }
}

#[allow(dead_code)]
fn spawn_pipeline(
    meta: Arc<Meta>,
    live: Arc<Mutex<Live>>,
    transcribe: Transcriber,
    store: Arc<Store>,
    limit_ms: u64,
) -> (Running, Input) {
    let lives = vec![Arc::clone(&live)];
    spawn_pipeline_in(meta, live, lives, transcribe, store, limit_ms)
}

/// Spawn one source's pipeline. Checkpoints save the merge of all `lives`
/// so a two-source meeting's checkpoint holds both transcripts.
fn spawn_pipeline_in(
    meta: Arc<Meta>,
    live: Arc<Mutex<Live>>,
    lives: Vec<Arc<Mutex<Live>>>,
    transcribe: Transcriber,
    store: Arc<Store>,
    limit_ms: u64,
) -> (Running, Input) {
    let (tx, rx) = mpsc::sync_channel(CAPTURE_QUEUE);
    let captured = Arc::new(AtomicU64::new(0));
    let (decode_tx, decode_rx) = mpsc::sync_channel(DECODE_QUEUE);
    let prep = {
        let (captured, live) = (Arc::clone(&captured), Arc::clone(&live));
        std::thread::spawn(move || run_prep(rx, captured, decode_tx, live, limit_ms))
    };
    let decoder = {
        let (live, meta) = (Arc::clone(&live), Arc::clone(&meta));
        std::thread::spawn(move || {
            run_decoder(decode_rx, transcribe, live, || {
                let record = build_record(&meta, &merge(&lives), now_ms(), false);
                if let Err(e) = store.save(&record) {
                    log::warn!("meeting checkpoint failed: {e}");
                }
            })
        })
    };
    (
        Running {
            meta,
            started: Instant::now(),
            live,
            prep,
            decoder,
        },
        Input { tx, captured },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixture: `secs` of 48 kHz audio with tone bursts at the given
    /// (start_s, len_s) spans and digital silence elsewhere.
    fn fixture(rate: u32, secs: f32, bursts: &[(f32, f32)]) -> Vec<f32> {
        let mut out = vec![0f32; (secs * rate as f32) as usize];
        for &(start, len) in bursts {
            let a = (start * rate as f32) as usize;
            let b = ((start + len) * rate as f32) as usize;
            for (i, v) in out[a..b].iter_mut().enumerate() {
                *v = 0.2 * (i as f32 * 440. * std::f32::consts::TAU / rate as f32).sin();
            }
        }
        out
    }

    fn temp_store() -> (Arc<Store>, PathBuf) {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "greta-meetings-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        (Arc::new(Store::new(dir.clone())), dir)
    }

    /// Like the audio callback's `send`, but waits for room so fixture runs
    /// are deterministic regardless of machine load.
    fn feed(input: &Input, samples: &[f32], rate: u32) {
        let offset = input
            .captured
            .fetch_add(samples.len() as u64, Ordering::SeqCst);
        input
            .tx
            .send(Block {
                offset,
                samples: samples.to_vec(),
                rate,
            })
            .unwrap();
    }

    fn echo_len() -> Transcriber {
        Box::new(|s: &[f32]| Ok(format!("speech {}", s.len())))
    }

    fn run_fixture(
        audio: &[f32],
        rate: u32,
        block: usize,
        transcribe: Transcriber,
        limit_ms: u64,
    ) -> MeetingRecord {
        let (store, dir) = temp_store();
        let meta = Arc::new(Meta::new(1_700_000_000_000));
        let live = Arc::new(Mutex::new(Live::default()));
        let (running, input) = spawn_pipeline(Arc::clone(&meta), live, transcribe, store, limit_ms);
        for chunk in audio.chunks(block) {
            feed(&input, chunk, rate);
        }
        drop(input);
        running.prep.join().unwrap();
        running.decoder.join().unwrap();
        let record = build_record(&meta, &running.live.lock_recover(), 1_700_000_100_000, true);
        let _ = std::fs::remove_dir_all(dir);
        record
    }

    #[test]
    fn segment_timestamps_come_from_sample_offsets() {
        let audio = fixture(48_000, 9.0, &[(1.0, 1.5), (5.0, 2.0)]);
        let record = run_fixture(&audio, 48_000, 512, echo_len(), MAX_MEETING_MS);
        assert_eq!(record.segments.len(), 2, "{:?}", record.segments);
        let [a, b] = [&record.segments[0], &record.segments[1]];
        // Each segment carries PAD (200 ms) of context on both sides.
        assert_eq!((a.start_ms, a.end_ms), (800, 2700));
        assert_eq!((b.start_ms, b.end_ms), (4800, 7200));
        assert_eq!(record.audio_ms, 9000);
        assert!(record.gaps.is_empty());
        assert_eq!(record.speech_ms, 1900 + 2400);
        assert_eq!(record.text, format!("{}\n{}", a.text, b.text));
    }

    #[test]
    fn continuous_speech_is_cut_without_losing_or_duplicating_samples() {
        let mut c = Chunker::default();
        let mut source: Vec<f32> = (0..(75 * RATE as usize))
            .map(|i| 0.1 + (i % 7) as f32 * 0.01)
            .collect();
        // A quiet (but voiced) dip lets the cutter choose a natural boundary.
        for v in &mut source[28 * RATE as usize..28 * RATE as usize + FRAME] {
            *v = 0.001;
        }
        let mut out = vec![];
        for chunk in source.chunks(1000) {
            out.extend(c.push(chunk));
        }
        out.extend(c.finish());
        assert!(out.len() >= 3);
        assert_eq!(out[0].0, 0);
        assert_eq!(out[0].1.len(), 28 * RATE as usize, "cut at the quiet frame");
        let mut next = 0;
        for (start, samples) in &out {
            assert_eq!(*start, next, "segments are contiguous");
            assert!(samples.len() <= MAX_SEGMENT);
            next += samples.len() as u64;
        }
        assert_eq!(next, source.len() as u64);
        let joined: Vec<f32> = out.into_iter().flat_map(|(_, s)| s).collect();
        assert_eq!(joined, source);
    }

    #[test]
    fn dropped_blocks_become_gaps_and_keep_the_timeline_aligned() {
        let (store, dir) = temp_store();
        let meta = Arc::new(Meta::new(1));
        let live = Arc::new(Mutex::new(Live::default()));
        let (running, input) =
            spawn_pipeline(meta, Arc::clone(&live), echo_len(), store, MAX_MEETING_MS);
        let audio = fixture(16_000, 8.0, &[(5.0, 1.0)]);
        // Simulate the callback losing 2 s (32000 samples) at t=1 s.
        feed(&input, &audio[..16_000], 16_000);
        input.captured.fetch_add(32_000, Ordering::SeqCst);
        for chunk in audio[48_000..].chunks(4000) {
            feed(&input, chunk, 16_000);
        }
        drop(input);
        running.prep.join().unwrap();
        running.decoder.join().unwrap();
        let live = live.lock_recover();
        assert_eq!(
            live.gaps,
            vec![Gap {
                start_ms: 1000,
                end_ms: 3000,
                reason: "audio dropped (capture queue full)".into(),
                source: AudioSource::Microphone,
            }]
        );
        assert_eq!(live.segments.len(), 1);
        assert_eq!(live.segments[0].start_ms, 4800);
        assert_eq!(live.timeline, 8 * 16_000);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn callback_send_never_blocks_and_counts_dropped_samples() {
        let (tx, rx) = mpsc::sync_channel(1);
        let input = Input {
            tx,
            captured: Arc::new(AtomicU64::new(0)),
        };
        input.send(vec![0.; 100], 16_000);
        input.send(vec![0.; 50], 16_000); // queue full: dropped, not blocked
        input.send(vec![0.; MAX_BLOCK + 1], 16_000); // oversized: dropped
        assert_eq!(
            input.captured.load(Ordering::SeqCst),
            150 + MAX_BLOCK as u64 + 1
        );
        let first = rx.try_recv().unwrap();
        assert_eq!((first.offset, first.samples.len()), (0, 100));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn slow_transcriber_produces_disclosed_gaps_not_unbounded_queues() {
        let mut bursts = vec![];
        for i in 0..12 {
            bursts.push((1.0 + i as f32 * 2.5, 0.5));
        }
        let audio = fixture(16_000, 32.0, &bursts);
        let slow: Transcriber = Box::new(|_| {
            std::thread::sleep(Duration::from_millis(400));
            Ok("x".into())
        });
        let record = run_fixture(&audio, 16_000, 16_000, slow, MAX_MEETING_MS);
        assert!(record
            .gaps
            .iter()
            .any(|g| g.reason == "transcriber fell behind"));
        assert_eq!(record.segments.len() + record.gaps.len(), 12);
        // Gaps and segments never overlap.
        for g in &record.gaps {
            assert!(record
                .segments
                .iter()
                .all(|s| s.end_ms <= g.start_ms || s.start_ms >= g.end_ms));
        }
    }

    #[test]
    fn failed_recognition_is_kept_as_a_gap_and_silence_is_not_decoded() {
        let audio = fixture(16_000, 6.0, &[(1.0, 1.0)]);
        let calls = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&calls);
        let failing: Transcriber = Box::new(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            Err("engine busy".into())
        });
        let record = run_fixture(&audio, 16_000, 1600, failing, MAX_MEETING_MS);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(record.segments.is_empty());
        assert_eq!(record.gaps[0].reason, "transcription failed");
        assert_eq!(
            (record.gaps[0].start_ms, record.gaps[0].end_ms),
            (800, 2200)
        );
    }

    #[test]
    fn meeting_length_is_bounded_and_disclosed() {
        let audio = fixture(16_000, 5.0, &[(0.5, 4.0)]);
        let record = run_fixture(&audio, 16_000, 1600, echo_len(), 2000);
        assert!(record.limit_reached);
        assert_eq!(record.audio_ms, 2000);
        assert_eq!(record.segments.len(), 1, "speech before the limit is kept");
        assert_eq!(
            (record.segments[0].start_ms, record.segments[0].end_ms),
            (300, 2000)
        );
    }

    #[test]
    fn controller_reserves_one_meeting_and_saves_on_finish() {
        let (store, dir) = temp_store();
        let c = Controller::default();
        assert!(!c.is_active());
        c.reserve().unwrap();
        assert!(c.reserve().is_err(), "only one meeting at a time");
        assert!(c.is_active());
        let mut sink = None;
        c.begin(
            |input| {
                sink = Some(input);
                Ok(16_000)
            },
            echo_len(),
            Arc::clone(&store),
        )
        .unwrap();
        let input = sink.take().unwrap();
        for chunk in fixture(16_000, 4.0, &[(1.0, 1.0)]).chunks(1600) {
            feed(&input, chunk, 16_000);
        }
        assert!(c.view().active);
        let record = c.finish(|| drop(input), &store).unwrap();
        assert!(!c.is_active());
        assert_eq!(record.segments.len(), 1);
        assert!(record.complete);
        assert_eq!(
            store.get(&record.id).unwrap().unwrap().segments,
            record.segments
        );
        assert_eq!(store.list()[0].id, record.id);
        assert!(c.finish(|| {}, &store).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_microphone_open_releases_the_slot() {
        let (store, dir) = temp_store();
        let c = Controller::default();
        c.reserve().unwrap();
        let err = c
            .begin(|_| Err("no mic".into()), echo_len(), store)
            .unwrap_err();
        assert_eq!(err, "no mic");
        assert!(!c.is_active());
        assert_eq!(c.view().error.as_deref(), Some("no mic"));
        c.reserve().unwrap();
        assert!(c.is_active(), "the slot is reusable after a failed start");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn store_rejects_path_ids_and_lists_newest_first() {
        let (store, dir) = temp_store();
        for bad in ["", "m", "../m1", "m1/../../x", "x123", "m12a"] {
            assert!(!valid_id(bad), "{bad}");
            assert!(store.get(bad).is_err());
            assert!(store.delete(bad).is_err());
        }
        let live = Live::default();
        for started in [1000, 3000, 2000] {
            store
                .save(&build_record(&Meta::new(started), &live, started + 1, true))
                .unwrap();
        }
        let ids: Vec<_> = store.list().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["m3000", "m2000", "m1000"]);
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join("m1000.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        store.delete("m2000").unwrap();
        store.delete("m2000").unwrap();
        assert_eq!(store.list().len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    struct LateStream {
        _sink: Option<crate::audio::Sink>,
        dropped: Arc<std::sync::atomic::AtomicBool>,
    }
    impl Drop for LateStream {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    fn slow_recorder(
        delay: Duration,
        succeed: bool,
    ) -> (crate::audio::Recorder, Arc<std::sync::atomic::AtomicBool>) {
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&dropped);
        let recorder = crate::audio::Recorder::spawn_with(
            move |_, sink| {
                std::thread::sleep(delay);
                if !succeed {
                    return Err("device vanished".to_string());
                }
                Ok((
                    LateStream {
                        _sink: sink,
                        dropped: Arc::clone(&flag),
                    },
                    16_000,
                ))
            },
            Duration::from_millis(50),
        );
        (recorder, dropped)
    }

    fn wait_until(cond: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        cond()
    }

    #[test]
    fn late_microphone_open_after_timeout_is_dropped_and_releases_the_slot() {
        let (store, dir) = temp_store();
        let (recorder, dropped) = slow_recorder(Duration::from_millis(300), true);
        let c = Controller::default();
        c.reserve().unwrap();
        let began = Instant::now();
        let err = c
            .begin(|input| recorder.start_meeting(input), echo_len(), store)
            .unwrap_err();
        assert_eq!(err, "microphone did not respond");
        assert!(
            began.elapsed() < Duration::from_millis(250),
            "begin must not wait for the late open"
        );
        assert!(!c.is_active());
        assert!(
            wait_until(|| dropped.load(Ordering::SeqCst)),
            "late stream and sink must be dropped"
        );
        c.reserve().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn late_microphone_failure_releases_the_slot() {
        let (store, dir) = temp_store();
        let (recorder, _) = slow_recorder(Duration::from_millis(200), false);
        let c = Controller::default();
        c.reserve().unwrap();
        assert!(c
            .begin(|input| recorder.start_meeting(input), echo_len(), store)
            .is_err());
        assert!(!c.is_active());
        let (store, dir2) = temp_store();
        let (fast, _) = slow_recorder(Duration::ZERO, true);
        std::thread::sleep(Duration::from_millis(250));
        c.reserve().unwrap();
        c.begin(
            |input| fast.start_meeting(input),
            echo_len(),
            Arc::clone(&store),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(dir2);
    }

    #[test]
    fn stop_queued_during_a_slow_start_closes_the_stream() {
        let (recorder, dropped) = slow_recorder(Duration::from_millis(100), true);
        let (_running, input) = spawn_pipeline(
            Arc::new(Meta::new(now_ms())),
            Arc::new(Mutex::new(Live::default())),
            echo_len(),
            temp_store().0,
            MAX_MEETING_MS,
        );
        let handle = std::thread::scope(|s| {
            let started = s.spawn(|| recorder.start_meeting(input));
            std::thread::sleep(Duration::from_millis(20));
            let _ = recorder.stop();
            started.join().unwrap()
        });
        assert!(handle.is_err());
        assert!(
            dropped.load(Ordering::SeqCst),
            "stop after a late open closes the stream"
        );
    }

    fn tagged(record: &MeetingRecord) -> Vec<(AudioSource, u64, u64)> {
        record
            .segments
            .iter()
            .map(|s| (s.source, s.start_ms, s.end_ms))
            .collect()
    }

    #[test]
    fn meeting_two_sources_merge_by_start_with_per_source_gaps() {
        let (store, dir) = temp_store();
        let meta = Arc::new(Meta::with_source(1, SourceMode::Both));
        let mic_live = Arc::new(Mutex::new(Live::for_source(AudioSource::Microphone)));
        let sys_live = Arc::new(Mutex::new(Live::for_source(AudioSource::System)));
        let lives = vec![Arc::clone(&mic_live), Arc::clone(&sys_live)];
        let spawn = |live: &Arc<Mutex<Live>>| {
            spawn_pipeline_in(
                Arc::clone(&meta),
                Arc::clone(live),
                lives.clone(),
                echo_len(),
                Arc::clone(&store),
                MAX_MEETING_MS,
            )
        };
        let (mic, mic_in) = spawn(&mic_live);
        let (sys, sys_in) = spawn(&sys_live);
        for chunk in fixture(48_000, 8.0, &[(1.0, 1.0), (5.0, 1.0)]).chunks(4800) {
            feed(&mic_in, chunk, 48_000);
        }
        let sys_audio = fixture(16_000, 8.0, &[(1.0, 1.0), (3.0, 1.0)]);
        for chunk in sys_audio[..96_000].chunks(1600) {
            feed(&sys_in, chunk, 16_000);
        }
        // System loses 1 s at t=6 s; the gap is tagged with its source.
        sys_in.captured.fetch_add(16_000, Ordering::SeqCst);
        feed(&sys_in, &sys_audio[112_000..], 16_000);
        drop((mic_in, sys_in));
        for running in [mic, sys] {
            running.prep.join().unwrap();
            running.decoder.join().unwrap();
        }
        let record = build_record(&meta, &merge(&lives), 2, true);
        use AudioSource::{Microphone as M, System as S};
        assert_eq!(
            tagged(&record),
            vec![
                (M, 800, 2200),
                (S, 800, 2200),
                (S, 2800, 4200),
                (M, 4800, 6200)
            ],
            "ordered by start, microphone first on ties"
        );
        assert_eq!(record.source, "both");
        assert_eq!(record.audio_ms, 8000);
        assert_eq!(
            record.gaps,
            vec![Gap {
                start_ms: 6000,
                end_ms: 7000,
                reason: "audio dropped (capture queue full)".into(),
                source: S,
            }]
        );
        let json = serde_json::to_value(&record.segments[1]).unwrap();
        assert_eq!(json["source"], "system");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn meeting_old_records_default_to_microphone_source() {
        let seg: Segment =
            serde_json::from_str(r#"{"start_ms":1,"end_ms":2,"text":"hi"}"#).unwrap();
        assert_eq!(seg.source, AudioSource::Microphone);
        let gap: Gap = serde_json::from_str(r#"{"start_ms":1,"end_ms":2,"reason":"x"}"#).unwrap();
        assert_eq!(gap.source, AudioSource::Microphone);
        assert_eq!(SourceMode::parse(None), Ok(SourceMode::Microphone));
        assert_eq!(SourceMode::parse(Some("both")), Ok(SourceMode::Both));
        assert!(SourceMode::parse(Some("speaker")).is_err());
    }

    /// Writes a python3 helper that speaks the `greta-system-audio` protocol.
    /// Returns None when python3 is unavailable.
    fn fake_helper(dir: &Path, body: &str) -> Option<std::process::Command> {
        let ok = std::process::Command::new("python3")
            .arg("-c")
            .arg("pass")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            eprintln!("python3 unavailable; skipping helper process test");
            return None;
        }
        std::fs::create_dir_all(dir).unwrap();
        let script = dir.join("helper.py");
        let prelude = "import sys, json, math, struct\n\
            out = sys.stdout.buffer\n\
            def frame(b):\n    out.write(struct.pack('<I', len(b)) + b); out.flush()\n";
        std::fs::write(&script, format!("{prelude}{body}")).unwrap();
        let mut command = std::process::Command::new("python3");
        command.arg("-I").arg(script);
        Some(command)
    }

    const TONE_HELPER: &str = "rate = 16000\n\
        frame(json.dumps({'sample_rate': rate}).encode())\n\
        def voiced(t):\n    return 1.0 <= t < 2.0 or 4.0 <= t < 4.5\n\
        samples = [0.2 * math.sin(i * 440 * 2 * math.pi / rate) if voiced(i / rate) else 0.0\n\
                   for i in range(6 * rate)]\n\
        for i in range(0, len(samples), 1600):\n\
        \x20   chunk = samples[i:i + 1600]\n\
        \x20   frame(struct.pack('<%df' % len(chunk), *chunk))\n\
        sys.stdin.read()\n";

    fn open_helper(command: std::process::Command) -> SystemOpen<'static> {
        Box::new(move |input| {
            crate::system_audio::Capture::spawn(command, input, Duration::from_secs(10))
        })
    }

    #[test]
    fn meeting_system_audio_from_real_helper_process_is_tagged_system() {
        let (store, dir) = temp_store();
        let Some(helper) = fake_helper(&dir.join("helper"), TONE_HELPER) else {
            return;
        };
        let c = Controller::default();
        c.reserve().unwrap();
        c.begin_sources(
            SourceMode::System,
            Box::new(|_| panic!("microphone must not open for system-only")),
            open_helper(helper),
            echo_len,
            Arc::clone(&store),
        )
        .unwrap();
        assert_eq!(c.view().source, "system");
        assert!(
            wait_until(|| c.view().segment_count >= 2),
            "helper frames processed"
        );
        let record = c.finish(|| {}, &store).unwrap();
        use AudioSource::System as S;
        assert_eq!(tagged(&record), vec![(S, 800, 2200), (S, 3800, 4700)]);
        assert_eq!(record.source, "system");
        assert_eq!(record.audio_ms, 6000);
        assert!(record.gaps.is_empty(), "{:?}", record.gaps);
        assert!(!c.is_active());
        let _ = std::fs::remove_dir_all(dir);
    }

    const ERROR_HELPER: &str =
        "frame(json.dumps({'error': 'System audio capture requires macOS 14.2 or later'}).encode())\n";

    #[test]
    fn meeting_system_only_start_fails_with_the_helper_reason() {
        let (store, dir) = temp_store();
        let Some(helper) = fake_helper(&dir.join("helper"), ERROR_HELPER) else {
            return;
        };
        let c = Controller::default();
        c.reserve().unwrap();
        let err = c
            .begin_sources(
                SourceMode::System,
                Box::new(|_| panic!("microphone must not open")),
                open_helper(helper),
                echo_len,
                Arc::clone(&store),
            )
            .unwrap_err();
        assert_eq!(err, "System audio capture requires macOS 14.2 or later");
        assert!(!c.is_active());
        c.reserve().unwrap();
        let err = c
            .begin_sources(
                SourceMode::System,
                Box::new(|_| unreachable!()),
                open_helper(std::process::Command::new(dir.join("missing-helper"))),
                echo_len,
                store,
            )
            .unwrap_err();
        assert!(err.contains("couldn't start system audio helper"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn meeting_both_mode_continues_with_microphone_when_system_fails() {
        let (store, dir) = temp_store();
        let Some(helper) = fake_helper(&dir.join("helper"), ERROR_HELPER) else {
            return;
        };
        let c = Controller::default();
        c.reserve().unwrap();
        let mut sink = None;
        c.begin_sources(
            SourceMode::Both,
            Box::new(|input| {
                sink = Some(input);
                Ok(16_000)
            }),
            open_helper(helper),
            echo_len,
            Arc::clone(&store),
        )
        .unwrap();
        let input = sink.take().unwrap();
        assert!(c.view().error.unwrap().contains("macOS 14.2"));
        for chunk in fixture(16_000, 4.0, &[(1.0, 1.0)]).chunks(1600) {
            feed(&input, chunk, 16_000);
        }
        let record = c.finish(|| drop(input), &store).unwrap();
        assert_eq!(tagged(&record), vec![(AudioSource::Microphone, 800, 2200)]);
        assert_eq!(record.gaps.len(), 1);
        let gap = &record.gaps[0];
        assert_eq!(gap.source, AudioSource::System);
        assert_eq!((gap.start_ms, gap.end_ms), (0, 4000));
        assert!(gap.reason.contains("macOS 14.2"), "{}", gap.reason);
        let _ = std::fs::remove_dir_all(dir);
    }
}
