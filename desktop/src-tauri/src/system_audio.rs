//! System-audio capture through the `greta-system-audio` helper.
//!
//! Pipe protocol on the helper's stdout, all lengths little-endian u32:
//! one header frame `len + JSON` (`{"sample_rate":N}` or `{"error":"..."}`),
//! then repeated `byte_len + raw LE f32 mono samples`. The helper exits when
//! its stdin closes or on SIGTERM. Captured audio is only forwarded to a
//! meeting `Input`; nothing is written to disk here.
use crate::meeting::Input;
use std::io::{ErrorKind, Read};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_HEADER: usize = 64 * 1024;
/// One frame holds at most 4 s of 48 kHz audio; larger frames are rejected.
pub const MAX_FRAME_BYTES: usize = 192_000 * 4;
/// Allows time for the first-run System Audio Recording permission check.
pub const HEADER_TIMEOUT: Duration = Duration::from_secs(20);
const EXIT_GRACE: Duration = Duration::from_secs(2);

#[derive(Debug, PartialEq)]
pub enum Header {
    Rate(u32),
    Error(String),
}

fn read_len(reader: &mut impl Read) -> std::io::Result<Option<usize>> {
    let mut len = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match reader.read(&mut len[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(ErrorKind::UnexpectedEof.into()),
            Ok(n) => got += n,
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(Some(u32::from_le_bytes(len) as usize))
}

/// Read and validate the header frame.
pub fn read_header(reader: &mut impl Read) -> Result<Header, String> {
    let len = read_len(reader)
        .map_err(|e| format!("system audio header unreadable: {e}"))?
        .ok_or("system audio helper exited before reporting a format")?;
    if len == 0 || len > MAX_HEADER {
        return Err(format!("system audio header has invalid length {len}"));
    }
    let mut body = vec![0u8; len];
    reader
        .read_exact(&mut body)
        .map_err(|_| "system audio header was truncated".to_string())?;
    let value: serde_json::Value =
        serde_json::from_slice(&body).map_err(|_| "system audio header is not JSON")?;
    if let Some(error) = value.get("error").and_then(|e| e.as_str()) {
        return Ok(Header::Error(error.to_string()));
    }
    match value.get("sample_rate").and_then(|r| r.as_u64()) {
        Some(rate) if (8_000..=192_000).contains(&rate) => Ok(Header::Rate(rate as u32)),
        Some(rate) => Err(format!("system audio sample rate {rate} is unsupported")),
        None => Err("system audio header has no sample_rate".into()),
    }
}

/// Read one sample frame. `Ok(None)` is a clean end of stream.
pub fn read_frame(reader: &mut impl Read) -> Result<Option<Vec<f32>>, String> {
    let Some(len) = read_len(reader).map_err(|_| "system audio frame was truncated")? else {
        return Ok(None);
    };
    if len > MAX_FRAME_BYTES {
        return Err(format!("system audio frame of {len} bytes is too large"));
    }
    if len % 4 != 0 {
        return Err(format!(
            "system audio frame of {len} bytes is not f32 aligned"
        ));
    }
    let mut bytes = vec![0u8; len];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| "system audio frame was truncated".to_string())?;
    Ok(Some(
        bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect(),
    ))
}

/// Forward frames to `input` until end of stream or a protocol error.
pub fn pump(reader: &mut impl Read, rate: u32, input: &Input) -> Result<(), String> {
    while let Some(samples) = read_frame(reader)? {
        if !samples.is_empty() {
            input.send(samples, rate);
        }
    }
    Ok(())
}

/// Locate the helper the same way `parakeet.rs` locates `gretchen-parakeet`.
pub fn helper_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("GRETA_SYSTEM_AUDIO_HELPER") {
        return Ok(path.into());
    }
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let bundled = executable
        .parent()
        .ok_or("missing executable parent")?
        .join("../Resources/parakeet/greta-system-audio");
    if bundled.is_file() {
        return Ok(bundled);
    }
    if cfg!(debug_assertions) {
        let development = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../parakeet/staging/greta-system-audio");
        if development.is_file() {
            return Ok(development);
        }
    }
    Err("System audio helper is missing from this build".into())
}

/// A running helper whose frames feed a meeting input.
pub struct Capture {
    child: Child,
    stdin: Option<ChildStdin>,
    reader: Option<std::thread::JoinHandle<()>>,
    stopping: Arc<AtomicBool>,
    /// Why capture ended before Stop, if it did.
    ended: Arc<Mutex<Option<String>>>,
}

