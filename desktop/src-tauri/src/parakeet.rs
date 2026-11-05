//! Persistent local CoreML worker, isolated from the app's event loop.
use crate::locks::MutexExt;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

const MAX_SAMPLES: usize = 16_000 * 600;
type Reply = Result<serde_json::Value, String>;
type Request = (Vec<f32>, mpsc::Sender<Reply>);

struct Session {
    child: Child,
    requests: mpsc::Sender<Request>,
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn read_reply(reader: &mut impl BufRead) -> Reply {
    let mut line = String::new();
    // Bound output as well as input; a broken worker cannot exhaust app memory.
    (&mut *reader)
        .take(1_048_577)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    if line.len() > 1_048_576 || !line.ends_with('\n') {
        return Err("Parakeet worker closed or sent an invalid response".into());
    }
    serde_json::from_str(&line).map_err(|e| format!("Parakeet response: {e}"))
}

// Borrow the PCM bytes on Apple Silicon instead of allocating another full
// recording (up to 38.4 MB). Preserve the little-endian wire protocol everywhere.
fn write_samples(writer: &mut impl Write, samples: &[f32]) -> std::io::Result<()> {
    writer.write_all(&(samples.len() as u32).to_le_bytes())?;
    #[cfg(target_endian = "little")]
    writer.write_all(bytemuck::cast_slice(samples))?;
    #[cfg(target_endian = "big")]
    {
        let mut bytes = [0u8; 65_536];
        for chunk in samples.chunks(bytes.len() / 4) {
            for (sample, dest) in chunk.iter().zip(bytes.chunks_exact_mut(4)) {
                dest.copy_from_slice(&sample.to_le_bytes());
            }
            writer.write_all(&bytes[..chunk.len() * 4])?;
        }
    }
    writer.flush()
}
impl Session {
    fn start(helper: &Path, model: &Path, timeout: Duration) -> Result<Self, String> {
        let child = Command::new(helper)
            .arg(model)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("couldn't start Parakeet helper {}: {e}", helper.display()))?;
        let (requests, rx) = mpsc::channel::<Request>();
        let (ready_tx, ready_rx) = mpsc::channel();
        // Install the child guard before any fallible pipe setup.
        let mut session = Self { child, requests };
        let mut input = session.child.stdin.take().ok_or("missing helper stdin")?;
        let mut output =
            BufReader::new(session.child.stdout.take().ok_or("missing helper stdout")?);
        std::thread::spawn(move || {
            let ready = read_reply(&mut output);
            let ok = ready
                .as_ref()
                .is_ok_and(|v| v["ready"] == true && v["protocol"] == 1);
            let _ = ready_tx.send(ready);
            if !ok {
                return;
            }
            for (samples, reply) in rx {
                let result = (|| {
                    write_samples(&mut input, &samples).map_err(|e| e.to_string())?;
                    read_reply(&mut output)
                })();
                let failed = result.is_err();
                let _ = reply.send(result);
                if failed {
                    break;
                }
            }
        });
        let ready = ready_rx
            .recv_timeout(timeout)
            .map_err(|e| format!("Parakeet startup: {e}"))??;
        if ready["ready"] != true || ready["protocol"] != 1 {
            return Err("Parakeet helper protocol mismatch".into());
        }
        Ok(session)
    }

    fn warmed(helper: &Path, model: &Path) -> Result<Self, String> {
        let session = Self::start(helper, model, Duration::from_secs(120))?;
        let start = std::time::Instant::now();
        // FluidAudio loadModels installs weights but does not run inference.
        // The helper creates fresh decoder state for each call; discard silence
        // output before declaring the app ready, including after a restart.
        session.transcribe(&vec![0.; 16_000], Duration::from_secs(60))?;
        log::info!(
            "asr_stage engine=parakeet stage=warmup ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.
        );
        Ok(session)
    }
    fn transcribe(&self, samples: &[f32], timeout: Duration) -> Result<String, String> {
        let (tx, rx) = mpsc::channel();
        self.requests
            .send((samples.to_vec(), tx))
            .map_err(|e| e.to_string())?;
        let reply = rx
            .recv_timeout(timeout)
            .map_err(|e| format!("Parakeet transcription: {e}"))??;
        if let Some(error) = reply["error"].as_str() {
            return Err(error.into());
        }
        reply["text"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "Parakeet response missing text".into())
    }
}
pub struct Engine {
    session: Mutex<Option<Session>>,
    helper: PathBuf,
    model: PathBuf,
}
impl Engine {
    pub fn load(path: &str, cfg: &crate::config::Config) -> Result<Self, String> {
        if !matches!(cfg.language.as_str(), "en" | "auto") {
            return Err("Parakeet v2 supports English. Select Whisper for other languages.".into());
        }
        let helper = helper_path()?;
        let model = PathBuf::from(path);
        let session = Session::warmed(&helper, &model)?;
        Ok(Self {
            session: Mutex::new(Some(session)),
            helper,
            model,
        })
    }
    pub fn transcribe(&self, samples: &[f32]) -> Result<String, String> {
        if samples.is_empty() {
            return Ok(String::new());
        }
        if samples.len() > MAX_SAMPLES || samples.iter().any(|s| !s.is_finite()) {
            return Err("Parakeet requires finite audio of at most 10 minutes".into());
        }
        let mut session = self.session.lock_repair(|session| {
            session.take();
        });
        self.restart_if_needed(&mut session)?;
        let result = session
            .as_ref()
            .ok_or("Parakeet worker unavailable")?
            .transcribe(samples, Duration::from_secs(60));
        // Killing the process releases even a blocked pipe write. Never retry
        // uncertain in-flight recognition automatically; the next call recovers.
        if result.is_err() {
            session.take();
        }
        result
    }

