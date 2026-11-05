use crate::{config::Config, parakeet, transcribe};

pub enum Engine {
    Whisper(Box<transcribe::Engine>, String),
    Parakeet(parakeet::Engine),
}
impl Engine {
    pub fn load(path: &str, cfg: &Config) -> Result<Self, String> {
        let start = std::time::Instant::now();
        let result = if cfg.model == crate::config::DEFAULT_MODEL {
            parakeet::Engine::load(path, cfg).map(Self::Parakeet)
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
