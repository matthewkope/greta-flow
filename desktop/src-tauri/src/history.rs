//! A single bounded persistence owner orders append, clear, retention, and reads.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::{
        mpsc::{self, Sender, SyncSender},
        OnceLock,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    pub enabled: bool,
    pub max_entries: usize,
    pub max_age_days: u64,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_entries: 200,
            max_age_days: 0,
        }
    }
}
#[derive(Serialize, Deserialize)]
struct Entry {
    ts: u64,
    text: String,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
pub fn history_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(crate::config::storage_name())
        .join("history.jsonl")
}
struct Store {
    path: PathBuf,
    policy: Policy,
}
impl Store {
    fn read(&self) -> Vec<Entry> {
        if !self.policy.enabled {
            return vec![];
        }
        let Ok(data) = fs::read_to_string(&self.path) else {
            return vec![];
        };
        data.lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }
    fn prune(&self, entries: &mut Vec<Entry>) {
        let age = self.policy.max_age_days.saturating_mul(86400);
        entries.retain(|e| age == 0 || now().saturating_sub(e.ts) <= age);
        let cap = self.policy.max_entries.min(10_000);
        if entries.len() > cap {
            entries.drain(..entries.len() - cap);
        }
        // Bound retained transcript data even when individual dictations are long.
        let mut bytes = entries.iter().map(|e| e.text.len()).sum::<usize>();
        let mut count = 0;
        while bytes > 8 * 1024 * 1024 && count < entries.len() {
            bytes -= entries[count].text.len();
            count += 1;
        }
        entries.drain(..count);
    }
    fn save(&self, entries: &[Entry]) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp = self
            .path
            .with_extension(format!("{}.tmp", std::process::id()));
        let result = (|| {
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .mode(0o600)
                .open(&temp)?;
            for entry in entries {
                serde_json::to_writer(&mut file, entry)?;
                file.write_all(b"\n")?;
            }
            file.sync_all()?;
            fs::rename(&temp, &self.path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
    fn append(&mut self, text: String) -> std::io::Result<()> {
        if !self.policy.enabled || self.policy.max_entries == 0 {
            return Ok(());
        }
        let mut entries = self.read();
        entries.push(Entry { ts: now(), text });
        self.prune(&mut entries);
        self.save(&entries)
    }
    fn expire(&self) -> std::io::Result<()> {
        if !self.policy.enabled || !self.path.exists() {
            return Ok(());
        }
        let mut entries = self.read();
        let before = entries.len();
        self.prune(&mut entries);
        if entries.len() != before {
            self.save(&entries)?;
        }
        Ok(())
    }
    fn clear(&self) -> std::io::Result<()> {
        match fs::remove_file(&self.path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
    fn configure(&mut self, policy: Policy) -> std::io::Result<()> {
        self.policy = policy;
        if !self.policy.enabled || self.policy.max_entries == 0 {
            return self.clear();
        }
        if self.path.exists() {
            let mut entries = self.read();
            self.prune(&mut entries);
            self.save(&entries)?;
        }
        Ok(())
    }
}
enum Command {
    Append(String),
    Clear(Sender<()>),
    Recent(usize, Sender<Vec<String>>),
    Configure(Policy, Sender<()>),
}
fn spawn(path: PathBuf) -> SyncSender<Command> {
    let (tx, rx) = mpsc::sync_channel(64);
    std::thread::spawn(move || {
        let mut store = Store {
            path,
            policy: Policy::default(),
        };
        loop {
            let command = match rx.recv_timeout(Duration::from_secs(60)) {
                Ok(command) => command,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Err(e) = store.expire() {
                        log::warn!("history retention failed: {}", e.kind());
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            let started = std::time::Instant::now();
            let result = match command {
                Command::Append(text) => store.append(text),
                Command::Clear(reply) => {
                    let result = store.clear();
                    let _ = reply.send(());
                    result
                }
                Command::Configure(policy, reply) => {
                    let result = store.configure(policy);
                    let _ = reply.send(());
                    result
                }
                Command::Recent(n, reply) => {
                    let mut entries = store.read();
                    store.prune(&mut entries);
                    let _ = reply.send(entries.into_iter().rev().take(n).map(|e| e.text).collect());
                    Ok(())
                }
            };
            log::debug!(
                "history_operation ms={:.3}",
                started.elapsed().as_secs_f64() * 1000.
            );
            if let Err(e) = result {
                log::warn!("history operation failed: {}", e.kind());
            }
        }
    });
    tx
}
fn owner() -> &'static SyncSender<Command> {
    static OWNER: OnceLock<SyncSender<Command>> = OnceLock::new();
    OWNER.get_or_init(|| spawn(history_path()))
}
pub fn append(text: &str) {
    if owner().try_send(Command::Append(text.into())).is_err() {
        log::warn!("history queue full or unavailable");
    }
}
pub fn clear() {
    let (tx, rx) = mpsc::channel();
    if owner().send(Command::Clear(tx)).is_ok() {
        let _ = rx.recv_timeout(Duration::from_secs(1));
    }
}
pub fn configure(policy: Policy) {
    let (tx, rx) = mpsc::channel();
    if owner().send(Command::Configure(policy, tx)).is_ok() {
        let _ = rx.recv_timeout(Duration::from_secs(1));
    }
}
pub fn recent(n: usize) -> Vec<String> {
    let (tx, rx) = mpsc::channel();
    if owner().try_send(Command::Recent(n, tx)).is_err() {
        return vec![];
    }
    rx.recv_timeout(Duration::from_millis(250))
        .unwrap_or_default()
}
#[cfg(test)]
mod tests {
    use super::*;
    fn temp() -> PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::env::temp_dir()
            .join(format!(
                "gretchen-history-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            ))
            .join("history.jsonl")
    }
    #[test]
    fn retention_count_age_and_disabled_storage() {
        let path = temp();
        let mut s = Store {
            path: path.clone(),
            policy: Policy {
                max_entries: 2,
                max_age_days: 1,
                ..Default::default()
            },
        };
        for word in ["one", "two", "three"] {
            s.append(word.into()).unwrap();
        }
        assert_eq!(s.read().len(), 2);
        assert_eq!(s.read()[0].text, "two");
        let mut entries = vec![
            Entry {
                ts: 1,
                text: "old".into(),
            },
            Entry {
                ts: now(),
                text: "new".into(),
            },
        ];
        s.prune(&mut entries);
        assert_eq!(entries.len(), 1);
        s.configure(Policy {
            enabled: false,
            ..Default::default()
        })
        .unwrap();
        s.append("secret".into()).unwrap();
        assert!(!path.exists());
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn concurrent_appends_then_clear_are_ordered() {
        let path = temp();
        let tx = spawn(path.clone());
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    for j in 0..4 {
                        tx.send(Command::Append(format!("{i}-{j}"))).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let (reply, rx) = mpsc::channel();
        tx.send(Command::Recent(200, reply)).unwrap();
        assert_eq!(rx.recv().unwrap().len(), 32);
        let (reply, rx) = mpsc::channel();
        tx.send(Command::Clear(reply)).unwrap();
        rx.recv().unwrap();
        assert!(!path.exists());
        tx.send(Command::Append("after".into())).unwrap();
        let (reply, rx) = mpsc::channel();
        tx.send(Command::Recent(200, reply)).unwrap();
        assert_eq!(rx.recv().unwrap(), vec!["after"]);
        drop(tx);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn write_failure_does_not_kill_owner() {
        let path = temp();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"block directory").unwrap();
        let tx = spawn(path.join("bad"));
        tx.send(Command::Append("secret".into())).unwrap();
        let (reply, rx) = mpsc::channel();
        tx.send(Command::Recent(1, reply)).unwrap();
        assert!(rx.recv_timeout(Duration::from_secs(1)).unwrap().is_empty());
        drop(tx);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
