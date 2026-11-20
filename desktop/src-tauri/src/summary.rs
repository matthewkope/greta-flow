//! Local, grounded meeting summaries.
//!
//! The model runs in a separate helper process (`greta-summarizer`, MLX on
//! Apple Silicon). This module validates the model folder, checks it against
//! the pinned manifest, downloads it on explicit request, and speaks the
//! helper's framed JSON protocol: a 4-byte little-endian length followed by
//! JSON, once in each direction. Output is untrusted; callers must pass it
//! through `meeting_notes::validate_model_grounding` before showing it.
use crate::locks::MutexExt;
use crate::meeting::Segment;
use crate::meeting_notes::{FailedChunk, ModelItem};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

pub const HELPER_NAME: &str = "greta-summarizer";
pub const MODEL_DIR_NAME: &str = "qwen3-4b-instruct-2507-4bit-mlx";
pub const MAX_RESPONSE_BYTES: u32 = 1024 * 1024;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_RUN_TIME: Duration = Duration::from_secs(45 * 60);
pub const MAX_TOKENS: u32 = 1024;
/// Transcript characters per map step; keeps prompts well inside the context.
const CHUNK_CHARS: usize = 12_000;
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_INDEX_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SummaryState {
    None,
    Missing,
    Invalid,
    Unsupported,
    Ready,
    Loading,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SummaryModelInfo {
    pub path: String,
    pub model_id: String,
    pub pinned: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SummaryRuntimeStatus {
    pub state: SummaryState,
    pub path: Option<String>,
    pub model: Option<SummaryModelInfo>,
    pub detail: String,
}

// ---------------------------------------------------------------- manifest

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub repo: String,
    pub revision: String,
    pub files: BTreeMap<String, ManifestFile>,
}

#[derive(Debug, Deserialize)]
pub struct ManifestFile {
    pub size: u64,
    #[serde(default)]
    pub sha256: Option<String>,
    /// Git blob id: sha1("blob <len>\0" + bytes).
    #[serde(default)]
    pub git_sha1: Option<String>,
}

pub fn manifest() -> Manifest {
    #[allow(clippy::expect_used)]
    serde_json::from_str(include_str!("summary-model.json")).expect("valid summary manifest")
}

impl Manifest {
    pub fn total_bytes(&self) -> u64 {
        self.files.values().map(|f| f.size).sum()
    }
    pub fn label(&self) -> String {
        format!("{}@{}", self.repo, self.revision)
    }
}

/// Minimal SHA-1 (FIPS 180-4), used only to compare git blob ids.
struct Sha1 {
    h: [u32; 5],
    buf: Vec<u8>,
    len: u64,
}
impl Sha1 {
    fn new() -> Self {
        Self {
            h: [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0],
            buf: Vec::with_capacity(64),
            len: 0,
        }
    }
    fn update(&mut self, mut data: &[u8]) {
        self.len += data.len() as u64;
        if !self.buf.is_empty() {
            let take = (64 - self.buf.len()).min(data.len());
            self.buf.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.buf.len() == 64 {
                let block: [u8; 64] = self.buf[..].try_into().unwrap_or([0; 64]);
                self.block(&block);
                self.buf.clear();
            }
        }
        while data.len() >= 64 {
            let block: [u8; 64] = data[..64].try_into().unwrap_or([0; 64]);
            self.block(&block);
            data = &data[64..];
        }
        self.buf.extend_from_slice(data);
    }
    fn block(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 80];
        for (i, chunk) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*chunk);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = self.h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (h, v) in self.h.iter_mut().zip([a, b, c, d, e]) {
            *h = h.wrapping_add(v);
        }
    }
    fn finish(mut self) -> String {
        let bits = self.len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.buf.len() != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        let bytes: Vec<u8> = self.h.iter().flat_map(|h| h.to_be_bytes()).collect();
        crate::model::hex_lower(&bytes)
    }
}

/// Git blob id of a file, streamed.
pub fn git_blob_sha1(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    let mut sha = Sha1::new();
    sha.update(format!("blob {len}\0").as_bytes());
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        sha.update(&buf[..n]);
    }
    Ok(sha.finish())
}

