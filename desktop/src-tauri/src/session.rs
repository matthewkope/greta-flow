//! One delivery per session, with bounded, expiring, memory-only recovery.
use crate::inject::Target;
use std::time::{Duration, Instant};
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
        job.text = Some(text.clone());
        if job.attempted {
            return Err("Delivery already attempted; use recovery text to avoid duplicates".into());
        }
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
