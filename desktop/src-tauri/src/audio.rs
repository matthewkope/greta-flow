//! Microphone capture on a dedicated thread.
//!
//! cpal's `Stream` is not `Send`, so a worker thread owns it and the rest of
//! the app talks to it over a channel. The stream only exists while recording,
//! so the macOS mic indicator is accurate.

use crate::locks::MutexExt;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

pub struct Recording {
    /// Mono PCM at `sample_rate`.
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub last_speech: Option<std::time::Instant>,
}

#[derive(Default)]
pub(crate) struct Capture {
    samples: Vec<f32>,
    last_speech: Option<std::time::Instant>,
}
impl Capture {
    fn invalidate(&mut self) {
        self.samples.clear();
        self.samples.push(f32::NAN);
        self.last_speech = None;
    }
}
/// Where captured mono blocks go besides the whole-recording buffer.
pub enum Sink {
    /// Dictation streaming: the full recording is still retained for fallback.
    Stream(crate::streaming::Input),
    /// Meeting capture: blocks are forwarded only, so memory stays bounded for
    /// long meetings (the 10-minute whole-recording cap does not apply).
    Meeting(crate::meeting::Input),
}
impl Sink {
    fn retains_recording(&self) -> bool {
        matches!(self, Self::Stream(_))
    }
    fn send(&self, samples: Vec<f32>, rate: u32) {
        match self {
            Self::Stream(input) => input.send(samples, rate),
            Self::Meeting(input) => input.send(samples, rate),
        }
    }
}
const PENDING: u8 = 0;
const DELIVERED: u8 = 1;
const CANCELLED: u8 = 2;
struct Started {
    reply: Sender<Result<u32, String>>,
    state: Arc<std::sync::atomic::AtomicU8>,
}
fn claim(state: &std::sync::atomic::AtomicU8, from: u8, to: u8) -> bool {
    state
        .compare_exchange(
            from,
            to,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .is_ok()
}
enum Cmd {
    Start(Option<Sink>, Option<Started>),
    Stop(Sender<Recording>),
}

pub struct Recorder {
    tx: Sender<Cmd>,
    start_timeout: std::time::Duration,
}

impl Recorder {
    pub fn spawn() -> Self {
        Self::spawn_with(
            |buffer, sink| build_stream(buffer, sink).map_err(|e| e.to_string()),
            std::time::Duration::from_secs(5),
        )
    }

    pub(crate) fn spawn_with<S: 'static>(
        mut open: impl FnMut(Arc<Mutex<Capture>>, Option<Sink>) -> Result<(S, u32), String>
            + Send
            + 'static,
        start_timeout: std::time::Duration,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<Cmd>();
        std::thread::spawn(move || {
            let mut stream: Option<S> = None;
            let buffer: Arc<Mutex<Capture>> = Arc::new(Mutex::new(Capture::default()));
            let mut rate = 16_000u32;

            while let Ok(cmd) = rx.recv() {
                match cmd {
                    Cmd::Start(sink, started) => {
                        if stream.is_some() {
                            if let Some(started) = started {
                                let _ = started
                                    .reply
                                    .send(Err("microphone is already in use".into()));
                            }
                            continue;
                        }
                        *buffer.lock_recover() = Capture::default();
                        let result = open(Arc::clone(&buffer), sink);
                        let reply = match result {
                            Ok((s, r)) => {
                                rate = r;
                                stream = Some(s);
                                Ok(r)
                            }
                            Err(e) => {
                                log::error!("failed to open microphone: {e}");
                                Err(format!("couldn't open the microphone: {e}"))
                            }
                        };
                        if let Some(started) = started {
                            if claim(&started.state, PENDING, DELIVERED) {
                                let _ = started.reply.send(reply);
                            } else if stream.take().is_some() {
                                log::warn!("microphone opened after the requester gave up");
                                *buffer.lock_recover() = Capture::default();
                            }
                        }
                    }
                    Cmd::Stop(reply) => {
                        stream = None; // drop closes the input stream
                        let capture = std::mem::take(&mut *buffer.lock_repair(Capture::invalidate));
                        let samples = capture.samples;
                        let last_speech = capture.last_speech;
                        let _ = reply.send(Recording {
                            samples,
                            sample_rate: rate,
                            last_speech,
                        });
                    }
                }
            }
        });
        Self { tx, start_timeout }
    }

    pub fn start(&self) {
        let _ = self.tx.send(Cmd::Start(None, None));
    }

    pub fn start_stream(&self, sink: crate::streaming::Input) {
        let _ = self.tx.send(Cmd::Start(Some(Sink::Stream(sink)), None));
    }

    /// Open the microphone for a meeting and report whether it opened, with
    /// the device sample rate. Captured audio is forwarded to `sink` only.
    pub fn start_meeting(&self, sink: crate::meeting::Input) -> Result<u32, String> {
        let (tx, reply) = mpsc::channel();
        let started = Started {
            reply: tx,
            state: Arc::new(std::sync::atomic::AtomicU8::new(PENDING)),
        };
        let state = Arc::clone(&started.state);
        self.tx
            .send(Cmd::Start(Some(Sink::Meeting(sink)), Some(started)))
            .map_err(|_| "audio thread unavailable".to_string())?;
        match reply.recv_timeout(self.start_timeout) {
            Ok(result) => result,
            Err(_) => {
                if claim(&state, PENDING, CANCELLED) {
                    Err("microphone did not respond".to_string())
                } else {
                    reply
                        .recv()
                        .unwrap_or_else(|_| Err("audio thread unavailable".to_string()))
                }
            }
        }
    }
    pub fn stop_async(&self) -> mpsc::Receiver<Recording> {
        let (reply_tx, reply_rx) = mpsc::channel();
        let _ = self.tx.send(Cmd::Stop(reply_tx));
        reply_rx
    }

    pub fn stop(&self) -> Recording {
        self.stop_async()
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap_or(Recording {
                samples: Vec::new(),
                sample_rate: 16_000,
                last_speech: None,
            })
    }
}