/// Check one file's size and pinned hash.
pub fn verify_file(path: &Path, expected: &ManifestFile) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() || meta.len() != expected.size {
        return Err(format!(
            "{}: expected {} bytes, found {}",
            path.display(),
            expected.size,
            meta.len()
        ));
    }
    let (actual, wanted) = match (&expected.sha256, &expected.git_sha1) {
        (Some(sha256), _) => (crate::model::file_sha256(path)?, sha256),
        (None, Some(sha1)) => (git_blob_sha1(path)?, sha1),
        (None, None) => return Err("manifest entry has no hash".into()),
    };
    if actual.eq_ignore_ascii_case(wanted) {
        Ok(())
    } else {
        Err(format!("{}: hash mismatch", path.display()))
    }
}

pub fn verify_manifest(dir: &Path, manifest: &Manifest) -> Result<(), String> {
    for (name, file) in &manifest.files {
        verify_file(&dir.join(name), file)?;
    }
    Ok(())
}

// Hashing 2.3 GB on every status poll is too slow; remember the result per
// (path, sizes, mtimes) fingerprint.
static PINNED_CACHE: Mutex<Option<(String, bool)>> = Mutex::new(None);

fn fingerprint(dir: &Path, manifest: &Manifest) -> String {
    let mut out = dir.display().to_string();
    for name in manifest.files.keys() {
        let meta = std::fs::metadata(dir.join(name)).ok();
        let mtime = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        out.push_str(&format!("|{name}:{:?}:{mtime}", meta.map(|m| m.len())));
    }
    out
}

fn is_pinned(dir: &Path) -> bool {
    let manifest = manifest();
    // Cheap reject before hashing anything.
    if manifest
        .files
        .iter()
        .any(|(n, f)| std::fs::metadata(dir.join(n)).map(|m| m.len()).ok() != Some(f.size))
    {
        return false;
    }
    let key = fingerprint(dir, &manifest);
    if let Some((cached, pinned)) = PINNED_CACHE.lock_recover().as_ref() {
        if *cached == key {
            return *pinned;
        }
    }
    let pinned = verify_manifest(dir, &manifest).is_ok();
    *PINNED_CACHE.lock_recover() = Some((key, pinned));
    pinned
}

// --------------------------------------------------------------- validation

fn contained_file(root: &Path, name: &str) -> Result<PathBuf, String> {
    let path = root.join(name);
    let resolved = path.canonicalize().map_err(|_| format!("missing {name}"))?;
    if !resolved.starts_with(root) {
        return Err(format!("{name} resolves outside the model folder"));
    }
    if !resolved.is_file() {
        return Err(format!("{name} is not a file"));
    }
    Ok(resolved)
}

fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let mut bytes = vec![];
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err(format!("{} is too large", path.display()));
    }
    Ok(bytes)
}

/// Structural check of an MLX model folder: config.json, tokenizer.json and
/// either model.safetensors or an index whose every shard is present, all
/// resolving inside the folder.
pub fn validate_model_dir(path: &Path) -> Result<SummaryModelInfo, String> {
    let meta = std::fs::metadata(path).map_err(|_| "model folder not found".to_string())?;
    if !meta.is_dir() {
        return Err("not a folder; choose the model directory".into());
    }
    let root = path.canonicalize().map_err(|e| e.to_string())?;
    let config = contained_file(&root, "config.json")?;
    let config: serde_json::Value =
        serde_json::from_slice(&read_bounded(&config, MAX_CONFIG_BYTES)?)
            .map_err(|e| format!("config.json: {e}"))?;
    let model_type = config["model_type"]
        .as_str()
        .ok_or("config.json has no model_type")?
        .to_string();
    contained_file(&root, "tokenizer.json")?;
    let single = root.join("model.safetensors");
    if std::fs::symlink_metadata(&single).is_ok() {
        contained_file(&root, "model.safetensors")?;
    } else {
        let index = contained_file(&root, "model.safetensors.index.json")
            .map_err(|_| "missing model.safetensors (or its index)".to_string())?;
        let index: serde_json::Value =
            serde_json::from_slice(&read_bounded(&index, MAX_INDEX_BYTES)?)
                .map_err(|e| format!("model.safetensors.index.json: {e}"))?;
        let map = index["weight_map"]
            .as_object()
            .ok_or("model.safetensors.index.json has no weight_map")?;
        let shards: std::collections::BTreeSet<&str> =
            map.values().filter_map(|v| v.as_str()).collect();
        if shards.is_empty() {
            return Err("model.safetensors.index.json lists no shards".into());
        }
        for shard in shards {
            contained_file(&root, shard)?;
        }
    }
    let dir_name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(SummaryModelInfo {
        path: root.display().to_string(),
        model_id: format!("{model_type}/{dir_name}"),
        pinned: is_pinned(&root),
    })
}

