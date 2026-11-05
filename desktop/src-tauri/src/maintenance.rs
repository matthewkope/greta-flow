//! Rate-limit failed background helper recovery, independently of user retries.
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Recovery {
    generation: u64,
    failures: u32,
    retry_at: Option<Instant>,
}
impl Recovery {
    pub fn due(&mut self, generation: u64, now: Instant) -> bool {
        if self.generation != generation {
            *self = Self {
                generation,
                ..Self::default()
            };
        }
        self.retry_at.is_none_or(|deadline| now >= deadline)
    }
    pub fn succeeded(&mut self) {
        self.failures = 0;
        self.retry_at = None;
    }
    pub fn failed(&mut self, now: Instant) -> Duration {
        self.failures = self.failures.saturating_add(1);
        // 30, 60, 120, 240, then 300 seconds; measured from completion, so a
        // startup timeout cannot consume the cooldown and immediately retry.
        let delay = Duration::from_secs((30u64 << (self.failures - 1).min(4)).min(300));
        self.retry_at = Some(now + delay);
        delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_failures_back_off_and_a_new_model_is_not_delayed() {
        let mut recovery = Recovery::default();
        let mut now = Instant::now();
        for seconds in [30, 60, 120, 240, 300, 300] {
            assert!(recovery.due(1, now));
            // Simulate a slow failed load, followed by a full cooldown.
            now += Duration::from_secs(180);
            assert_eq!(recovery.failed(now), Duration::from_secs(seconds));
            assert!(!recovery.due(1, now + Duration::from_secs(5)));
            now += Duration::from_secs(seconds);
        }
        recovery.failed(now);
        assert!(recovery.due(2, now));
        assert_eq!(recovery.failed(now), Duration::from_secs(30));
    }

    #[test]
    fn successful_recovery_restores_normal_health_checks() {
        let mut recovery = Recovery::default();
        let now = Instant::now();
        recovery.failed(now);
        recovery.succeeded();
        assert!(recovery.due(0, now));
        assert_eq!(recovery.failed(now), Duration::from_secs(30));
    }
}
