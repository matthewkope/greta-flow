//! One delivery per session, with bounded, expiring, memory-only recovery.
use crate::inject::Target;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

pub struct CaptureContext {
    pub target: Option<Target>,
    pub cfg: crate::config::Config,
    pub completed: Instant,
}
const TTL: Duration = Duration::from_secs(120);
const MAX_AUDIO: usize = 16_000 * 120;
#[derive(Default)]
pub struct Sessions {
    next: u64,
    current: Option<Job>,
}
pub struct Job {
    pub id: u64,
    pub cfg: crate::config::Config,
    pub stream: Option<crate::streaming::Worker>,
    pub target: Option<Target>,
    pub pending_context: Option<Receiver<CaptureContext>>,
    history_text: Option<String>,
    pub audio: Vec<f32>,
    pub text: Option<String>,
    expires: Instant,
    cancelled: bool,
    attempted: bool,
    delivered: bool,
}
impl Sessions {
    pub fn begin(&mut self) -> u64 {
        self.next += 1;
        self.current = Some(Job {
            id: self.next,
            cfg: Default::default(),
            stream: None,
            target: None,
            pending_context: None,
            history_text: None,
            audio: vec![],
            text: None,
            expires: Instant::now() + Duration::from_secs(720),
            cancelled: false,
            attempted: false,
            delivered: false,
        });
        self.next
    }
    pub fn get(&mut self, id: u64) -> Option<&mut Job> {
        self.expire();
        self.current
            .as_mut()
            .filter(|j| j.id == id && !j.cancelled && !j.delivered)
    }
    pub fn expire(&mut self) {
        if self
            .current
            .as_ref()
            .is_some_and(|j| Instant::now() >= j.expires)
        {
            self.current = None;
        }
    }
    pub fn cancel(&mut self) {
        self.current = None;
    }
    pub fn id(&mut self) -> Option<u64> {
        self.expire();
        self.current.as_ref().map(|j| j.id)
    }
    pub fn retain_audio(&mut self, id: u64, samples: &[f32]) {
        if let Some(j) = self.get(id) {
            if samples.len() <= MAX_AUDIO {
                j.audio = samples.to_vec();
            }
            j.expires = Instant::now() + TTL;
        }
    }
    /// Resolve only a destination obtained before release. Slow AX calls never
    /// delay microphone start or retarget a completed recording to a later focus.
    pub fn resolve_capture(&mut self, id: u64, release: Instant) {
        let Some(job) = self.get(id) else { return };
        let Some(pending) = job.pending_context.take() else {
            return;
        };
        match pending.try_recv() {
            Ok(context) if context.completed <= release => {
                job.target = context.target;
                job.cfg = context.cfg;
            }
            _ => {
                // Keep the key-down config snapshot and decode the full captured
                // audio. The pending worker may have a different app profile.
                job.stream = None;
                job.target = None;
            }
        }
    }
    /// Preserve each distinct result in the existing history surface, including
    /// insertion failures. History's configured retention/disabled policy still applies.
    pub fn take_history(&mut self, id: u64) -> Option<String> {
        self.expire();
        let job = self.current.as_mut().filter(|j| j.id == id)?;
        let text = job.text.as_ref().filter(|text| !text.is_empty())?;
        if job.history_text.as_ref() == Some(text) {
            return None;
        }
        job.history_text = Some(text.clone());
        Some(text.clone())
    }
    pub fn recovery(&mut self) -> Option<String> {
        self.expire();
        self.current.as_ref().and_then(|j| j.text.clone())
    }
    /// Serialized with cancel. After an insertion attempt, recovery is manual:
    /// an application could report failure after accepting part/all of a write.
    pub fn deliver(
        &mut self,
        id: u64,
        text: String,
        insert: impl FnOnce(Option<&Target>, &str) -> Result<(), String>,
    ) -> Result<bool, String> {
        self.deliver_checked(
            id,
            text,
            |target| {
                if let Some(target) = target {
                    target.validate()?;
                }
                Ok(())
            },
            insert,
        )
    }
    fn deliver_checked(
        &mut self,
        id: u64,
        text: String,
        validate: impl FnOnce(Option<&Target>) -> Result<(), String>,
        insert: impl FnOnce(Option<&Target>, &str) -> Result<(), String>,
    ) -> Result<bool, String> {
        let Some(job) = self.get(id) else {
            return Ok(false);
        };
        if job.attempted {
            return Err("Delivery already attempted; use recovery text to avoid duplicates".into());
        }
        job.text = Some(text.clone());
        validate(job.target.as_ref())?;
        job.attempted = true;
        insert(job.target.as_ref(), &text)?;
        job.delivered = true;
        job.audio.clear();
        Ok(true)
    }
    pub fn retry_audio(&mut self) -> Option<(u64, Vec<f32>)> {
        let id = self.id()?;
        let job = self.get(id)?;
        if job.attempted || job.audio.is_empty() {
            return None;
        }
        Some((id, job.audio.clone()))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn poison_discards_uncertain_delivery_without_reusing_session_ids() {
        use crate::locks::MutexExt;
        let sessions = std::sync::Mutex::new(Sessions::default());
        let old = sessions.lock().unwrap().begin();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = sessions.lock().unwrap();
            panic!("interrupted delivery");
        }));
        let mut sessions = sessions.lock_repair(Sessions::cancel);
        assert!(!sessions
            .deliver(old, "stale".into(), |_, _| panic!())
            .unwrap());
        let next = sessions.begin();
        assert_ne!(old, next);
        assert!(sessions
            .deliver(next, "next".into(), |_, _| Ok(()))
            .unwrap());
    }

    #[test]
    fn default_config_keeps_failed_transcripts_in_history_without_duplicates() {
        let cfg = crate::config::Config::default();
        assert!(cfg.history.enabled);
        assert!(cfg.recovery_shortcut.is_none());
        for failure in [
            "missing target",
            "unsupported field",
            "uncertain native setter",
        ] {
            let mut sessions = Sessions::default();
            let id = sessions.begin();
            assert!(sessions
                .deliver(id, "recover me".into(), |_, _| Err(failure.into()))
                .is_err());
            assert_eq!(sessions.take_history(id).as_deref(), Some("recover me"));
            assert!(sessions.take_history(id).is_none());
            assert!(sessions
                .deliver(id, "do not replace uncertain text".into(), |_, _| panic!())
                .is_err());
            assert!(sessions.take_history(id).is_none());
            assert_eq!(sessions.recovery().as_deref(), Some("recover me"));
        }
        let mut sessions = Sessions::default();
        let id = sessions.begin();
        assert!(sessions
            .deliver_checked(
                id,
                "focus moved".into(),
                |_| Err("changed".into()),
                |_, _| panic!()
            )
            .is_err());
        assert_eq!(sessions.take_history(id).as_deref(), Some("focus moved"));
        assert!(sessions
            .deliver_checked(id, "focus moved".into(), |_| Ok(()), |_, _| Ok(()))
            .unwrap());
        assert!(sessions.take_history(id).is_none());
        let id = sessions.begin();
        sessions.cancel();
        assert!(!sessions
            .deliver(id, "cancelled".into(), |_, _| panic!())
            .unwrap());
        assert!(sessions.take_history(id).is_none());
    }

    #[test]
    fn pending_or_late_capture_cannot_retarget_after_release() {
        for late in [false, true] {
            let mut sessions = Sessions::default();
            let id = sessions.begin();
            let (tx, rx) = std::sync::mpsc::channel();
            let release = Instant::now();
            sessions.get(id).unwrap().pending_context = Some(rx);
            let (input, worker) = crate::streaming::Worker::spawn(
                std::sync::Arc::new(std::sync::Mutex::new(None)),
                crate::config::Config::default(),
            );
            let cancelled = worker.cancelled.clone();
            sessions.get(id).unwrap().stream = Some(worker);
            if late {
                tx.send(CaptureContext {
                    target: None,
                    cfg: crate::config::Config {
                        language: "late-profile".into(),
                        ..Default::default()
                    },
                    completed: release + Duration::from_millis(1),
                })
                .unwrap();
            }
            sessions.resolve_capture(id, release);
            assert_eq!(sessions.get(id).unwrap().cfg.language, "en");
            assert!(sessions.get(id).unwrap().stream.is_none());
            assert!(cancelled.load(std::sync::atomic::Ordering::SeqCst));
            assert!(tx
                .send(CaptureContext {
                    target: None,
                    cfg: Default::default(),
                    completed: release
                })
                .is_err());
            drop(input);
        }
    }

    #[test]
    fn timely_context_uses_original_profile_snapshot() {
        let mut sessions = Sessions::default();
        let id = sessions.begin();
        let (tx, rx) = std::sync::mpsc::channel();
        sessions.get(id).unwrap().pending_context = Some(rx);
        let mut cfg = crate::config::Config::default();
        cfg.profile.mode = crate::profile::Mode::Literal;
        tx.send(CaptureContext {
            target: None,
            cfg,
            completed: Instant::now(),
        })
        .unwrap();
        sessions.resolve_capture(id, Instant::now());
        assert_eq!(
            sessions.get(id).unwrap().cfg.profile.mode,
            crate::profile::Mode::Literal
        );
        assert!(sessions.get(id).unwrap().pending_context.is_none());
    }

    #[test]
    fn cancellation_and_stale_results_never_insert() {
        let mut s = Sessions::default();
        let first = s.begin();
        s.cancel();
        assert!(!s.deliver(first, "x".into(), |_, _| panic!()).unwrap());
        let second = s.begin();
        let third = s.begin();
        assert_ne!(second, third);
        assert!(!s.deliver(second, "x".into(), |_, _| panic!()).unwrap());
    }
    #[test]
    fn success_is_exactly_once_and_failure_recoverable_not_replayed() {
        let mut s = Sessions::default();
        let id = s.begin();
        assert!(s
            .deliver(id, "a\nb".into(), |_, text| {
                assert_eq!(text, "a\nb");
                Ok(())
            })
            .unwrap());
        assert!(!s.deliver(id, "again".into(), |_, _| panic!()).unwrap());
        let id = s.begin();
        assert!(s
            .deliver(id, "saved".into(), |_, _| Err("failure".into()))
            .is_err());
        assert_eq!(s.recovery().as_deref(), Some("saved"));
        assert!(s.deliver(id, "saved".into(), |_, _| panic!()).is_err());
    }
    #[test]
    fn retry_is_bounded_and_expiring() {
        let mut s = Sessions::default();
        let id = s.begin();
        s.retain_audio(id, &[0.1; 10]);
        assert!(s.retry_audio().is_some());
        s.current.as_mut().unwrap().expires = Instant::now();
        assert!(s.retry_audio().is_none());
        assert!(s.recovery().is_none());
        let id = s.begin();
        s.retain_audio(id, &vec![0.; MAX_AUDIO + 1]);
        assert!(s.retry_audio().is_none());
    }
    #[test]
    fn focus_change_retains_text_without_consuming_delivery_attempt() {
        let mut sessions = Sessions::default();
        let id = sessions.begin();
        let result = sessions.deliver_checked(
            id,
            "line one\nline two".into(),
            |_| Err("focus moved".into()),
            |_, _| panic!("must not insert"),
        );
        assert!(result.is_err());
        assert_eq!(sessions.recovery().as_deref(), Some("line one\nline two"));
        assert!(sessions
            .deliver_checked(id, "line one\nline two".into(), |_| Ok(()), |_, _| Ok(()))
            .unwrap());
        assert!(!sessions
            .deliver_checked(id, "duplicate".into(), |_| panic!(), |_, _| panic!())
            .unwrap());
    }
}