// ------------------------------------------------------------------ status

/// Last helper run outcome, reflected in `runtime_status`.
static RUN: Mutex<Option<(SummaryState, String)>> = Mutex::new(None);

static ACTIVE_RUN: Mutex<Option<String>> = Mutex::new(None);

pub fn set_run_state(state: Option<(SummaryState, String)>) {
    *RUN.lock_recover() = state;
}

/// Exclusive claim on the summary helper for one meeting. Dropping it clears
/// a still-`Loading` run state, so every exit path frees the slot.
pub struct RunClaim(());

pub fn claim_run(id: &str) -> Result<RunClaim, String> {
    let mut active = ACTIVE_RUN.lock_recover();
    if active.is_some() {
        return Err("A summary is already being generated.".into());
    }
    *active = Some(id.to_string());
    set_run_state(Some((
        SummaryState::Loading,
        "Generating a summary…".into(),
    )));
    Ok(RunClaim(()))
}

impl Drop for RunClaim {
    fn drop(&mut self) {
        let mut run = RUN.lock_recover();
        if matches!(run.as_ref(), Some((SummaryState::Loading, _))) {
            *run = None;
        }
        drop(run);
        *ACTIVE_RUN.lock_recover() = None;
    }
}

/// True while a helper run for meeting `id` is live in this process.
pub fn run_active_for(id: &str) -> bool {
    ACTIVE_RUN.lock_recover().as_deref() == Some(id)
}

pub fn helper_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("GRETA_SUMMARIZER_HELPER") {
        return Ok(path.into());
    }
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let bundled = executable
        .parent()
        .ok_or("missing executable parent")?
        .join("../Resources/parakeet")
        .join(HELPER_NAME);
    if bundled.is_file() {
        return Ok(bundled);
    }
    if cfg!(debug_assertions) {
        let development = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../parakeet/staging")
            .join(HELPER_NAME);
        if development.is_file() {
            return Ok(development);
        }
    }
    Err("The summarizer helper is not in this build.".into())
}

pub fn runtime_status(cfg: &crate::config::Config) -> SummaryRuntimeStatus {
    let status = |state, path: Option<String>, model, detail: &str| SummaryRuntimeStatus {
        state,
        path,
        model,
        detail: detail.into(),
    };
    let Some(path) = cfg.summary_model.clone().filter(|p| !p.is_empty()) else {
        return status(
            SummaryState::None,
            None,
            None,
            "No summary model is set up. Download the pinned model or choose a local MLX model folder.",
        );
    };
    if std::fs::symlink_metadata(&path).is_err() {
        return status(
            SummaryState::Missing,
            Some(path),
            None,
            "The summary model folder no longer exists.",
        );
    }
    let model = match validate_model_dir(Path::new(&path)) {
        Ok(model) => model,
        Err(e) => {
            return status(
                SummaryState::Invalid,
                Some(path),
                None,
                &format!("Model folder is not usable: {e}"),
            )
        }
    };
    if !(cfg!(target_os = "macos") && cfg!(target_arch = "aarch64")) {
        return status(
            SummaryState::Unsupported,
            Some(path),
            Some(model),
            "Local summaries need an Apple Silicon Mac.",
        );
    }
    if let Err(e) = helper_path() {
        return status(SummaryState::Unsupported, Some(path), Some(model), &e);
    }
    if let Some((state, detail)) = RUN.lock_recover().clone() {
        return status(state, Some(path), Some(model), &detail);
    }
    let detail = if model.pinned {
        "Ready (pinned model, hashes verified).".to_string()
    } else {
        "Ready (custom model folder; not the pinned build).".to_string()
    };
    status(SummaryState::Ready, Some(path), Some(model), &detail)
}

// ---------------------------------------------------------------- download

pub fn models_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache/gretchen-flow/models")
}

