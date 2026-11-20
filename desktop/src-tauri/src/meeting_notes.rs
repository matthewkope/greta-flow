//! Notes derived from a meeting transcript without inventing content.
//!
//! Summaries come only from an explicit local model run (`summary.rs`) and
//! pass `validate_model_grounding`; otherwise `SummaryStatus` says plainly that
//! none exists. Keyword action items are *candidates* found by commitment phrases;
//! each quotes one transcript sentence verbatim with its segment timestamp.
//! `validate_grounding` is the gate any future model output must pass: an
//! item whose quote is not literally present in its cited segment is dropped.
use crate::meeting::{MeetingRecord, Segment};
use serde::{Deserialize, Serialize};

const MAX_ITEMS: usize = 100;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionItem {
    /// Index into the meeting's `segments`.
    pub segment_index: usize,
    pub start_ms: u64,
    /// Exact sentence from the transcript.
    pub quote: String,
    /// The phrase that marked it ("keyword" items) — not an interpretation.
    pub cue: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SummaryStatus {
    /// "unavailable" | "generating" | "generated" | "failed".
    pub status: String,
    pub reason: String,
    /// How `MeetingRecord::action_items` were produced: always "keyword".
    /// Model action items live in `summary` and are labelled separately.
    pub action_items_method: String,
    /// Present only after a successful local model run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<MeetingSummary>,
}
impl SummaryStatus {
    pub fn unavailable() -> Self {
        Self {
            status: "unavailable".into(),
            reason: "Not summarized. The transcript is saved; a summary can be generated \
                     on this Mac with a local model from the meeting view."
                .into(),
            action_items_method: "keyword".into(),
            summary: None,
        }
    }
    pub fn generating() -> Self {
        Self {
            status: "generating".into(),
            reason: "Generating a local summary…".into(),
            ..Self::unavailable()
        }
    }
    pub fn failed(reason: String) -> Self {
        Self {
            status: "failed".into(),
            reason,
            ..Self::unavailable()
        }
    }
    pub fn generated(summary: MeetingSummary) -> Self {
        Self {
            status: "generated".into(),
            reason: format!(
                "Local model summary; every item quotes the transcript ({} ungrounded item(s) dropped).",
                summary.dropped_ungrounded
            ),
            action_items_method: "keyword".into(),
            summary: Some(summary),
        }
    }
}

/// A model-proposed item before grounding checks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelItem {
    pub text: String,
    #[serde(default)]
    pub owner: Option<String>,
    pub segment_index: usize,
    pub quote: String,
}

/// A grounded summary bullet or action item. Bullets never carry an owner.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SummaryItem {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    pub segment_index: usize,
    pub start_ms: u64,
    pub quote: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeetingSummary {
    pub bullets: Vec<SummaryItem>,
    pub action_items: Vec<SummaryItem>,
    /// `repo@revision` for the pinned model, otherwise the folder name.
    pub model: String,
    pub generated_ms: u64,
    pub dropped_ungrounded: usize,
    /// Transcript sections sent to the model; 0 in records saved before this was tracked.
    #[serde(default)]
    pub sections: usize,
    /// Sections the model failed on; their content is not summarized.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_chunks: Vec<FailedChunk>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FailedChunk {
    pub index: usize,
    pub reason: String,
}

/// Grounding gate for model output: keep an item only if its quote is
/// non-empty and appears verbatim in the text of the cited segment. An owner
/// survives only if it appears in that quote. Returns kept items and the
/// number dropped.
pub fn validate_model_grounding(
    segments: &[Segment],
    items: Vec<ModelItem>,
) -> (Vec<SummaryItem>, usize) {
    let total = items.len();
    let kept: Vec<SummaryItem> = items
        .into_iter()
        .filter_map(|item| {
            let segment = segments.get(item.segment_index)?;
            if item.quote.trim().is_empty() || !segment.text.contains(&item.quote) {
                return None;
            }
            let owner = item
                .owner
                .map(|o| o.trim().to_string())
                .filter(|o| !o.is_empty() && item.quote.contains(o.as_str()));
            Some(SummaryItem {
                text: item.text.trim().to_string(),
                owner,
                segment_index: item.segment_index,
                start_ms: segment.start_ms,
                quote: item.quote,
            })
        })
        .collect();
    let dropped = total - kept.len();
    (kept, dropped)
}

/// Phrases that usually introduce a commitment or request. Matched on word
/// boundaries, case-insensitively.
const CUES: &[&str] = &[
    "action item",
    "to do",
    "todo",
    "follow up",
    "follow-up",
    "i will",
    "i'll",
    "we will",
    "we'll",
    "you will",
    "you'll",
    "need to",
    "needs to",
    "have to",
    "has to",
    "let's",
    "let us",
    "can you",
    "could you",
    "please",
    "assigned to",
    "deadline",
    "by tomorrow",
    "by monday",
    "by tuesday",
    "by wednesday",
    "by thursday",
    "by friday",
    "next week",
    "end of day",
];

