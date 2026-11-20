//! Personal dictionary and spoken shortcuts.
//!
//! Both are deterministic transcript rewrites applied after recognition and
//! before delivery. Nothing here changes what the recognizer hears or trains
//! any model:
//!
//! - A dictionary entry corrects explicit misheard variants (and other casings
//!   of the canonical spelling) to one canonical spelling.
//! - A spoken shortcut replaces an explicit whole phrase with literal text such
//!   as an email address or URL.
//!
//! Matching is whole-term and boundary-aware (no substrings inside other
//! words), ASCII case-insensitive, and never fuzzy. The longest trigger at a
//! position wins; equal lengths prefer shortcuts, then dictionary entries, then
//! the active profile's spellings and vocabulary. Inserted text is never
//! scanned again, so expansions cannot cascade.
use serde::{Deserialize, Serialize};

pub const MAX_ENTRIES: usize = 500;
pub const MAX_TRIGGER_CHARS: usize = 100;
pub const MAX_TEXT_CHARS: usize = 2000;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DictionaryEntry {
    /// The spelling to deliver, preserved exactly.
    pub canonical: String,
    /// Explicit misheard forms to replace with `canonical`.
    pub variants: Vec<String>,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SpokenShortcut {
    /// The whole phrase to say, e.g. "my email".
    pub phrase: String,
    /// Literal text to insert. Empty means not configured yet; the phrase is
    /// then left as spoken.
    pub text: String,
    pub enabled: bool,
}

/// The user's global entries, attached to each dictation's profile snapshot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Personal {
    pub dictionary: Vec<DictionaryEntry>,
    pub shortcuts: Vec<SpokenShortcut>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Shortcut,
    Dictionary,
    Profile,
}

/// One match rule: a trigger and the exact text that replaces it.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub trigger: String,
    pub value: String,
    pub kind: Kind,
}

