//! Local monotonic stage timings. Never includes text, vocabulary, or audio.
use std::{collections::BTreeMap, time::Instant};
#[derive(Debug)]
pub struct Timing {
    release: Instant,
    last_speech: Option<Instant>,
    pub stages_ms: BTreeMap<&'static str, f64>,
}
impl Timing {
    pub fn new(release: Instant) -> Self {
        Self {
            release,
            last_speech: None,
            stages_ms: BTreeMap::new(),
        }
    }
    pub fn stage(&mut self, name: &'static str, start: Instant) {
        self.stages_ms
            .insert(name, start.elapsed().as_secs_f64() * 1000.);
    }
    pub fn speech(&mut self, speech: Option<Instant>) {
        self.last_speech = speech;
    }
    pub fn mark(&mut self, name: &'static str) {
        self.stage(name, self.release);
    }
    pub fn inserted(&mut self) {
        if let Some(speech) = self.last_speech {
            self.stage("speech_end_to_inserted", speech);
        }
    }
    pub fn emit(&self, outcome: &str) {
        let speech_ms = self.last_speech.map(|t| t.elapsed().as_secs_f64() * 1000.);
        log::info!(
            "pipeline_timing {}",
            serde_json::json!({"version":1,"outcome":outcome,"stages_ms":self.stages_ms,"speech_end_to_completion_ms":speech_ms,"speech_end_estimator":"energy_threshold"})
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn records_monotonic_nonnegative_durations() {
        let mut m = Timing::new(Instant::now());
        m.mark("final_text");
        assert!(m.stages_ms["final_text"] >= 0.);
        assert!(m.last_speech.is_none());
    }
}
