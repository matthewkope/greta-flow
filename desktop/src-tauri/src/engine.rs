use crate::{config::Config, parakeet, transcribe};
use std::sync::Arc;

pub enum Engine {
    Whisper(Box<transcribe::Engine>, String),
    Parakeet(Arc<parakeet::Engine>),
}
impl Engine {
    /// Let idle recovery release the app's model slot before potentially slow
    /// startup. A model swap must not wait for a broken old helper to recover.
    pub fn maintenance_worker(&self) -> Option<Arc<parakeet::Engine>> {
        match self {
            Self::Parakeet(engine) => Some(Arc::clone(engine)),
            Self::Whisper(_, _) => None,
        }
    }

    pub fn load(path: &str, cfg: &Config) -> Result<Self, String> {
        let start = std::time::Instant::now();
        let result = if cfg.model == crate::config::DEFAULT_MODEL {
            parakeet::Engine::load(path, cfg).map(|engine| Self::Parakeet(Arc::new(engine)))
        } else {
            transcribe::Engine::load(path, cfg)
                .map(|engine| Self::Whisper(Box::new(engine), cfg.model.clone()))
        };
        log::info!(
            "asr_stage stage=model_load ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.
        );
        result
    }
    pub fn transcribe(&self, samples: &[f32], cfg: &Config) -> Result<String, String> {
        match self {
            Self::Whisper(engine, model) if model == &cfg.model => {
                engine.transcribe_config(samples, cfg)
            }
            Self::Whisper(_, _) => {
                Err("Model changed during dictation; retry with the original model".into())
            }
            Self::Parakeet(_)
                if cfg.model != crate::config::DEFAULT_MODEL
                    || !matches!(cfg.language.as_str(), "en" | "auto") =>
            {
                Err("Model/language changed during dictation".into())
            }
            Self::Parakeet(engine) => {
                let start = std::time::Instant::now();
                let text = engine.transcribe(samples)?;
                log::info!(
                    "asr_stage engine=parakeet stage=inference_ipc ms={:.3}",
                    start.elapsed().as_secs_f64() * 1000.
                );
                let start = std::time::Instant::now();
                let result = cfg
                    .profile
                    .format(&text, true, cfg.remove_fillers, cfg.auto_lists);
                log::info!(
                    "asr_stage engine=parakeet stage=formatting ms={:.3}",
                    start.elapsed().as_secs_f64() * 1000.
                );
                Ok(result)
            }
        }
    }
}