/// Collapse runs of whitespace so "my  email" and "my email" are one trigger.
pub fn normalize_trigger(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Personal {
    /// Enabled rules in precedence order (before length sorting).
    pub fn rules(&self) -> Vec<Rule> {
        let mut rules = vec![];
        for s in self.shortcuts.iter().filter(|s| s.enabled) {
            let trigger = normalize_trigger(&s.phrase);
            if trigger.is_empty() || s.text.trim().is_empty() {
                continue;
            }
            rules.push(Rule {
                trigger,
                value: s.text.clone(),
                kind: Kind::Shortcut,
            });
        }
        for e in self.dictionary.iter().filter(|e| e.enabled) {
            let canonical = e.canonical.trim();
            if canonical.is_empty() {
                continue;
            }
            for v in std::iter::once(canonical).chain(e.variants.iter().map(|v| v.as_str())) {
                let trigger = normalize_trigger(v);
                if trigger.is_empty() {
                    continue;
                }
                rules.push(Rule {
                    trigger,
                    value: canonical.to_string(),
                    kind: Kind::Dictionary,
                });
            }
        }
        rules
    }
}

fn has_word_char(s: &str) -> bool {
    s.chars().any(char::is_alphanumeric)
}

/// Validate and normalize user-edited lists before they are saved.
pub fn validate(
    dictionary: Vec<DictionaryEntry>,
    shortcuts: Vec<SpokenShortcut>,
) -> Result<Personal, String> {
    if dictionary.len() + shortcuts.len() > MAX_ENTRIES {
        return Err(format!("At most {MAX_ENTRIES} entries are supported."));
    }
    let mut seen: Vec<(String, String)> = vec![];
    let mut claim = |trigger: &str, owner: String, enabled: bool| -> Result<(), String> {
        if !enabled {
            return Ok(());
        }
        let key = trigger.to_ascii_lowercase();
        if let Some((_, other)) = seen.iter().find(|(k, _)| *k == key) {
            if *other != owner {
                return Err(format!(
                    "\"{trigger}\" is used by both {other} and {owner}."
                ));
            }
            return Ok(());
        }
        seen.push((key, owner));
        Ok(())
    };
    let mut out = Personal::default();
    for s in shortcuts {
        let phrase = normalize_trigger(&s.phrase);
        if phrase.is_empty() {
            return Err("Every spoken shortcut needs a phrase.".into());
        }
        if !has_word_char(&phrase) || phrase.chars().count() > MAX_TRIGGER_CHARS {
            return Err(format!(
                "\"{phrase}\" must contain a word and be at most {MAX_TRIGGER_CHARS} characters."
            ));
        }
        if s.text.chars().count() > MAX_TEXT_CHARS {
            return Err(format!(
                "The text for \"{phrase}\" is longer than {MAX_TEXT_CHARS} characters."
            ));
        }
        claim(
            &phrase,
            format!("shortcut \"{phrase}\""),
            s.enabled && !s.text.trim().is_empty(),
        )?;
        out.shortcuts.push(SpokenShortcut {
            phrase,
            text: if s.text.trim().is_empty() {
                String::new()
            } else {
                s.text
            },
            enabled: s.enabled,
        });
    }
    for e in dictionary {
        let canonical = e.canonical.trim().to_string();
        if canonical.is_empty() {
            return Err("Every dictionary entry needs a correct spelling.".into());
        }
        if !has_word_char(&canonical) || canonical.chars().count() > MAX_TRIGGER_CHARS {
            return Err(format!(
                "\"{canonical}\" must contain a word and be at most {MAX_TRIGGER_CHARS} characters."
            ));
        }
        let owner = format!("dictionary entry \"{canonical}\"");
        claim(&normalize_trigger(&canonical), owner.clone(), e.enabled)?;
        let mut variants: Vec<String> = vec![];
        for v in e.variants {
            let v = normalize_trigger(&v);
            if v.is_empty()
                || v.eq_ignore_ascii_case(&normalize_trigger(&canonical))
                || variants.iter().any(|x| x.eq_ignore_ascii_case(&v))
            {
                continue;
            }
            if !has_word_char(&v) || v.chars().count() > MAX_TRIGGER_CHARS {
                return Err(format!(
                    "\"{v}\" must contain a word and be at most {MAX_TRIGGER_CHARS} characters."
                ));
            }
            claim(&v, owner.clone(), e.enabled)?;
            variants.push(v);
        }
        out.dictionary.push(DictionaryEntry {
            canonical,
            variants,
            enabled: e.enabled,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{Mode, Profile};
    use std::collections::BTreeMap;

    fn entry(canonical: &str, variants: &[&str]) -> DictionaryEntry {
        DictionaryEntry {
            canonical: canonical.into(),
            variants: variants.iter().map(|s| s.to_string()).collect(),
            enabled: true,
        }
    }
    fn shortcut(phrase: &str, text: &str) -> SpokenShortcut {
        SpokenShortcut {
            phrase: phrase.into(),
            text: text.into(),
            enabled: true,
        }
    }
    fn profile(dictionary: Vec<DictionaryEntry>, shortcuts: Vec<SpokenShortcut>) -> Profile {
        Profile {
            personal: Personal {
                dictionary,
                shortcuts,
            },
            ..Default::default()
        }
    }
    fn fmt(p: &Profile, s: &str) -> String {
        p.format(s, true, true, false)
    }

    #[test]
    fn dictionary_corrects_variants_to_exact_canonical_casing() {
        let p = profile(
            vec![entry("Dana Kope", &["Dana Cope", "Dana Cobe"])],
            vec![],
        );
        assert_eq!(fmt(&p, "Hi, I'm Dana Cope."), "Hi, I'm Dana Kope.");
        assert_eq!(fmt(&p, "dana cobe called"), "Dana Kope called.");
        assert_eq!(fmt(&p, "ask DANA KOPE"), "ask Dana Kope.");
        assert_eq!(fmt(&p, "Dana Cope's notes"), "Dana Kope's notes.");
    }

    #[test]
    fn matching_is_boundary_aware_and_never_fuzzy() {
        let p = profile(
            vec![entry("Kope", &["Cope"])],
            vec![shortcut("my email", "user@example.com")],
        );
        // No substrings inside other words, identifiers, paths or addresses.
        assert_eq!(
            fmt(&p, "Copenhagen, telescope, cope_x and a/cope"),
            "Copenhagen, telescope, cope_x and a/cope."
        );
        assert_eq!(fmt(&p, "myemail and my emails"), "myemail and my emails.");
        // Near misses are not rewritten.
        assert_eq!(fmt(&p, "I can cape"), "I can cape.");
        assert_eq!(fmt(&p, "I can cope"), "I can Kope.");
    }

    #[test]
    fn shortcut_inserts_literal_text_inside_sentences_and_alone() {
        let p = profile(
            vec![],
            vec![
                shortcut("my email", "user.name@example.com"),
                shortcut("my site", "https://example.com/u/Name_1?x=A&y=b"),
            ],
        );
        assert_eq!(
            fmt(&p, "Send it to my email please"),
            "Send it to user.name@example.com please."
        );
        // A whole utterance that is only a shortcut inserts exactly the text.
        assert_eq!(fmt(&p, "My email."), "user.name@example.com");
        assert_eq!(
            fmt(&p, "  my site  "),
            "https://example.com/u/Name_1?x=A&y=b"
        );
        // Not capitalized at sentence start, and fillers around it still drop.
        assert_eq!(
            fmt(&p, "um my site is up"),
            "https://example.com/u/Name_1?x=A&y=b is up."
        );
    }

    #[test]
    fn unconfigured_or_disabled_entries_leave_speech_alone() {
        let mut off = shortcut("my email", "user@example.com");
        off.enabled = false;
        let mut dict_off = entry("Dana Kope", &["Dana Cope"]);
        dict_off.enabled = false;
        let p = profile(vec![dict_off], vec![shortcut("my profile", ""), off]);
        assert_eq!(
            fmt(&p, "my profile and my email for Dana Cope"),
            "my profile and my email for Dana Cope."
        );
    }

    #[test]
    fn longest_match_then_kind_precedence_and_no_recursion() {
        let p = Profile {
            spellings: BTreeMap::from([("my email".into(), "SPELLING".into())]),
            personal: Personal {
                dictionary: vec![entry("Work Mail", &["my email"]), entry("Mail", &["email"])],
                shortcuts: vec![
                    shortcut("my email", "a@example.com my email email"),
                    shortcut("my email address", "b@example.com"),
                    shortcut("comma", "x"),
                ],
            },
            spoken_commands: true,
            ..Default::default()
        };
        // Longest trigger wins.
        assert_eq!(fmt(&p, "use my email address"), "use b@example.com.");
        // A user shortcut takes its phrase away from the built-in command.
        assert_eq!(fmt(&p, "a comma b"), "a x b.");
        // Equal length: shortcut beats dictionary and profile spelling;
        // the inserted text is not expanded again.
        assert_eq!(
            fmt(&p, "use my email now"),
            "use a@example.com my email email now."
        );
        // A shortcut takes its phrase from spoken commands, and command words
        // inside inserted text are not executed.
        let q = Profile {
            personal: Personal {
                dictionary: vec![],
                shortcuts: vec![shortcut("sig", "Thanks comma new paragraph Dana")],
            },
            spoken_commands: true,
            ..Default::default()
        };
        assert_eq!(
            fmt(&q, "ok comma sig"),
            "ok, Thanks comma new paragraph Dana."
        );
    }

    #[test]
    fn literal_mode_is_untouched() {
        let mut p = profile(
            vec![entry("Dana Kope", &["Dana Cope"])],
            vec![shortcut("my email", "user@example.com")],
        );
        p.mode = Mode::Literal;
        assert_eq!(fmt(&p, "Dana Cope my email"), "Dana Cope my email");
    }

    #[test]
    fn marker_like_text_in_values_and_transcript_is_preserved() {
        let p = profile(vec![], vec![shortcut("tag", "GFPROTECTED1Z")]);
        assert_eq!(
            fmt(&p, "GFPROTECTED0Z tag tag"),
            "GFPROTECTED0Z GFPROTECTED1Z GFPROTECTED1Z."
        );
    }

    #[test]
    fn validation_normalizes_and_rejects_ambiguous_triggers() {
        let ok = validate(
            vec![entry(
                "  Dana Kope ",
                &["dana  cope", "Dana Cope", "", "DANA KOPE"],
            )],
            vec![
                shortcut(" my   email ", "   "),
                shortcut("my site", "https://example.com"),
            ],
        )
        .unwrap();
        assert_eq!(ok.dictionary[0].canonical, "Dana Kope");
        assert_eq!(ok.dictionary[0].variants, vec!["dana cope".to_string()]);
        assert_eq!(ok.shortcuts[0].phrase, "my email");
        assert_eq!(ok.shortcuts[0].text, "");
        // The same trigger owned by two enabled entries is rejected.
        assert!(validate(
            vec![entry("A", &["my site"])],
            vec![shortcut("My Site", "https://example.com")]
        )
        .is_err());
        assert!(validate(vec![entry("A", &["x y"]), entry("B", &["X Y"])], vec![]).is_err());
        // An unconfigured or disabled shortcut does not claim its phrase.
        assert!(validate(
            vec![entry("A", &["my site"])],
            vec![shortcut("my site", "")]
        )
        .is_ok());
        assert!(validate(vec![entry("", &["x"])], vec![]).is_err());
        assert!(validate(vec![], vec![shortcut("", "x")]).is_err());
        assert!(validate(vec![], vec![shortcut("...", "x")]).is_err());
        assert!(validate(vec![], vec![shortcut("a", &"x".repeat(MAX_TEXT_CHARS + 1))]).is_err());
    }

    #[test]
    fn config_round_trips_without_personal_defaults() {
        let cfg = crate::config::Config::default();
        assert!(cfg.dictionary.is_empty());
        assert!(cfg.spoken_shortcuts.is_empty());
        let json = r#"{"dictionary":[{"canonical":"Dana Kope","variants":["Dana Cope"],"enabled":true}],
            "spoken_shortcuts":[{"phrase":"my email","text":"user@example.com","enabled":true}]}"#;
        let cfg: crate::config::Config = serde_json::from_str(json).unwrap();
        let snap = cfg.for_app(None);
        assert_eq!(
            fmt(&snap.profile, "email Dana Cope at my email"),
            "email Dana Kope at user@example.com."
        );
        let back = serde_json::to_value(&cfg).unwrap();
        assert_eq!(back["dictionary"][0]["canonical"], "Dana Kope");
        assert_eq!(back["spoken_shortcuts"][0]["text"], "user@example.com");
        // Per-app profiles still receive the global entries.
        let mut cfg = cfg;
        cfg.profiles.insert("chat".into(), Profile::default());
        cfg.app_profiles
            .insert("com.example.chat".into(), "chat".into());
        let snap = cfg.for_app(Some("com.example.chat"));
        assert_eq!(fmt(&snap.profile, "Dana Cope"), "Dana Kope.");
    }
}
