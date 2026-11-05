//! Download ggml Whisper models from the ggerganov/whisper.cpp Hugging Face repo
//! into `~/.cache/gretchen-flow/models/`.

use crate::locks::MutexExt;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;

use sha2::{Digest, Sha256};

/// Known ggml models and the SHA-256 of the exact file served from
/// `huggingface.co/ggerganov/whisper.cpp/resolve/main/`. Downloads are verified
/// against these so a tampered mirror, MITM, or redirect can't slip a malicious
/// ggml file (parsed by native whisper.cpp code) past us. Names not listed here
/// are still allowed (e.g. new community models) but download unverified.
const KNOWN_MODEL_SHA256: &[(&str, &str)] = &[
    (
        "large-v3-turbo-q5_0",
        "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
    ),
    (
        "large-v3-turbo",
        "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
    ),
    (
        "small",
        "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
    ),
    (
        "base",
        "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
    ),
];

/// A ggml model name is a bare identifier like `large-v3-turbo-q5_0`. Reject
/// anything that could escape the models directory or rewrite the download URL:
/// only ASCII letters, digits, `.`, `_` and `-` are allowed, and never `..`.
/// (Absolute paths to local model files are handled separately, before this.)
pub fn is_valid_model_name(name: &str) -> bool {
    !name.is_empty()
        && name != ".."
        && !name.contains("..")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

pub fn model_path(name: &str) -> PathBuf {
    if name == crate::config::DEFAULT_MODEL {
        return dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".cache/gretchen-flow/models/parakeet-tdt-0.6b-v2-coreml");
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache/gretchen-flow/models")
        .join(format!("ggml-{name}.bin"))
}

/// Ensure the Whisper model is available locally. Absolute paths are used
/// as-is (user-supplied model files); names are downloaded from the
/// whisper.cpp collection if needed.
pub fn ensure_model(name: &str) -> Result<PathBuf, String> {
    ensure_model_with_progress(name, &mut |_| {})
}

pub fn ensure_model_with_progress(
    name: &str,
    report: &mut dyn FnMut(Progress),
) -> Result<PathBuf, String> {
    report(Progress::phase("checking"));
    if name == crate::config::DEFAULT_MODEL {
        return ensure_parakeet(report);
    }
    if name.starts_with('/') {
        let path = PathBuf::from(name);
        return if path.exists() {
            Ok(path)
        } else {
            Err(format!("model file not found: {name}"))
        };
    }
    if !is_valid_model_name(name) {
        return Err(format!("invalid model name: {name:?}"));
    }
    let path = model_path(name);
    download_if_missing(
        || Ok(path.exists()),
        || {
            fs::create_dir_all(path.parent().ok_or("model path has no parent")?)
                .map_err(|e| e.to_string())?;

            let url = format!(
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{name}.bin"
            );
            log::info!("downloading model {name} from {url}");

            let resp = ureq::get(&url)
                .call()
                .map_err(|e| format!("download failed: {e}"))?;
            let tmp = path.with_extension("bin.part");
            let mut file = fs::File::create(&tmp).map_err(|e| e.to_string())?;
            download(resp, &mut file, &path, name, report)?;
            report(Progress::phase("verifying"));
            drop(file);

            // Verify integrity against the pinned hash before the file is ever loaded
            // by native whisper.cpp. Unknown models can't be verified, so log that.
            if let Some(expected) = expected_sha256(name) {
                match file_sha256(&tmp) {
                    Ok(actual) if actual.eq_ignore_ascii_case(expected) => {}
                    Ok(actual) => {
                        let _ = fs::remove_file(&tmp);
                        return Err(format!(
                    "model {name} failed integrity check (expected {expected}, got {actual}); discarded"
                ));
                    }
                    Err(e) => {
                        let _ = fs::remove_file(&tmp);
                        return Err(format!("couldn't hash downloaded model {name}: {e}"));
                    }
                }
            } else {
                log::warn!("model {name} has no pinned hash; integrity not verified");
            }

            fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
            log::info!("model saved to {}", path.display());
            Ok(())
        },
    )?;
    Ok(path)
}

// Only writers wait for downloads. Verified/cached files and local paths remain
// available while an obsolete selection is downloading another model.
static DOWNLOAD: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn download_if_missing(
    mut cached: impl FnMut() -> Result<bool, String>,
    download: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if cached()? {
        return Ok(());
    }
    let _guard = DOWNLOAD.lock_recover();
    // Another worker may have finished this file while we waited.
    if cached()? {
        return Ok(());
    }
    download()
}

fn expected_sha256(name: &str) -> Option<&'static str> {
    KNOWN_MODEL_SHA256
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, h)| *h)
}

