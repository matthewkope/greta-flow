//! Notes derived from a meeting transcript without inventing content.
//!
//! No local summarization model is approved yet, so `SummaryStatus` reports
//! that plainly. Action items are *candidates* found by commitment phrases;
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
    /// "unavailable" until a local summarizer is approved and installed.
    pub status: String,
    pub reason: String,
    /// How `action_items` were produced.
    pub action_items_method: String,
}
impl SummaryStatus {
    pub fn unavailable() -> Self {
        Self {
            status: "unavailable".into(),
            reason: "No on-device summary model is installed. The transcript is saved; \
                     summaries will appear once a local model is approved."
                .into(),
            action_items_method: "keyword".into(),
        }
    }
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
    if record.summary.status == "unavailable" {
        out.push_str(&format!("_{}_\n", record.summary.reason));
    }
    out.push_str("\n## Action item candidates (keyword-detected, not AI)\n\n");
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
            }],
            source: "microphone".into(),
            tz_offset_min: 0,
            summary: SummaryStatus::unavailable(),
            complete: true,
            limit_reached: false,
        };
        let md = markdown(&record);
        assert!(md.contains("Duration 01:02:10"));
        assert!(md.contains("No on-device summary model is installed"));
        assert!(md.contains("- [00:00:03] I will book the room.\n"));
        let gap = md.find("[00:00:10–00:00:12]").unwrap();
        assert!(md.find("[00:00:03] I will").unwrap() < gap);
        assert!(gap < md.find("[01:02:05] Bye.").unwrap());
        assert_eq!(clock(59_999), "00:00:59");
    }
}
