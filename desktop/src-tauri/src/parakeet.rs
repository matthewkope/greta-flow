//! Persistent local CoreML worker, isolated from the app's event loop.
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
impl Session {
    fn start(helper: &Path, model: &Path, timeout: Duration) -> Result<Self, String> {
        let mut child = Command::new(helper)
            .arg(model)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("couldn't start Parakeet helper {}: {e}", helper.display()))?;
        let mut input = child.stdin.take().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let (requests, rx) = mpsc::channel::<Request>();
        let (ready_tx, ready_rx) = mpsc::channel();
        let session = Self { child, requests };
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
                    input
                        .write_all(&(samples.len() as u32).to_le_bytes())
                        .map_err(|e| e.to_string())?;
                    let bytes: Vec<u8> = samples.iter().flat_map(|x| x.to_le_bytes()).collect();
                    input.write_all(&bytes).map_err(|e| e.to_string())?;
                    input.flush().map_err(|e| e.to_string())?;
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
        let session = Session::start(&helper, &model, Duration::from_secs(120))?;
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
        let mut session = self
            .session
            .lock()
            .map_err(|_| "Parakeet session lock failed")?;
        if session.is_none() {
            log::info!("asr_stage engine=parakeet worker_restart=1");
            *session = Some(Session::start(
                &self.helper,
                &self.model,
                Duration::from_secs(120),
            )?);
        }
        let result = session
            .as_ref()
            .unwrap()
            .transcribe(samples, Duration::from_secs(60));
        // Killing the process releases even a blocked pipe write. A later dictation restarts it.
        if result.is_err() {
            session.take();
        }
        result
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