fn sentences(text: &str) -> impl Iterator<Item = &str> {
    text.split_inclusive(['.', '?', '!'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn find_cue(sentence: &str) -> Option<&'static str> {
    let lower = sentence.to_lowercase();
    CUES.iter().copied().find(|cue| {
        lower.match_indices(cue).any(|(i, _)| {
            let before = lower[..i].chars().next_back();
            let after = lower[i + cue.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
    })
}

/// Candidate action items, each a verbatim transcript sentence.
pub fn candidate_action_items(segments: &[Segment]) -> Vec<ActionItem> {
    let mut items = vec![];
    for (index, segment) in segments.iter().enumerate() {
        for sentence in sentences(&segment.text) {
            if items.len() >= MAX_ITEMS {
                return items;
            }
            if let Some(cue) = find_cue(sentence) {
                items.push(ActionItem {
                    segment_index: index,
                    start_ms: segment.start_ms,
                    quote: sentence.to_string(),
                    cue: cue.to_string(),
                });
            }
        }
    }
    items
}

/// Keep only items whose quote appears verbatim in the cited segment and
/// whose timestamp matches it.
pub fn validate_grounding(items: Vec<ActionItem>, segments: &[Segment]) -> Vec<ActionItem> {
    items
        .into_iter()
        .filter(|item| {
            let quote = item.quote.trim();
            !quote.is_empty()
                && segments
                    .get(item.segment_index)
                    .is_some_and(|s| s.start_ms == item.start_ms && s.text.contains(quote))
        })
        .collect()
}

pub fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// Markdown notes for copying or export. Contains only transcript content and
/// capture facts.
pub fn markdown(record: &MeetingRecord) -> String {
    let mut out = format!(
        "# {}\n\nDuration {} · {} words · source: {}\n",
        record.title,
        clock(record.audio_ms),
        record.words,
        record.source
    );
    if !record.complete {
        out.push_str("\n> Interrupted: this meeting was not stopped normally.\n");
    }
    if record.limit_reached {
        out.push_str("\n> Recording reached the maximum meeting length.\n");
    }
    out.push_str("\n## Summary\n\n");
    match &record.summary.summary {
        Some(summary) if record.summary.status == "generated" => {
            out.push_str(&format!(
                "_Model summary ({}). Each line is the model's wording followed by the verbatim transcript quote it cites._\n\n",
                summary.model
            ));
            if !summary.failed_chunks.is_empty() {
                out.push_str(&format!(
                    "> Partial summary — {} of {} sections failed:\n",
                    summary.failed_chunks.len(),
                    summary.sections
                ));
                for f in &summary.failed_chunks {
                    out.push_str(&format!("> - section {}: {}\n", f.index + 1, f.reason));
                }
                out.push('\n');
            }
            for b in &summary.bullets {
                out.push_str(&format!(
                    "- [{}] Model wording: {} — Transcript: \"{}\"\n",
                    clock(b.start_ms),
                    b.text,
                    b.quote
                ));
            }
            if !summary.action_items.is_empty() {
                out.push_str("\n### Action items (model, grounded)\n\n");
            }
            for a in &summary.action_items {
                let owner = a
                    .owner
                    .as_deref()
                    .map(|o| format!(" ({o})"))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "- [{}] Model wording: {}{} — Transcript: \"{}\"\n",
                    clock(a.start_ms),
                    a.text,
                    owner,
                    a.quote
                ));
            }
        }
        _ => out.push_str(&format!("_{}_\n", record.summary.reason)),
    }
    out.push_str("\n## Action item candidates (keyword-detected)\n\n");
    if record.action_items.is_empty() {
        out.push_str("_None detected._\n");
    }
    for item in &record.action_items {
        out.push_str(&format!("- [{}] {}\n", clock(item.start_ms), item.quote));
    }
    out.push_str("\n## Transcript\n\n");
    let mut gaps = record.gaps.iter().peekable();
    for segment in &record.segments {
        while let Some(gap) = gaps.next_if(|g| g.start_ms < segment.start_ms) {
            out.push_str(&format!(
                "[{}–{}] _(no transcript: {})_\n",
                clock(gap.start_ms),
                clock(gap.end_ms),
                gap.reason
            ));
        }
        out.push_str(&format!("[{}] {}\n", clock(segment.start_ms), segment.text));
    }
    for gap in gaps {
        out.push_str(&format!(
            "[{}–{}] _(no transcript: {})_\n",
            clock(gap.start_ms),
            clock(gap.end_ms),
            gap.reason
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start_ms: u64, text: &str) -> Segment {
        Segment {
            start_ms,
            end_ms: start_ms + 1000,
            text: text.into(),
            source: Default::default(),
        }
    }

    #[test]
    fn candidates_quote_the_transcript_verbatim() {
        let segments = vec![
            seg(0, "Thanks everyone for joining. The numbers look good."),
            seg(
                61_000,
                "I'll send the deck by Friday. Can you review the budget? The displeased client left.",
            ),
            seg(125_000, "We discussed the roadmap"),
        ];
        let items = candidate_action_items(&segments);
        let quotes: Vec<_> = items.iter().map(|i| i.quote.as_str()).collect();
        assert_eq!(
            quotes,
            vec![
                "I'll send the deck by Friday.",
                "Can you review the budget?"
            ]
        );
        assert_eq!(items[0].segment_index, 1);
        assert_eq!(items[0].start_ms, 61_000);
        assert_eq!(items[0].cue, "i'll");
        // "displeased" contains "please" but is not a cue on word boundaries.
        assert_eq!(validate_grounding(items.clone(), &segments), items);
    }

    #[test]
    fn empty_or_cue_free_transcripts_produce_no_items() {
        assert!(candidate_action_items(&[]).is_empty());
        assert!(candidate_action_items(&[seg(0, "Hello there. Pleased to meet you.")]).is_empty());
    }

    #[test]
    fn grounding_rejects_fabricated_or_misattributed_items() {
        let segments = vec![seg(0, "Let's ship on Tuesday."), seg(5000, "Okay.")];
        let item = |segment_index, start_ms, quote: &str| ActionItem {
            segment_index,
            start_ms,
            quote: quote.into(),
            cue: "model".into(),
        };
        let kept = validate_grounding(
            vec![
                item(0, 0, "Let's ship on Tuesday."),
                item(0, 0, "Alice will write the launch blog post."),
                item(1, 5000, "Let's ship on Tuesday."),
                item(0, 5000, "Let's ship on Tuesday."),
                item(7, 0, "Okay."),
                item(1, 5000, "  "),
            ],
            &segments,
        );
        assert_eq!(kept, vec![item(0, 0, "Let's ship on Tuesday.")]);
    }

    #[test]
    fn markdown_has_timestamps_gaps_and_honest_summary_status() {
        let segments = vec![seg(3_000, "I will book the room."), seg(3_725_000, "Bye.")];
        let record = MeetingRecord {
            id: "m1".into(),
            kind: "meeting".into(),
            title: "Meeting 2026-11-19 10:00".into(),
            started_ms: 1,
            ended_ms: 2,
            audio_ms: 3_730_000,
            speech_ms: 2000,
            words: 6,
            text: "I will book the room.\nBye.".into(),
            action_items: candidate_action_items(&segments),
            segments,
            gaps: vec![crate::meeting::Gap {
                start_ms: 10_000,
                end_ms: 12_000,
                reason: "transcription failed".into(),
                source: Default::default(),
            }],
            source: "microphone".into(),
            tz_offset_min: 0,
            summary: SummaryStatus::unavailable(),
            complete: true,
            limit_reached: false,
        };
        let md = markdown(&record);
        assert!(md.contains("Duration 01:02:10"));
        assert!(md.contains("Not summarized."));
        assert!(md.contains("- [00:00:03] I will book the room.\n"));
        let gap = md.find("[00:00:10–00:00:12]").unwrap();
        assert!(md.find("[00:00:03] I will").unwrap() < gap);
        assert!(gap < md.find("[01:02:05] Bye.").unwrap());
        assert_eq!(clock(59_999), "00:00:59");
    }

    #[test]
    fn model_grounding_drops_fabricated_and_misattributed_quotes() {
        let segments = vec![
            seg(0, "Alice will send the deck by Friday."),
            seg(4000, "Bob said the budget is fine."),
        ];
        let item = |i, quote: &str, owner: Option<&str>| ModelItem {
            text: "t".into(),
            owner: owner.map(Into::into),
            segment_index: i,
            quote: quote.into(),
        };
        let (kept, dropped) = validate_model_grounding(
            &segments,
            vec![
                item(0, "Alice will send the deck", Some("Alice")),
                item(0, "Carol will write the blog post", Some("Carol")), // fabricated
                item(1, "Alice will send the deck", None),                // wrong segment
                item(9, "Alice will send the deck", None),                // no such segment
                item(1, "the budget is fine", Some("Carol")),             // owner not quoted
                item(0, "", None),
                item(0, "alice will send", None), // not verbatim (case)
            ],
        );
        assert_eq!(dropped, 5);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].owner.as_deref(), Some("Alice"));
        assert_eq!(kept[0].start_ms, 0);
        assert_eq!(kept[1].owner, None);
        assert_eq!(kept[1].start_ms, 4000);
    }

    #[test]
    fn old_summary_records_still_deserialize() {
        let old = r#"{"status":"unavailable","reason":"x","action_items_method":"keyword"}"#;
        let s: SummaryStatus = serde_json::from_str(old).unwrap();
        assert!(s.summary.is_none());
        assert!(!serde_json::to_string(&s).unwrap().contains("\"summary\""));
    }
}