/// Streaming SHA-256 of a file as a lowercase hex string.
fn file_sha256(path: &std::path::Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Fetch only the pinned CoreML exports used by our bundled runtime.
fn ensure_parakeet(report: &mut dyn FnMut(Progress)) -> Result<PathBuf, String> {
    let root = model_path(crate::config::DEFAULT_MODEL);
    let manifest: std::collections::BTreeMap<String, String> =
        serde_json::from_str(include_str!("parakeet-model.json")).map_err(|e| e.to_string())?;
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(30))
        .timeout_read(std::time::Duration::from_secs(120))
        .build();
    for (relative, hash) in manifest {
        let path = root.join(&relative);
        download_if_missing(
            || Ok(path.is_file() && file_sha256(&path)? == hash),
            || {
                fs::create_dir_all(path.parent().ok_or("model path has no parent")?)
                    .map_err(|e| e.to_string())?;
                let url = format!("https://huggingface.co/FluidInference/parakeet-tdt-0.6b-v2-coreml/resolve/ee09c569f73759e6d44c9bd16766f477b2b36d39/{relative}");
                let response = agent.get(&url).call().map_err(|e| e.to_string())?;
                let temp = path.with_extension("download-part");
                let mut file = fs::File::create(&temp).map_err(|e| e.to_string())?;
                download(response, &mut file, &path, &relative, report)?;
                report(Progress {
                    file: Some(relative.clone()),
                    ..Progress::phase("verifying")
                });
                drop(file);
                if file_sha256(&temp)? != hash {
                    let _ = fs::remove_file(&temp);
                    return Err(format!("Parakeet integrity check failed: {relative}"));
                }
                fs::rename(temp, &path).map_err(|e| e.to_string())
            },
        )?;
    }
    Ok(root)
}

/// Byte counts are per file; totals are unknown when the server omits Content-Length.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Progress {
    pub phase: String,
    pub file: Option<String>,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}
impl Progress {
    pub fn phase(phase: &str) -> Self {
        Self {
            phase: phase.into(),
            file: None,
            downloaded_bytes: 0,
            total_bytes: None,
        }
    }
}

#[derive(serde::Serialize)]
pub struct DiskSpace {
    pub available_bytes: u64,
    pub required_bytes: u64,
    pub sufficient: bool,
}

/// Query the actual cache volume (or its nearest existing ancestor).
pub fn disk_space(path: &std::path::Path, required_bytes: u64) -> Result<DiskSpace, String> {
    use std::os::unix::ffi::OsStrExt;
    let mut ancestor = path;
    while !ancestor.exists() {
        ancestor = ancestor.parent().ok_or("no existing cache ancestor")?;
    }
    let path =
        std::ffi::CString::new(ancestor.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // statvfs initializes the structure only on success; f_bavail excludes reserved blocks.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(format!(
            "disk space check: {}",
            std::io::Error::last_os_error()
        ));
    }
    let stats = unsafe { stats.assume_init() };
    let available_bytes = u64::from(stats.f_bavail).saturating_mul(stats.f_frsize);
    Ok(DiskSpace {
        available_bytes,
        required_bytes,
        sufficient: available_bytes >= required_bytes,
    })
}

fn download(
    response: ureq::Response,
    file: &mut fs::File,
    path: &std::path::Path,
    name: &str,
    report: &mut dyn FnMut(Progress),
) -> Result<(), String> {
    let total = response
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok());
    let space = disk_space(path, total.unwrap_or(0).saturating_add(256 * 1024 * 1024))?;
    if !space.sufficient {
        return Err(format!(
            "insufficient disk space: need {} bytes, available {} bytes",
            space.required_bytes, space.available_bytes
        ));
    }
    copy_download(&mut response.into_reader(), file, name, total, report)
}