impl Capture {
    /// Spawn `command`, wait for its header, and forward frames to `input`.
    /// Returns the helper's sample rate. Fails with the helper's own reason.
    pub fn spawn(
        mut command: Command,
        input: Input,
        header_timeout: Duration,
    ) -> Result<(Self, u32), String> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("couldn't start system audio helper: {e}"))?;
        let stdin = child.stdin.take();
        let mut stdout = child
            .stdout
            .take()
            .ok_or("system audio helper has no output")?;
        let stopping = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(Mutex::new(None));
        let (header_tx, header_rx) = mpsc::channel();
        let reader = {
            let (stopping, ended) = (Arc::clone(&stopping), Arc::clone(&ended));
            std::thread::spawn(move || {
                let rate = match read_header(&mut stdout) {
                    Ok(Header::Rate(rate)) => rate,
                    Ok(Header::Error(e)) => {
                        let _ = header_tx.send(Err(e));
                        return;
                    }
                    Err(e) => {
                        let _ = header_tx.send(Err(e));
                        return;
                    }
                };
                if header_tx.send(Ok(rate)).is_err() {
                    return;
                }
                let reason = match pump(&mut stdout, rate, &input) {
                    Ok(()) => "system audio helper exited".to_string(),
                    Err(e) => e,
                };
                if !stopping.load(Ordering::SeqCst) {
                    log::warn!("system audio capture ended: {reason}");
                    if let Ok(mut ended) = ended.lock() {
                        *ended = Some(reason);
                    }
                }
            })
        };
        let mut capture = Self {
            child,
            stdin,
            reader: Some(reader),
            stopping,
            ended,
        };
        match header_rx.recv_timeout(header_timeout) {
            Ok(Ok(rate)) => Ok((capture, rate)),
            Ok(Err(e)) => {
                capture.stop();
                Err(e)
            }
            Err(_) => {
                capture.stop();
                Err("system audio helper did not start in time".into())
            }
        }
    }

    /// Why capture ended on its own, if it did.
    pub fn ended(&self) -> Option<String> {
        self.ended.lock().ok().and_then(|e| e.clone())
    }

    /// Close stdin, wait briefly, then kill; joins the reader so the meeting
    /// input is dropped when this returns.
    pub fn stop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        drop(self.stdin.take());
        let deadline = Instant::now() + EXIT_GRACE;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                _ => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
            }
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if self.reader.is_some() {
            self.stop();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn frame(bytes: &[u8]) -> Vec<u8> {
        let mut out = (bytes.len() as u32).to_le_bytes().to_vec();
        out.extend_from_slice(bytes);
        out
    }
    fn pcm(samples: &[f32]) -> Vec<u8> {
        frame(
            &samples
                .iter()
                .flat_map(|s| s.to_le_bytes())
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn system_audio_parses_header_and_sample_frames() {
        let mut bytes = frame(br#"{"sample_rate":48000}"#);
        bytes.extend(pcm(&[0.5, -0.25]));
        bytes.extend(pcm(&[]));
        bytes.extend(pcm(&[1.0]));
        let mut r = bytes.as_slice();
        assert_eq!(read_header(&mut r), Ok(Header::Rate(48_000)));
        assert_eq!(read_frame(&mut r), Ok(Some(vec![0.5, -0.25])));
        assert_eq!(read_frame(&mut r), Ok(Some(vec![])));
        assert_eq!(read_frame(&mut r), Ok(Some(vec![1.0])));
        assert_eq!(read_frame(&mut r), Ok(None));
    }

    #[test]
    fn system_audio_reports_helper_error_header() {
        let bytes = frame(br#"{"error":"System audio capture requires macOS 14.2 or later"}"#);
        assert_eq!(
            read_header(&mut bytes.as_slice()),
            Ok(Header::Error(
                "System audio capture requires macOS 14.2 or later".into()
            ))
        );
        assert!(read_header(&mut &b""[..]).is_err());
        assert!(read_header(&mut frame(br#"{"sample_rate":5}"#).as_slice()).is_err());
        assert!(read_header(&mut frame(b"not json").as_slice()).is_err());
    }

    #[test]
    fn system_audio_rejects_truncated_and_oversized_frames() {
        let mut truncated = pcm(&[0.1, 0.2]);
        truncated.truncate(truncated.len() - 1);
        assert!(read_frame(&mut truncated.as_slice()).is_err());
        assert!(read_frame(&mut &[1u8, 0][..]).is_err(), "truncated length");
        let oversized = ((MAX_FRAME_BYTES + 4) as u32).to_le_bytes();
        let err = read_frame(&mut &oversized[..]).unwrap_err();
        assert!(err.contains("too large"), "{err}");
        let misaligned = frame(&[0, 0, 0]);
        assert!(read_frame(&mut misaligned.as_slice()).is_err());
        let mut header = 9_999_999u32.to_le_bytes().to_vec();
        header.extend(b"{}");
        assert!(read_header(&mut header.as_slice()).is_err());
    }
}
