//! Whisper inference via whisper-rs (whisper.cpp, Metal-accelerated on macOS),
//! plus pause-based punctuation: a silence of `pause_ms` between words becomes
//! a period, and the next word is capitalized. Smaller Whisper models often
//! skip punctuation entirely; this recovers sentence structure from how the
//! user actually spoke.

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::config::Config;

pub struct Engine {
    ctx: WhisperContext,
    #[allow(dead_code)]
    cfg: Config,
}

struct Token {
    text: String,
    /// Start/end time in 10 ms units (whisper.cpp convention).
    t0: i64,
    t1: i64,
}

impl Engine {
    pub fn load(model_path: &str, cfg: &Config) -> Result<Self, String> {
        let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
            .map_err(|e| format!("failed to load model: {e}"))?;
        Ok(Self {
            ctx,
            cfg: cfg.clone(),
        })
    }

    /// Transcribe 16 kHz mono f32 PCM.
    #[allow(dead_code)]
    pub fn transcribe(&self, samples: &[f32]) -> Result<String, String> {
        self.transcribe_config(samples, &self.cfg)
    }

    pub fn transcribe_config(&self, samples: &[f32], cfg: &Config) -> Result<String, String> {
        recover_transcription(|| self.transcribe_inner(samples, cfg))
    }
    fn transcribe_inner(&self, samples: &[f32], cfg: &Config) -> Result<String, String> {
        let language = match cfg.language.as_str() {
            "auto" | "" => None,
            l => Some(l),
        };
        let pause_ms = if cfg.profile.mode == crate::profile::Mode::Literal {
            0
        } else {
            cfg.pause_punctuation_ms as i64
        };
        let prompt = (!cfg.vocabulary.is_empty())
            .then(|| format!("Glossary: {}.", cfg.vocabulary.join(", ")));
        let setup_start = std::time::Instant::now();
        let mut state = self.ctx.create_state().map_err(|e| e.to_string())?;
        log::info!(
            "asr_stage engine=whisper stage=state_setup ms={:.3}",
            setup_start.elapsed().as_secs_f64() * 1000.
        );
        let mut params = FullParams::new(SamplingStrategy::BeamSearch {
            beam_size: 5,
            patience: -1.0,
        });
        params.set_language(language);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        params.set_token_timestamps(pause_ms > 0);
        if let Some(prompt) = &prompt {
            params.set_initial_prompt(prompt);
        }

        let inference_start = std::time::Instant::now();
        state.full(params, samples).map_err(|e| e.to_string())?;
        log::info!(
            "asr_stage engine=whisper stage=inference_including_decoder_fallbacks ms={:.3}",
            inference_start.elapsed().as_secs_f64() * 1000.
        );
        let formatting_start = std::time::Instant::now();

        let n = state.full_n_segments().map_err(|e| e.to_string())?;
        let english = language.or_else(|| {
            state
                .full_lang_id_from_state()
                .ok()
                .and_then(whisper_rs::get_lang_str)
        }) == Some("en");

        if pause_ms == 0 {
            let mut text = String::new();
            for i in 0..n {
                let seg = state.full_get_segment_text(i).map_err(|e| e.to_string())?;
                text.push_str(seg.trim());
                text.push(' ');
            }
            let result =
                cfg.profile
                    .format(text.trim(), english, cfg.remove_fillers, cfg.auto_lists);
            log::info!(
                "asr_stage engine=whisper stage=formatting ms={:.3}",
                formatting_start.elapsed().as_secs_f64() * 1000.
            );
            return Ok(result);
        }

        let mut tokens = Vec::new();
        for seg in 0..n {
            let n_tokens = state.full_n_tokens(seg).map_err(|e| e.to_string())?;
            for i in 0..n_tokens {
                let text = state
                    .full_get_token_text(seg, i)
                    .map_err(|e| e.to_string())?;
                // Skip special/marker tokens like <|en|> or [_BEG_].
                if text.starts_with("<|") || text.starts_with("[_") {
                    continue;
                }
                let data = state
                    .full_get_token_data(seg, i)
                    .map_err(|e| e.to_string())?;
                tokens.push(Token {
                    text,
                    t0: data.t0,
                    t1: data.t1,
                });
            }
        }

        let result = cfg.profile.format(
            &build_raw_text(&tokens, pause_ms),
            english,
            cfg.remove_fillers,
            cfg.auto_lists,
        );
        log::info!(
            "asr_stage engine=whisper stage=formatting ms={:.3}",
            formatting_start.elapsed().as_secs_f64() * 1000.
        );
        Ok(result)
    }
}

/// Catch Rust inference/formatting panics before they unwind through the caller's
/// engine mutex. The caller receives an error and still completes the session.
/// Native aborts cannot be recovered this way.
fn recover_transcription(f: impl FnOnce() -> Result<String, String>) -> Result<String, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err("transcription panicked; please retry".into()))
}