fn build_stream(
    buffer: Arc<Mutex<Capture>>,
    sink: Option<Sink>,
) -> Result<(cpal::Stream, u32), Box<dyn std::error::Error>> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or("no default input device")?;
    let name = device.name().unwrap_or_else(|_| "<unknown>".into());
    let config = device.default_input_config()?;
    let rate = config.sample_rate().0;
    if rate == 0 || rate > 192_000 {
        return Err("unsupported microphone sample rate".into());
    }
    let channels = config.channels() as usize;
    let format = config.sample_format();
    log::info!("opening input '{name}' rate={rate} ch={channels} fmt={format:?}");

    let cfg: cpal::StreamConfig = config.into();
    // Capture in whatever native sample format the device reports, converting
    // each frame to mono f32. Hardcoding f32 broke on devices (e.g. the
    // built-in mic) whose native format is integer PCM.
    let stream = match format {
        SampleFormat::F32 => build_typed::<f32>(&device, &cfg, channels, buffer, sink)?,
        SampleFormat::I16 => build_typed::<i16>(&device, &cfg, channels, buffer, sink)?,
        SampleFormat::U16 => build_typed::<u16>(&device, &cfg, channels, buffer, sink)?,
        SampleFormat::I8 => build_typed::<i8>(&device, &cfg, channels, buffer, sink)?,
        SampleFormat::I32 => build_typed::<i32>(&device, &cfg, channels, buffer, sink)?,
        SampleFormat::U8 => build_typed::<u8>(&device, &cfg, channels, buffer, sink)?,
        other => {
            return Err(format!("unsupported input sample format {other:?}").into());
        }
    };
    stream.play()?;
    Ok((stream, rate))
}

/// Build an input stream for a concrete sample type, downmixing interleaved
/// frames to mono f32 into `buffer`.
fn build_typed<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    buffer: Arc<Mutex<Capture>>,
    sink: Option<Sink>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let rate = config.sample_rate.0;
    let max_samples = rate as usize * 600;
    let retain = sink.as_ref().is_none_or(Sink::retains_recording);
    device.build_input_stream(
        config,
        move |data: &[T], _| {
            let mono: Vec<f32> = data
                .chunks(channels)
                .map(|frame| {
                    frame.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / channels as f32
                })
                .collect();
            if !retain {
                if let Some(sink) = &sink {
                    sink.send(mono, rate);
                }
                return;
            }
            let mut buf = buffer.lock_repair(Capture::invalidate);
            if mono.iter().map(|s| s * s).sum::<f32>() / mono.len().max(1) as f32
                > 0.0001f32.powi(2)
            {
                buf.last_speech = Some(std::time::Instant::now());
            }
            if buf.samples.len() + mono.len() > max_samples {
                // Sentinel invalidates the entire recording instead of silently truncating it.
                if !buf.samples.last().is_some_and(|v| v.is_nan()) {
                    buf.samples.push(f32::NAN);
                }
                return;
            }
            if buf.samples.last().is_some_and(|v| v.is_nan()) {
                return;
            }
            buf.samples.extend_from_slice(&mono);
            if let Some(sink) = &sink {
                sink.send(mono, rate);
            }
        },
        |err| log::error!("audio stream error: {err}"),
        None,
    )
}

/// Linear resample to 16 kHz mono for Whisper.
pub fn resample_to_16k(rec: &Recording) -> Vec<f32> {
    const TARGET: u32 = 16_000;
    if rec.sample_rate == TARGET || rec.samples.is_empty() {
        return rec.samples.clone();
    }
    let ratio = rec.sample_rate as f64 / TARGET as f64;
    let out_len = (rec.samples.len() as f64 / ratio) as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let a = rec.samples[idx.min(rec.samples.len() - 1)];
            let b = rec.samples[(idx + 1).min(rec.samples.len() - 1)];
            a + (b - a) * frac
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poisoned_capture_is_rejected_and_the_next_capture_is_usable() {
        let buffer = Mutex::new(Capture::default());
        let _ = std::panic::catch_unwind(|| {
            let mut capture = buffer.lock().unwrap();
            capture.samples.extend([0.1, 0.2]);
            panic!("interrupted audio callback");
        });
        let stopped = std::mem::take(&mut *buffer.lock_repair(Capture::invalidate));
        assert!(stopped.samples.iter().any(|sample| !sample.is_finite()));
        assert!(!buffer.is_poisoned());
        *buffer.lock_recover() = Capture::default();
        buffer.lock_recover().samples.push(0.3);
        assert_eq!(buffer.lock_recover().samples, vec![0.3]);
    }
}
