//! Per-session formatting. No model reloads, fuzzy substitutions, or external services.
use crate::dictionary::{Kind, Personal, Rule};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Prose,
    Literal,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub mode: Mode,
    pub vocabulary: Vec<String>,
    /// Explicit whole-term corrections, shared by both engines.
    pub spellings: BTreeMap<String, String>,
    pub spoken_commands: bool,
    /// Global personal dictionary and spoken shortcuts, attached per dictation
    /// by `Config::for_app`; stored at the top level of the config file.
    #[serde(skip)]
    pub personal: Personal,
}
impl Profile {
    pub fn format(&self, raw: &str, english: bool, fillers: bool, lists: bool) -> String {
        if self.mode == Mode::Literal {
            return raw.to_string();
        }
        // Precedence for equal-length triggers follows this order (the sort
        // below is stable): shortcuts, dictionary, spellings, vocabulary.
        let mut terms = self.personal.rules();
        terms.extend(
            self.spellings
                .iter()
                .chain(self.vocabulary.iter().map(|s| (s, s)))
                .filter(|(k, _)| !k.is_empty())
                .map(|(a, b)| Rule {
                    trigger: a.clone(),
                    value: b.clone(),
                    kind: Kind::Profile,
                }),
        );
        terms.sort_by_key(|rule| std::cmp::Reverse(rule.trigger.len()));
        let mut protected: Vec<(String, &Rule)> = vec![];
        let mut text = String::new();
        let mut i = 0;
        // Unique marker avoids changing marker-like text in the transcript or
        // in any replacement value.
        let mut marker = "GFPROTECTED".to_string();
        while raw.contains(&marker) || terms.iter().any(|t| t.value.contains(&marker)) {
            marker.push('X');
        }
        while i < raw.len() {
            let found = terms
                .iter()
                .find(|rule| matches_term(raw, i, &rule.trigger));
            if let Some(rule) = found {
                let token = format!("{marker}{}Z", protected.len());
                text.push_str(&token);
                protected.push((token, rule));
                i += rule.trigger.len();
            } else {
                let Some(c) = raw[i..].chars().next() else {
                    break;
                };
                text.push(c);
                i += c.len_utf8();
            }
        }
        let mut out = crate::transcribe::post_process(&text, english, fillers, lists);
        // Commands run before protected values are restored, so command words
        // inside corrections or shortcut text are never executed.
        if self.spoken_commands {
            out = commands(&out);
        }
        // An utterance that is only a spoken shortcut inserts exactly its text,
        // without sentence punctuation.
        if let [(token, rule)] = protected.as_slice() {
            let bare = out.trim().trim_end_matches(['.', '!', '?', ',']).trim_end();
            if rule.kind == Kind::Shortcut && bare == token {
                return rule.value.clone();
            }
        }
        for (token, rule) in &protected {
            out = out.replace(token, &rule.value);
        }
        out
    }
}
fn word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '/' | '.' | '@')
}
fn matches_term(text: &str, at: usize, term: &str) -> bool {
    let Some(part) = text.get(at..at + term.len()) else {
        return false;
    };
    part.eq_ignore_ascii_case(term)
        && (at == 0 || !text[..at].chars().next_back().is_some_and(word))
        && !text[at + term.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/' | '@'))
}
fn commands(text: &str) -> String {
    let mut out = String::new();
    let mut i = 0;
    let mut quote = None;
    let mappings = [
        ("new paragraph", "\n\n"),
        ("bullet point", "\n• "),
        ("comma", ","),
    ];
    while i < text.len() {
        let Some(c) = text[i..].chars().next() else {
            break;
        };
        if matches!(c, '"' | '“' | '”') {
            quote = if quote.is_some() { None } else { Some(c) };
        }
        if quote.is_none() {
            let escaped = matches_term(text, i, "literal");
            let at = if escaped {
                let j = i + 7;
                j + text[j..].len() - text[j..].trim_start().len()
            } else {
                i
            };
            if let Some((term, replacement)) = mappings
                .iter()
                .find(|(term, _)| matches_term(text, at, term))
            {
                if escaped {
                    out.push_str(&text[at..at + term.len()]);
                } else {
                    while out.ends_with(' ') {
                        out.pop();
                    }
                    out.push_str(replacement);
                }
                i = at + term.len();
                if !escaped {
                    while text[i..].starts_with([' ', '.', ',']) {
                        i += 1;
                    }
                    if *replacement == "," && i < text.len() {
                        out.push(' ');
                    }
                }
                continue;
            }
        }
        out.push(c);
        i += c.len_utf8();
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_is_exact_including_unicode_paths_and_numbers() {
        let p = Profile {
            mode: Mode::Literal,
            ..Default::default()
        };
        for s in [
            "https://example.com/a",
            "foo_bar = 1.25",
            "  🙂 世界  ",
            "um new paragraph",
            "v1.2",
        ] {
            assert_eq!(p.format(s, true, true, true), s);
        }
    }
    #[test]
    fn protects_glossary_and_explicit_spelling_without_substrings_or_cascades() {
        let p = Profile {
            vocabulary: vec!["ER".into(), "Gretchen Flow".into()],
            spellings: BTreeMap::from([("gretchen flo".into(), "Gretchen Flow".into())]),
            ..Default::default()
        };
        assert_eq!(
            p.format("Set ER to 5 and use gretchen flo", true, true, false),
            "Set ER to 5 and use Gretchen Flow."
        );
        assert_eq!(p.format("server_er 25", true, true, false), "server_er 25.");
    }
    #[test]
    fn commands_are_opt_in_and_escaped_or_quoted_commands_are_literal() {
        let p = Profile {
            spoken_commands: true,
            ..Default::default()
        };
        assert_eq!(
            p.format("Hello comma world new paragraph next", true, false, false),
            "Hello, world\n\nnext."
        );
        assert_eq!(
            p.format(
                "Say literal comma and \"new paragraph\"",
                true,
                false,
                false
            ),
            "Say comma and \"new paragraph\""
        );
        assert_eq!(
            Profile::default().format("new paragraph", true, false, false),
            "new paragraph."
        );
    }
}