/// Join tokens into text, turning speech pauses into sentence breaks.
#[cfg(test)]
fn build_text(
    tokens: &[Token],
    pause_ms: i64,
    english: bool,
    fillers: bool,
    lists: bool,
) -> String {
    post_process(&build_raw_text(tokens, pause_ms), english, fillers, lists)
}
fn build_raw_text(tokens: &[Token], pause_ms: i64) -> String {
    let mut out = String::new();
    let mut last_t1: Option<i64> = None;
    let mut capitalize = true;

    for tok in tokens {
        if let Some(prev_end) = last_t1 {
            let gap_ms = (tok.t0 - prev_end) * 10;
            if gap_ms >= pause_ms {
                let trimmed = out.trim_end();
                if trimmed.ends_with([',', ':', ';']) {
                    // The model heard a clause break, but the speaker paused
                    // like it's a sentence break — the pause wins.
                    out.truncate(trimmed.len() - 1);
                    out.push('.');
                    capitalize = true;
                } else if !trimmed.is_empty() && !trimmed.ends_with(['.', '!', '?', '—']) {
                    out.truncate(trimmed.len());
                    out.push('.');
                    capitalize = true;
                }
            }
        }
        last_t1 = Some(tok.t1);

        if capitalize {
            out.push_str(&capitalize_first_alpha(&tok.text, &mut capitalize));
        } else {
            out.push_str(&tok.text);
        }
        if tok.text.trim_end().ends_with(['.', '!', '?']) {
            capitalize = true;
        }
    }

    out
}

/// Shared cleanup: filler removal, English "i" fix, list formatting,
/// terminal punctuation.
pub(crate) fn post_process(text: &str, english: bool, fillers: bool, lists: bool) -> String {
    let mut result = text.trim().to_string();
    if fillers && english {
        result = remove_filler_words(&result);
    }
    if english {
        result = fix_standalone_i(&result);
    }
    if lists && english {
        result = crate::lists::format_lists(&result);
    }
    if result.chars().last().is_some_and(|c| c.is_alphanumeric()) {
        result.push('.');
    }
    result
}

const FILLERS: &[&str] = &[
    "um", "uh", "er", "erm", "uhm", "umm", "uhh", "hmm", "mhm", "mmm",
];

/// Drop filler words, keeping sentence structure intact: punctuation a filler
/// carried moves to the previous word, and a filler that opened a sentence
/// passes its capital letter to the next word.
fn remove_filler_words(text: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut capitalize = false;

    for raw in text.split_whitespace() {
        // Strip only surrounding speech punctuation. Internal hyphens, digits,
        // quotes and identifier characters must not turn real words into fillers.
        let core = raw.trim_matches([',', '.', '!', '?', ';', ':']);
        let acronym = core.len() > 1 && core.chars().all(|c| c.is_ascii_uppercase());
        if !acronym && FILLERS.contains(&core.to_ascii_lowercase().as_str()) {
            if words.last().is_none_or(|w| w.ends_with(['.', '!', '?'])) {
                capitalize = true;
            }
            // "...word um." -> "...word."
            if let Some(p) = raw.chars().last().filter(|c| ['.', '!', '?'].contains(c)) {
                if let Some(last) = words.last_mut() {
                    if !last.ends_with(['.', '!', '?', ',']) {
                        last.push(p);
                    }
                }
            }
            continue;
        }
        if capitalize {
            let mut flag = true;
            words.push(capitalize_first_alpha(raw, &mut flag));
            capitalize = flag;
        } else {
            words.push(raw.to_string());
        }
    }
    words.join(" ")
}

/// Uppercase the first alphabetic character; clear the flag once one is found.
fn capitalize_first_alpha(text: &str, capitalize: &mut bool) -> String {
    let mut result = String::with_capacity(text.len());
    for c in text.chars() {
        if *capitalize && c.is_alphabetic() {
            result.extend(c.to_uppercase());
            *capitalize = false;
        } else {
            result.push(c);
        }
    }
    result
}