    pub fn ensure_ready(&self) -> Result<(), String> {
        let mut session = self.session.lock_repair(|session| {
            session.take();
        });
        self.restart_if_needed(&mut session)
    }

    fn restart_if_needed(&self, session: &mut Option<Session>) -> Result<(), String> {
        if let Some(worker) = session.as_mut() {
            match worker.child.try_wait() {
                Ok(None) => return Ok(()),
                Ok(Some(_)) | Err(_) => {
                    session.take();
                }
            }
        }
        if session.is_none() {
            log::info!("asr_stage engine=parakeet worker_restart=1");
            *session = Some(Session::warmed(&self.helper, &self.model)?);
        }
        Ok(())
    }
}
fn helper_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("GRETCHEN_PARAKEET_HELPER") {
        return Ok(path.into());
    }
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let bundled = executable
        .parent()
        .ok_or("missing executable parent")?
        .join("../Resources/parakeet/gretchen-parakeet");
    if bundled.is_file() {
        return Ok(bundled);
    }
    if cfg!(debug_assertions) {
        let development =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../parakeet/staging/gretchen-parakeet");
        if development.is_file() {
            return Ok(development);
        }
    }
    Err("Parakeet helper is missing; use the packaged app or set GRETCHEN_PARAKEET_HELPER".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pcm_frames_preserve_bits_for_long_buffers() {
        let samples: Vec<f32> = (0..40_001).map(|i| (i as f32 - 20_000.) / 7.).collect();
        let mut actual = vec![];
        write_samples(&mut actual, &samples).unwrap();
        let mut expected = (samples.len() as u32).to_le_bytes().to_vec();
        expected.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
        assert_eq!(actual, expected);
        assert!(write_samples(&mut std::io::sink(), &[]).is_ok());
    }

    #[cfg(unix)]
    fn fake_engine() -> Engine {
        Engine {
            session: Mutex::new(None),
            helper: PathBuf::from("/usr/bin/python3"),
            model: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/parakeet-worker.py"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn idle_maintenance_restarts_a_crashed_worker_and_keeps_it_warm() {
        let engine = fake_engine();
        engine.ensure_ready().unwrap();
        let first_pid = {
            let mut slot = engine.session.lock().unwrap();
            let worker = slot.as_mut().unwrap();
            let pid = worker.child.id();
            worker.child.kill().unwrap();
            worker.child.wait().unwrap();
            pid
        };
        engine.ensure_ready().unwrap();
        let second_pid = engine.session.lock().unwrap().as_ref().unwrap().child.id();
        assert_ne!(first_pid, second_pid);
        assert_eq!(engine.transcribe(&[1.]).unwrap(), "fixture");
        engine.ensure_ready().unwrap();
        assert_eq!(
            engine.session.lock().unwrap().as_ref().unwrap().child.id(),
            second_pid
        );
    }

    #[cfg(unix)]
    #[test]
    fn failed_request_and_poisoned_session_do_not_break_later_dictation() {
        let engine = fake_engine();
        // The fixture exits during a two-sample request; do not silently retry it.
        assert!(engine.transcribe(&[1., 2.]).is_err());
        assert!(engine.session.lock().unwrap().is_none());
        assert_eq!(engine.transcribe(&[1.]).unwrap(), "fixture");
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = engine.session.lock().unwrap();
            panic!("interrupted helper access");
        }));
        assert_eq!(engine.transcribe(&[1.]).unwrap(), "fixture");
        assert!(!engine.session.is_poisoned());
    }

    #[test]
    fn rejects_closed_truncated_and_oversized_replies() {
        for bytes in [
            b"".to_vec(),
            b"{\"text\":\"a\"}".to_vec(),
            vec![b'a'; 1_048_577],
        ] {
            assert!(read_reply(&mut std::io::Cursor::new(bytes)).is_err());
        }
        assert_eq!(
            read_reply(&mut std::io::Cursor::new(b"{\"text\":\"hello\"}\n")).unwrap()["text"],
            "hello"
        );
    }
    // Real child processes exercise startup timeout and pipe failure without model inference.
    #[cfg(unix)]
    #[test]
    fn worker_startup_failure_is_bounded() {
        let start = std::time::Instant::now();
        assert!(Session::start(
            Path::new("/usr/bin/yes"),
            Path::new("bad protocol"),
            Duration::from_millis(100)
        )
        .is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
        let start = std::time::Instant::now();
        assert!(Session::start(
            Path::new("/bin/sleep"),
            Path::new("30"),
            Duration::from_millis(100)
        )
        .is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(Session::start(
            Path::new("/usr/bin/false"),
            Path::new("unused"),
            Duration::from_secs(1)
        )
        .is_err());
    }
}