pub fn default_model_dir() -> PathBuf {
    models_dir().join(MODEL_DIR_NAME)
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct DownloadState {
    pub active: bool,
    pub bytes: u64,
    pub total: u64,
    pub error: Option<String>,
}

static DOWNLOAD: Mutex<DownloadState> = Mutex::new(DownloadState {
    active: false,
    bytes: 0,
    total: 0,
    error: None,
});

pub fn download_state() -> DownloadState {
    DOWNLOAD.lock_recover().clone()
}

/// Claim the download slot; Err if one is already running.
pub fn begin_download() -> Result<(), String> {
    let mut state = DOWNLOAD.lock_recover();
    if state.active {
        return Err("A summary model download is already running.".into());
    }
    *state = DownloadState {
        active: true,
        bytes: 0,
        total: manifest().total_bytes(),
        error: None,
    };
    Ok(())
}

pub fn end_download(result: &Result<PathBuf, String>) {
    let mut state = DOWNLOAD.lock_recover();
    state.active = false;
    state.error = result.as_ref().err().cloned();
}

/// Fetch every pinned file into `dest`, verifying size and hash before an
/// atomic rename. Files already present and verified are kept.
pub fn download_pinned(dest: &Path) -> Result<PathBuf, String> {
    let manifest = manifest();
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(30))
        .timeout_read(Duration::from_secs(120))
        .build();
    let mut done = 0u64;
    for (name, file) in &manifest.files {
        let path = dest.join(name);
        if verify_file(&path, file).is_ok() {
            done += file.size;
            DOWNLOAD.lock_recover().bytes = done;
            continue;
        }
        let url = format!(
            "https://huggingface.co/{}/resolve/{}/{name}",
            manifest.repo, manifest.revision
        );
        log::info!("downloading summary model file {name}");
        let response = agent.get(&url).call().map_err(|e| e.to_string())?;
        let temp = dest.join(format!("{name}.download-part"));
        let mut out = std::fs::File::create(&temp).map_err(|e| e.to_string())?;
        let base = done;
        let result = crate::model::download(response, &mut out, &path, name, &mut |p| {
            DOWNLOAD.lock_recover().bytes = base + p.downloaded_bytes;
        })
        .and_then(|_| out.sync_all().map_err(|e| e.to_string()))
        .and_then(|_| verify_file(&temp, file));
        drop(out);
        if let Err(e) = result {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("{name}: {e}"));
        }
        std::fs::rename(&temp, &path).map_err(|e| e.to_string())?;
        done += file.size;
        DOWNLOAD.lock_recover().bytes = done;
    }
    validate_model_dir(dest)?;
    Ok(dest.to_path_buf())
}

// --------------------------------------------------------------------- IPC

#[derive(Serialize)]
struct Request<'a> {
    model_dir: &'a str,
    segments: Vec<RequestSegment<'a>>,
    max_tokens: u32,
}

#[derive(Serialize)]
struct RequestSegment<'a> {
    index: usize,
    start_ms: u64,
    text: &'a str,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawOutput {
    #[serde(default)]
    pub bullets: Vec<ModelItem>,
    #[serde(default)]
    pub action_items: Vec<ModelItem>,
    #[serde(skip)]
    pub chunks: usize,
    #[serde(skip)]
    pub failed_chunks: Vec<FailedChunk>,
}

#[derive(Deserialize)]
struct Reply {
    ok: bool,
    #[serde(default)]
    output: Option<RawOutput>,
    #[serde(default)]
    error: Option<String>,
}

fn read_frame(reader: &mut impl Read) -> Result<Vec<u8>, String> {
    let mut len = [0u8; 4];
    reader
        .read_exact(&mut len)
        .map_err(|_| "summarizer closed without a reply".to_string())?;
    let len = u32::from_le_bytes(len);
    if len > MAX_RESPONSE_BYTES {
        return Err(format!(
            "summarizer reply too large ({len} bytes; limit {MAX_RESPONSE_BYTES})"
        ));
    }
    let mut body = vec![0u8; len as usize];
    reader
        .read_exact(&mut body)
        .map_err(|_| "summarizer reply was truncated".to_string())?;
    Ok(body)
}

fn request_body(model_dir: &str, chunk: &[(usize, &Segment)]) -> Result<Vec<u8>, String> {
    let request = Request {
        model_dir,
        segments: chunk
            .iter()
            .map(|(index, s)| RequestSegment {
                index: *index,
                start_ms: s.start_ms,
                text: &s.text,
            })
            .collect(),
        max_tokens: MAX_TOKENS,
    };
    serde_json::to_vec(&request).map_err(|e| e.to_string())
}

fn parse_reply(frame: &[u8]) -> Result<RawOutput, String> {
    let reply: Reply =
        serde_json::from_slice(frame).map_err(|e| format!("malformed summarizer reply: {e}"))?;
    if !reply.ok {
        return Err(reply
            .error
            .unwrap_or_else(|| "summarizer failed without a reason".into()));
    }
    reply
        .output
        .ok_or_else(|| "summarizer reply has no output".into())
}