/// English nicety: standalone "i" (and contractions like "i'm") -> "I".
fn fix_standalone_i(text: &str) -> String {
    text.split(' ')
        .map(|word| {
            let Some(rest) = word.strip_prefix('i') else {
                return word.to_string();
            };
            let standalone =
                rest.is_empty() || rest.starts_with(['\'', '’', ',', '.', '!', '?', ';', ':']);
            if standalone {
                format!("I{rest}")
            } else {
                word.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(text: &str, t0: i64, t1: i64) -> Token {
        Token {
            text: text.into(),
            t0,
            t1,
        }
    }

    #[test]
    fn pause_becomes_period_and_capitalizes() {
        // 1.0 s gap between "world" and "this" (t in 10 ms units).
        let tokens = [
            tok(" hello", 0, 50),
            tok(" world", 50, 100),
            tok(" this", 200, 250),
            tok(" works", 250, 300),
        ];
        assert_eq!(
            build_text(&tokens, 700, true, false, false),
            "Hello world. This works."
        );
    }

    #[test]
    fn short_gap_does_not_break_sentence() {
        let tokens = [tok(" hello", 0, 50), tok(" world", 80, 130)];
        assert_eq!(build_text(&tokens, 700, true, false, false), "Hello world.");
    }

    #[test]
    fn sentence_punctuation_is_kept_at_pause() {
        let tokens = [
            tok(" really", 0, 50),
            tok("?", 50, 51),
            tok(" yes", 200, 250),
        ];
        assert_eq!(build_text(&tokens, 700, true, false, false), "Really? Yes.");
    }

    #[test]
    fn pause_upgrades_comma_to_period() {
        let tokens = [
            tok(" sentence", 0, 50),
            tok(" one,", 50, 100),
            tok(" sentence", 250, 300),
            tok(" two", 300, 350),
        ];
        assert_eq!(
            build_text(&tokens, 700, true, false, false),
            "Sentence one. Sentence two."
        );
    }

    #[test]
    fn comma_without_pause_is_kept() {
        let tokens = [
            tok(" great,", 0, 50),
            tok(" right", 80, 130),
            tok("?", 130, 131),
        ];
        assert_eq!(
            build_text(&tokens, 700, true, false, false),
            "Great, right?"
        );
    }

    #[test]
    fn fixes_standalone_i() {
        let tokens = [
            tok(" i", 0, 10),
            tok(" think", 10, 40),
            tok(" i'm", 40, 70),
            tok(" right", 70, 100),
        ];
        assert_eq!(
            build_text(&tokens, 700, true, false, false),
            "I think I'm right."
        );
    }

    #[test]
    fn fillers_are_removed() {
        assert_eq!(
            remove_filler_words("Um, hello there. I think, uh, this works."),
            "Hello there. I think, this works."
        );
        assert_eq!(remove_filler_words("so um yeah"), "so yeah");
        assert_eq!(remove_filler_words("It works um."), "It works.");
    }

    #[test]
    fn filler_lookalikes_are_kept() {
        assert_eq!(
            remove_filler_words("the umbrella and the error"),
            "the umbrella and the error"
        );
    }

    #[test]
    fn fillers_stripped_in_pipeline() {
        let tokens = [
            tok(" um", 0, 10),
            tok(" hello", 10, 40),
            tok(" world", 40, 70),
        ];
        assert_eq!(build_text(&tokens, 700, true, true, false), "Hello world.");
    }

    #[test]
    fn words_starting_with_i_untouched() {
        assert_eq!(fix_standalone_i("it is icy"), "it is icy");
        assert_eq!(fix_standalone_i("i, i. i"), "I, I. I");
    }

    #[test]
    fn unicode_words_survive_english_cleanup() {
        for text in [
            "éclair",
            "你好",
            "🙂 hello",
            "i like éclairs",
            "“hello” i’m here",
        ] {
            assert!(!fix_standalone_i(text).is_empty());
        }
        assert_eq!(
            fix_standalone_i("éclair 你好 🙂 i’m here"),
            "éclair 你好 🙂 I’m here"
        );
    }

    #[test]
    fn acronyms_identifiers_and_quoted_fillers_are_preserved() {
        let text = "Set ER to 5. Use U-H and uh_2. Say \"um\".";
        assert_eq!(remove_filler_words(text), text);
        assert_eq!(remove_filler_words("Um, this uh works."), "This works.");
    }

    #[test]
    fn english_rules_do_not_delete_foreign_words() {
        assert_eq!(
            post_process("Er geht um acht.", false, true, true),
            "Er geht um acht."
        );
        assert_eq!(
            post_process("First, eins. Second, zwei.", false, true, true),
            "First, eins. Second, zwei."
        );
    }

    #[test]
    fn panic_returns_error_without_poisoning_engine_lock() {
        let lock = std::sync::Mutex::new(());
        {
            let _guard = lock.lock().unwrap();
            assert!(recover_transcription(|| panic!("test formatting failure")).is_err());
        }
        let _guard = lock.lock().expect("engine mutex must remain usable");
        assert_eq!(
            recover_transcription(|| Ok("Next dictation.".into())).unwrap(),
            "Next dictation."
        );
    }

    #[test]
    fn pause_disabled_via_zero_is_handled_by_caller() {
        // build_text is only called when pause_ms > 0; still, a huge threshold
        // means no inserted periods.
        let tokens = [tok(" a", 0, 10), tok(" b", 500, 510)];
        assert_eq!(build_text(&tokens, 100_000, true, false, false), "A b.");
    }
}