fn copy_download(
    reader: &mut dyn Read,
    writer: &mut dyn Write,
    name: &str,
    total: Option<u64>,
    report: &mut dyn FnMut(Progress),
) -> Result<(), String> {
    let mut progress = Progress {
        file: Some(name.into()),
        total_bytes: total,
        ..Progress::phase("downloading")
    };
    report(progress.clone());
    let mut last = std::time::Instant::now();
    let mut buf = [0; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("download read failed: {e}"))?;
        if n == 0 {
            break;
        }
        writer
            .write_all(&buf[..n])
            .map_err(|e| format!("download write failed: {e}"))?;
        progress.downloaded_bytes += n as u64;
        if last.elapsed() >= std::time::Duration::from_millis(100) {
            report(progress.clone());
            last = std::time::Instant::now();
        }
    }
    report(progress.clone());
    if total.is_some_and(|total| total != progress.downloaded_bytes) {
        return Err("download ended with an unexpected byte count".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_and_local_models_do_not_wait_for_an_unrelated_download() {
        let guard = DOWNLOAD.lock_recover();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let cached = download_if_missing(|| Ok(true), || panic!("cached file downloaded"));
            let local = ensure_model(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));
            tx.send((cached, local)).unwrap();
        });
        let result = rx.recv_timeout(std::time::Duration::from_secs(2));
        // Release even on failure so a regression does not hang the test suite.
        drop(guard);
        worker.join().unwrap();
        let (cached, local) = result.expect("cached/local model waited on download lock");
        assert!(cached.is_ok());
        assert!(local.is_ok());
    }

    #[test]
    fn waiting_writer_rechecks_cache_before_downloading() {
        let mut checks = 0;
        download_if_missing(
            || {
                checks += 1;
                Ok(checks == 2)
            },
            || panic!("file completed by another writer was downloaded again"),
        )
        .unwrap();
        assert_eq!(checks, 2);
    }

    #[test]
    fn reports_bytes_and_rejects_truncated_downloads() {
        for total in [None, Some(150_000)] {
            let input = vec![7; 150_000];
            let mut output = Vec::new();
            let mut events = Vec::new();
            copy_download(&mut &input[..], &mut output, "weights", total, &mut |p| {
                events.push(p)
            })
            .unwrap();
            assert_eq!(output, input);
            assert_eq!(events.first().unwrap().downloaded_bytes, 0);
            assert_eq!(events.last().unwrap().downloaded_bytes, 150_000);
            assert_eq!(events.last().unwrap().total_bytes, total);
        }
        assert!(copy_download(
            &mut &b"short"[..],
            &mut Vec::new(),
            "weights",
            Some(10),
            &mut |_| {}
        )
        .is_err());
    }

    #[test]
    fn reports_write_failure_and_checks_cache_volume() {
        struct Full;
        impl Write for Full {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk full"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let error =
            copy_download(&mut &b"data"[..], &mut Full, "weights", None, &mut |_| {}).unwrap_err();
        assert!(error.contains("disk full"));
        let space =
            disk_space(&std::env::temp_dir().join("gf-nonexistent/child"), u64::MAX).unwrap();
        assert!(!space.sufficient);
        assert_eq!(space.required_bytes, u64::MAX);
    }

    #[test]
    fn rejects_traversal_and_separators() {
        assert!(!is_valid_model_name("x/../../../../tmp/evil"));
        assert!(!is_valid_model_name("../../etc/passwd"));
        assert!(!is_valid_model_name(".."));
        assert!(!is_valid_model_name("a/b"));
        assert!(!is_valid_model_name("a..b"));
        assert!(!is_valid_model_name(""));
        assert!(!is_valid_model_name("a b"));
    }

    #[test]
    fn accepts_real_model_names() {
        assert!(is_valid_model_name("large-v3-turbo-q5_0"));
        assert!(is_valid_model_name("large-v3-turbo"));
        assert!(is_valid_model_name("small"));
        assert!(is_valid_model_name("base.en"));
    }

    #[test]
    fn every_menu_model_has_a_pinned_hash() {
        // Guards against adding a tray model choice without its integrity hash.
        for name in ["large-v3-turbo-q5_0", "large-v3-turbo", "small", "base"] {
            assert!(expected_sha256(name).is_some(), "missing hash for {name}");
        }
    }

    #[test]
    fn hex_lower_is_zero_padded() {
        assert_eq!(hex_lower(&[0x00, 0x0f, 0xff]), "000fff");
    }
}
