//! Conservative silence-boundary recognition while capture continues.
//! Continuous speech over 30s or a full queue falls back to one whole-clip decode.
use crate::{config::Config, engine::Engine};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender},
    Arc, Mutex,
};
const RATE: usize = 16_000;
const MAX_SEGMENT: usize = 30 * RATE;
const SILENCE: usize = RATE; // One second of sustained silence, independent of punctuation.
const PAD: usize = RATE / 5;

#[derive(Clone)]
pub struct Input {
    tx: SyncSender<(Vec<f32>, u32)>,
    failed: Arc<AtomicBool>,
}
impl Input {
    pub fn send(&self, samples: Vec<f32>, rate: u32) {
        if samples.len() > 192_000 || self.tx.try_send((samples, rate)).is_err() {
            self.failed.store(true, Ordering::SeqCst);
        }
    }
}
pub struct Worker {
    done: Receiver<Result<String, String>>,
    pub cancelled: Arc<AtomicBool>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}
impl Worker {
    pub fn spawn(engine: Arc<Mutex<Option<Engine>>>, cfg: Config) -> (Input, Self) {
        let (tx, rx) = mpsc::sync_channel::<(Vec<f32>, u32)>(32);
        let (result_tx, done) = mpsc::channel();
        let failed = Arc::new(AtomicBool::new(false));
        let overflow = failed.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = cancelled.clone();
        std::thread::spawn(move || {
            let mut resampler = Resampler::default();
            let mut segmenter = Segmenter::default();
            let mut parts = vec![];
            let mut raw_cfg = cfg.clone();
            raw_cfg.profile.mode = crate::profile::Mode::Literal;
            let decode = |samples: Vec<f32>, parts: &mut Vec<String>| -> Result<(), String> {
                if cancel.load(Ordering::SeqCst) {
                    return Err("stream cancelled".into());
                }
                let samples = prepare(&samples);
                if samples.is_empty() {
                    return Ok(());
                }
                let lock = engine.lock().map_err(|_| "engine lock failed")?;
                let text = lock
                    .as_ref()
                    .ok_or("no engine")?
                    .transcribe(&samples, &raw_cfg)?;
                if !text.trim().is_empty() {
                    parts.push(text.trim().to_string());
                }
                Ok(())
            };
            let result = (|| {
                for (samples, rate) in rx {
                    if cancel.load(Ordering::SeqCst) || overflow.load(Ordering::SeqCst) {
                        return Err("stream cancelled or overflowed".into());
                    }
                    for chunk in segmenter.push(&resampler.push(&samples, rate)?)? {
                        decode(chunk, &mut parts)?;
                    }
                }
                if overflow.load(Ordering::SeqCst) {
                    return Err("stream queue overflow".into());
                }
                for chunk in segmenter.push(&resampler.finish())? {
                    decode(chunk, &mut parts)?;
                }
                if let Some(tail) = segmenter.finish() {
                    decode(tail, &mut parts)?;
                }
                if cancel.load(Ordering::SeqCst) {
                    return Err("stream cancelled".into());
                }
                Ok(cfg.profile.format(
                    &parts.join(" "),
                    cfg.language == "en" || cfg.model == crate::config::DEFAULT_MODEL,
                    cfg.remove_fillers,
                    cfg.auto_lists,
                ))
            })();
            let _ = result_tx.send(result);
        });
        (Input { tx, failed }, Self { done, cancelled })
    }
    pub fn finish(&self) -> Result<String, String> {
        self.done
            .recv_timeout(std::time::Duration::from_secs(65))
            .map_err(|_| "stream completion timed out")?
    }
}
/// Identical gain/padding for batch and incremental paths. Gate only near-digital silence.
pub fn prepare(samples: &[f32]) -> Vec<f32> {
    if samples.is_empty() || samples.iter().any(|v| !v.is_finite()) {
        return vec![];
    }
    let rms = (samples.iter().map(|v| v * v).sum::<f32>() / samples.len() as f32).sqrt();
    if rms < 0.0001 {
        return vec![];
    }
    let gain = (0.05 / rms).clamp(1., 12.);
    let mut out: Vec<f32> = samples.iter().map(|v| (v * gain).clamp(-1., 1.)).collect();
    out.resize(out.len().max(17_600), 0.);
    out
}
#[derive(Default)]
pub struct Segmenter {
    buffer: Vec<f32>,
    tail: usize,
    voiced: bool,
    frame: Vec<f32>,
}
impl Segmenter {
    pub fn push(&mut self, samples: &[f32]) -> Result<Vec<Vec<f32>>, String> {
        let mut out = vec![];
        for &s in samples {
            self.frame.push(s);
            if self.frame.len() < 160 {
                continue;
            }
            let voice = self.frame.iter().map(|v| v * v).sum::<f32>() / 160. > 0.0001f32.powi(2);
            self.buffer.append(&mut self.frame);
            if voice {
                self.voiced = true;
                self.tail = 0;
            } else {
                self.tail += 160;
            }
            if self.voiced && self.tail >= SILENCE {
                // The omitted middle of the silence carries no speech. Keep padding on both sides.
                let end = self.buffer.len() - self.tail + PAD;
                out.push(self.buffer[..end].to_vec());
                self.buffer = self.buffer[self.buffer.len() - PAD..].to_vec();
                self.voiced = false;
                self.tail = PAD;
            } else if !self.voiced && self.buffer.len() > PAD {
                self.buffer.drain(..self.buffer.len() - PAD);
            }
            if self.buffer.len() > MAX_SEGMENT {
                return Err("continuous speech requires whole-clip fallback".into());
            }
        }
        Ok(out)
    }
    pub fn finish(&mut self) -> Option<Vec<f32>> {
        self.buffer.append(&mut self.frame);
        if self.voiced || self.buffer.iter().any(|v| v.abs() > 0.0001) {
            Some(std::mem::take(&mut self.buffer))
        } else {
            None
        }
    }
}
/// Streaming linear interpolator retains a boundary sample and global phase.
#[derive(Default)]
pub struct Resampler {
    rate: u32,
    samples: Vec<f32>,
    base: usize,
    next: usize,
}
impl Resampler {
    pub fn push(&mut self, input: &[f32], rate: u32) -> Result<Vec<f32>, String> {
        if rate == 0 || rate > 192_000 || (self.rate != 0 && rate != self.rate) {
            return Err("invalid or changed sample rate".into());
        }
        self.rate = rate;
        self.samples.extend_from_slice(input);
        let mut out = vec![];
        loop {
            let numerator = self.next as u64 * rate as u64;
            let index = (numerator / 16_000) as usize;
            if index + 1 >= self.base + self.samples.len() {
                break;
            }
            let fraction = (numerator % 16_000) as f32 / 16_000.;
            let i = index - self.base;
            out.push(self.samples[i] + (self.samples[i + 1] - self.samples[i]) * fraction);
            self.next += 1;
        }
        let keep = ((self.next as u64 * rate as u64) / 16_000) as usize;
        let drain = keep
            .saturating_sub(self.base)
            .min(self.samples.len().saturating_sub(1));
        self.samples.drain(..drain);
        self.base += drain;
        Ok(out)
    }
    pub fn finish(&mut self) -> Vec<f32> {
        if self.rate == 0 || self.samples.is_empty() {
            return vec![];
        }
        let length = (self.base + self.samples.len()) as u64 * 16_000 / self.rate as u64;
        let mut out = vec![];
        while (self.next as u64) < length {
            out.push(*self.samples.last().unwrap());
            self.next += 1;
        }
        out
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunk_boundaries_do_not_change_resampling() {
        for rate in [16000, 44100, 48000] {
            let samples: Vec<f32> = (0..10001).map(|n| (n as f32 * 0.1).sin()).collect();
            let mut a = Resampler::default();
            let mut expected = a.push(&samples, rate).unwrap();
            expected.extend(a.finish());
            let mut b = Resampler::default();
            let mut actual = vec![];
            for part in samples.chunks(137) {
                actual.extend(b.push(part, rate).unwrap());
            }
            actual.extend(b.finish());
            assert_eq!(actual, expected);
            assert!(b.samples.len() <= 2);
        }
    }
    #[test]
    fn silent_audio_never_reaches_decoder_and_quiet_speech_survives() {
        let mut s = Segmenter::default();
        assert!(s.push(&vec![0.; 160_000]).unwrap().is_empty());
        assert!(s.finish().is_none());
        assert!(s.push(&vec![0.0002; 1600]).unwrap().is_empty());
        assert_eq!(
            s.finish().unwrap().iter().filter(|v| **v == 0.0002).count(),
            1600
        );
    }
    #[test]
    fn cuts_only_silence_no_voiced_samples_lost_or_duplicated() {
        let mut source = vec![0.1; 16000];
        source.extend(vec![0.; 24000]);
        source.extend(vec![0.2; 16000]);
        let mut s = Segmenter::default();
        let mut output = vec![];
        for chunk in source.chunks(113) {
            for segment in s.push(chunk).unwrap() {
                output.extend(segment);
            }
        }
        output.extend(s.finish().unwrap());
        for value in [0.1, 0.2] {
            assert_eq!(output.iter().filter(|v| **v == value).count(), 16000);
        }
    }
    #[test]
    fn bounded_continuous_speech_requests_fallback() {
        assert!(Segmenter::default()
            .push(&vec![0.1; MAX_SEGMENT + 160])
            .is_err());
    }
    #[test]
    fn backpressure_requests_fallback_instead_of_blocking_capture() {
        let (tx, _rx) = mpsc::sync_channel(1);
        let failed = Arc::new(AtomicBool::new(false));
        let input = Input {
            tx,
            failed: failed.clone(),
        };
        input.send(vec![0.; 160], 16000);
        input.send(vec![0.; 160], 16000);
        assert!(failed.load(Ordering::SeqCst));
    }
    #[test]
    fn cancelled_worker_discards_queued_audio() {
        let (input, worker) = Worker::spawn(Arc::new(Mutex::new(None)), Config::default());
        worker.cancelled.store(true, Ordering::SeqCst);
        input.send(vec![0.1; 1600], 16000);
        drop(input);
        assert!(worker.finish().is_err());
    }
}