fn exit_reason(child: &mut Child, err_rx: &mpsc::Receiver<String>, e: String) -> String {
    let status = match child.wait() {
        Ok(status) => status.to_string(),
        Err(e) => e.to_string(),
    };
    let stderr = err_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap_or_default();
    let tail: String = stderr
        .trim()
        .chars()
        .rev()
        .take(400)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if tail.is_empty() {
        format!("{e} ({status})")
    } else {
        format!("{e} ({status}): {tail}")
    }
}

/// Run every chunk through one helper process (the model loads once). Each
/// chunk's reply is independent; `per_chunk` scales one overall deadline.
fn run_chunks(
    helper: &Path,
    model_dir: &str,
    chunks: &[Vec<(usize, &Segment)>],
    per_chunk: Duration,
) -> Result<Vec<Result<RawOutput, String>>, String> {
    let bodies = chunks
        .iter()
        .map(|c| request_body(model_dir, c))
        .collect::<Result<Vec<_>, _>>()?;
    let mut child = Command::new(helper)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't start summarizer {}: {e}", helper.display()))?;
    let mut stdin = child.stdin.take().ok_or("missing summarizer stdin")?;
    let mut stdout = child.stdout.take().ok_or("missing summarizer stdout")?;
    let stderr = child.stderr.take().ok_or("missing summarizer stderr")?;
    // Writes may block if the helper stops reading; never block the caller.
    std::thread::spawn(move || {
        for body in bodies {
            if stdin
                .write_all(&(body.len() as u32).to_le_bytes())
                .and_then(|()| stdin.write_all(&body))
                .and_then(|()| stdin.flush())
                .is_err()
            {
                break;
            }
        }
    });
    let (err_tx, err_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.take(64 * 1024).read_to_string(&mut text);
        let _ = err_tx.send(text);
    });
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || loop {
        let frame = read_frame(&mut stdout);
        let done = frame.is_err();
        if tx.send(frame).is_err() || done {
            break;
        }
    });
    let budget = per_chunk
        .saturating_mul(chunks.len() as u32)
        .min(MAX_RUN_TIME);
    let deadline = Instant::now() + budget;
    let mut results = Vec::with_capacity(chunks.len());
    let mut fatal: Option<String> = None;
    for _ in chunks {
        if let Some(reason) = &fatal {
            results.push(Err(reason.clone()));
            continue;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining) {
            Ok(Ok(frame)) => results.push(parse_reply(&frame)),
            Ok(Err(e)) => {
                let _ = child.kill();
                let reason = exit_reason(&mut child, &err_rx, e);
                results.push(Err(reason.clone()));
                fatal = Some(reason);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let reason = format!(
                    "summarizer timed out after {}s and was stopped",
                    budget.as_secs()
                );
                results.push(Err(reason.clone()));
                fatal = Some(reason);
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    Ok(results)
}

/// Split segments into chunks by character budget, keeping global indices.
fn chunks(segments: &[Segment]) -> Vec<Vec<(usize, &Segment)>> {
    let mut out: Vec<Vec<(usize, &Segment)>> = vec![];
    let mut size = 0;
    for (i, s) in segments.iter().enumerate() {
        if s.text.trim().is_empty() {
            continue;
        }
        if out.is_empty() || (size + s.text.len() > CHUNK_CHARS && size > 0) {
            out.push(vec![]);
            size = 0;
        }
        size += s.text.len();
        if let Some(last) = out.last_mut() {
            last.push((i, s));
        }
    }
    out
}

/// Map step per chunk; successful chunks are merged by concatenation and
/// failed chunks are recorded. Fails only when every chunk fails.
pub fn summarize(
    helper: &Path,
    model_dir: &str,
    segments: &[Segment],
    timeout: Duration,
) -> Result<RawOutput, String> {
    let chunks = chunks(segments);
    if chunks.is_empty() {
        return Err("This meeting has no transcript to summarize.".into());
    }
    let results = run_chunks(helper, model_dir, &chunks, timeout)?;
    let mut merged = RawOutput {
        chunks: results.len(),
        ..RawOutput::default()
    };
    for (index, result) in results.into_iter().enumerate() {
        match result {
            Ok(out) => {
                merged.bullets.extend(out.bullets);
                merged.action_items.extend(out.action_items);
            }
            Err(reason) => merged.failed_chunks.push(FailedChunk { index, reason }),
        }
    }
    if merged.failed_chunks.len() == merged.chunks {
        return Err(if merged.chunks == 1 {
            merged.failed_chunks.remove(0).reason
        } else {
            merged
                .failed_chunks
                .iter()
                .map(|f| format!("section {}: {}", f.index + 1, f.reason))
                .collect::<Vec<_>>()
                .join("; ")
        });
    }
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting_notes::validate_model_grounding;
    use std::os::unix::fs::PermissionsExt;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gf-summary-{tag}-{}-{}",
            std::process::id(),
            crate::meeting::now_ms()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn model_fixture(tag: &str) -> PathBuf {
        let dir = temp_dir(tag);
        std::fs::write(dir.join("config.json"), r#"{"model_type":"qwen3"}"#).unwrap();
        std::fs::write(dir.join("tokenizer.json"), "{}").unwrap();
        std::fs::write(dir.join("model.safetensors"), "w").unwrap();
        dir
    }

    #[test]
    fn sha1_blob_matches_git() {
        let dir = temp_dir("sha1");
        let empty = dir.join("empty");
        std::fs::write(&empty, "").unwrap();
        assert_eq!(
            git_blob_sha1(&empty).unwrap(),
            "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
        );
        // `printf 'hello world\n' | git hash-object --stdin`
        let hello = dir.join("hello");
        std::fs::write(&hello, "hello world\n").unwrap();
        assert_eq!(
            git_blob_sha1(&hello).unwrap(),
            "3b18e512dba79e4c8300dd08aeb37f8e728b8dad"
        );
        // Multi-block input crosses the 64-byte buffering paths.
        let mut sha = Sha1::new();
        sha.update(b"abcdbcdecdefdefgefghfghighijhi");
        sha.update(b"jkijkljklmklmnlmnomnopnopq");
        assert_eq!(sha.finish(), "84983e441c3bd26ebaae4aa1f95129e5e54670f1");
    }

    #[test]
    fn manifest_verification_passes_and_fails() {
        let dir = temp_dir("manifest");
        std::fs::write(dir.join("a.json"), "").unwrap();
        std::fs::write(dir.join("b.bin"), "abc").unwrap();
        let m: Manifest = serde_json::from_value(serde_json::json!({
            "repo": "r", "revision": "v",
            "files": {
                "a.json": {"size": 0, "git_sha1": "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"},
                "b.bin": {"size": 3, "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}
            }
        }))
        .unwrap();
        verify_manifest(&dir, &m).unwrap();
        std::fs::write(dir.join("b.bin"), "abd").unwrap();
        assert!(verify_manifest(&dir, &m).unwrap_err().contains("hash"));
        std::fs::write(dir.join("b.bin"), "abcd").unwrap();
        assert!(verify_manifest(&dir, &m).unwrap_err().contains("bytes"));
        std::fs::remove_file(dir.join("a.json")).unwrap();
        assert!(verify_manifest(&dir, &m).is_err());
    }

    #[test]
    fn real_manifest_parses_and_is_pinned() {
        let m = manifest();
        assert_eq!(m.repo, "mlx-community/Qwen3-4B-Instruct-2507-4bit");
        assert_eq!(m.files["model.safetensors"].size, 2_263_022_417);
        assert!(m
            .files
            .values()
            .all(|f| f.sha256.is_some() || f.git_sha1.is_some()));
        assert!(m.files.contains_key("config.json"));
    }

    #[test]
    fn validate_model_dir_accepts_fixture_and_rejects_bad_ones() {
        let dir = model_fixture("ok");
        let info = validate_model_dir(&dir).unwrap();
        assert!(info.model_id.starts_with("qwen3/"));
        assert!(!info.pinned);

        assert!(validate_model_dir(&dir.join("config.json")).is_err());

        let no_tok = model_fixture("notok");
        std::fs::remove_file(no_tok.join("tokenizer.json")).unwrap();
        assert!(validate_model_dir(&no_tok)
            .unwrap_err()
            .contains("tokenizer.json"));

        let sharded = model_fixture("shard");
        std::fs::remove_file(sharded.join("model.safetensors")).unwrap();
        std::fs::write(
            sharded.join("model.safetensors.index.json"),
            r#"{"weight_map":{"a":"model-1.safetensors","b":"model-2.safetensors"}}"#,
        )
        .unwrap();
        std::fs::write(sharded.join("model-1.safetensors"), "x").unwrap();
        assert!(validate_model_dir(&sharded)
            .unwrap_err()
            .contains("model-2.safetensors"));
        std::fs::write(sharded.join("model-2.safetensors"), "x").unwrap();
        validate_model_dir(&sharded).unwrap();

        let escape = model_fixture("escape");
        let outside = temp_dir("outside");
        std::fs::write(outside.join("tokenizer.json"), "{}").unwrap();
        std::fs::remove_file(escape.join("tokenizer.json")).unwrap();
        std::os::unix::fs::symlink(
            outside.join("tokenizer.json"),
            escape.join("tokenizer.json"),
        )
        .unwrap();
        assert!(validate_model_dir(&escape).unwrap_err().contains("outside"));
    }

    #[test]
    fn runtime_status_reports_none_missing_invalid() {
        let mut cfg = crate::config::Config::default();
        assert_eq!(runtime_status(&cfg).state, SummaryState::None);
        cfg.summary_model = Some("/nonexistent/gf-summary-model".into());
        assert_eq!(runtime_status(&cfg).state, SummaryState::Missing);
        let dir = model_fixture("status");
        std::fs::remove_file(dir.join("tokenizer.json")).unwrap();
        cfg.summary_model = Some(dir.display().to_string());
        assert_eq!(runtime_status(&cfg).state, SummaryState::Invalid);
    }

    /// Write an executable /bin/sh helper that drains its framed request and
    /// emits `reply` as a framed response (via perl for the binary length).
    fn helper(tag: &str, body: &str) -> PathBuf {
        let dir = temp_dir(tag);
        let path = dir.join("helper.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn framed(json: &str) -> String {
        format!(
            "cat > /dev/null\nperl -e 'my $s=$ARGV[0]; print pack(\"V\", length $s), $s' '{json}'"
        )
    }

    fn segments() -> Vec<Segment> {
        vec![
            Segment {
                start_ms: 0,
                end_ms: 1000,
                text: "Welcome to the planning call.".into(),
                source: Default::default(),
            },
            Segment {
                start_ms: 61_000,
                end_ms: 62_000,
                text: "Dana will send the budget by Friday.".into(),
                source: Default::default(),
            },
        ]
    }

    const SECS: Duration = Duration::from_secs(10);

    #[test]
    fn ipc_grounded_output_is_kept() {
        let h = helper(
            "grounded",
            &framed(
                r#"{"ok":true,"output":{"bullets":[{"text":"Planning call","segment_index":0,"quote":"the planning call"}],"action_items":[{"text":"Send budget","owner":"Dana","segment_index":1,"quote":"Dana will send the budget"}]}}"#,
            ),
        );
        let segs = segments();
        let out = summarize(&h, "/m", &segs, SECS).unwrap();
        let (bullets, d1) = validate_model_grounding(&segs, out.bullets);
        let (items, d2) = validate_model_grounding(&segs, out.action_items);
        assert_eq!((bullets.len(), items.len(), d1 + d2), (1, 1, 0));
        assert_eq!(items[0].owner.as_deref(), Some("Dana"));
        assert_eq!(items[0].start_ms, 61_000);
    }

    #[test]
    fn ipc_fabricated_output_is_dropped() {
        let h = helper(
            "fabricated",
            &framed(
                r#"{"ok":true,"output":{"bullets":[{"text":"x","segment_index":0,"quote":"We agreed to hire two engineers"}],"action_items":[{"text":"y","owner":"Eve","segment_index":0,"quote":"Dana will send the budget"}]}}"#,
            ),
        );
        let segs = segments();
        let out = summarize(&h, "/m", &segs, SECS).unwrap();
        let (bullets, d1) = validate_model_grounding(&segs, out.bullets);
        let (items, d2) = validate_model_grounding(&segs, out.action_items);
        assert!(bullets.is_empty() && items.is_empty());
        assert_eq!(d1 + d2, 2);
    }

    #[test]
    fn ipc_helper_error_and_malformed_reply() {
        let h = helper(
            "err",
            &framed(r#"{"ok":false,"error":"model load failed"}"#),
        );
        assert_eq!(
            summarize(&h, "/m", &segments(), SECS).unwrap_err(),
            "model load failed"
        );
        let h = helper("malformed", &framed("not json"));
        assert!(summarize(&h, "/m", &segments(), SECS)
            .unwrap_err()
            .contains("malformed"));
        // Declared length above the bound.
        let h = helper(
            "huge",
            "cat > /dev/null\nperl -e 'print pack(\"V\", 4000000000)'",
        );
        assert!(summarize(&h, "/m", &segments(), SECS)
            .unwrap_err()
            .contains("too large"));
    }

    #[test]
    fn ipc_timeout_kills_the_helper() {
        let h = helper("timeout", "cat > /dev/null\nsleep 30");
        let start = std::time::Instant::now();
        let err = summarize(&h, "/m", &segments(), Duration::from_millis(500)).unwrap_err();
        assert!(err.contains("timed out"), "{err}");
        assert!(start.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn ipc_crash_reports_exit_and_stderr() {
        let h = helper(
            "crash",
            "cat > /dev/null\necho 'MLX: out of memory' >&2\nexit 3",
        );
        let err = summarize(&h, "/m", &segments(), SECS).unwrap_err();
        assert!(err.contains("without a reply"), "{err}");
        assert!(err.contains("out of memory"), "{err}");
        assert!(err.contains('3'), "{err}");
    }

    #[test]
    fn ipc_missing_binary() {
        let err = summarize(
            Path::new("/nonexistent/greta-summarizer"),
            "/m",
            &segments(),
            SECS,
        )
        .unwrap_err();
        assert!(err.contains("couldn't start summarizer"), "{err}");
    }

    /// Python fixture that serves every framed request in one process,
    /// logging each spawn, and fails requests whose first index is in `fail`.
    fn looping_helper(tag: &str, fail: &[usize]) -> (PathBuf, PathBuf) {
        let dir = temp_dir(tag);
        let spawns = dir.join("spawns");
        let fail: Vec<String> = fail.iter().map(|i| i.to_string()).collect();
        let body = format!(
            r#"exec python3 -c '
import sys, json, struct
open("{spawns}", "a").write("x\n")
fail = {{{fail}}}
while True:
    head = sys.stdin.buffer.read(4)
    if len(head) < 4:
        break
    req = json.loads(sys.stdin.buffer.read(struct.unpack("<I", head)[0]))
    seg = req["segments"][0]
    if seg["index"] in fail:
        rep = {{"ok": False, "error": "no JSON in section %d" % seg["index"]}}
    else:
        rep = {{"ok": True, "output": {{"bullets": [{{"text": "t", "segment_index": seg["index"], "quote": seg["text"][:20]}}], "action_items": []}}}}
    out = json.dumps(rep).encode()
    sys.stdout.buffer.write(struct.pack("<I", len(out)) + out)
    sys.stdout.buffer.flush()
'"#,
            spawns = spawns.display(),
            fail = if fail.is_empty() {
                "-1".into()
            } else {
                fail.join(",")
            },
        );
        (helper(tag, &body), spawns)
    }

    fn three_sections() -> Vec<Segment> {
        (0..3)
            .map(|i| Segment {
                start_ms: i * 60_000,
                end_ms: i * 60_000 + 1,
                text: format!("Section {i} says ").repeat(CHUNK_CHARS / 10),
                source: Default::default(),
            })
            .collect()
    }

    #[test]
    fn one_helper_serves_all_sections_and_keeps_partial_results() {
        let segs = three_sections();
        assert_eq!(chunks(&segs).len(), 3);
        let (h, spawns) = looping_helper("partial", &[1]);
        let out = summarize(&h, "/m", &segs, SECS).unwrap();
        assert_eq!(std::fs::read_to_string(&spawns).unwrap().lines().count(), 1);
        assert_eq!(out.chunks, 3);
        assert_eq!(out.failed_chunks.len(), 1);
        assert_eq!(out.failed_chunks[0].index, 1);
        assert!(out.failed_chunks[0].reason.contains("section 1"));
        let (kept, dropped) = validate_model_grounding(&segs, out.bullets);
        assert_eq!(dropped, 0);
        let cited: Vec<usize> = kept.iter().map(|b| b.segment_index).collect();
        assert_eq!(cited, vec![0, 2]);
    }

    #[test]
    fn every_section_failing_fails_with_each_reason() {
        let segs = three_sections();
        let (h, _) = looping_helper("allfail", &[0, 1, 2]);
        let err = summarize(&h, "/m", &segs, SECS).unwrap_err();
        for i in 0..3 {
            assert!(err.contains(&format!("no JSON in section {i}")), "{err}");
        }
    }

    #[test]
    fn chunking_keeps_global_indices() {
        let segs: Vec<Segment> = (0..10)
            .map(|i| Segment {
                start_ms: i * 1000,
                end_ms: i * 1000 + 1,
                text: "x".repeat(5000),
                source: Default::default(),
            })
            .collect();
        let c = chunks(&segs);
        assert!(c.len() > 1);
        let indices: Vec<usize> = c.iter().flatten().map(|(i, _)| *i).collect();
        assert_eq!(indices, (0..10).collect::<Vec<_>>());
    }
}
