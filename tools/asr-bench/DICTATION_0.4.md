# Natural technical dictation gate

The November 4 comparison is complete on Matt's 14 supplied short recordings.
The agreed warm-call budget is **median ≤100 ms, p95 ≤150 ms**. Parakeet v2 remains
the default after the quality/latency comparison; see
[the report](../../docs/MODEL_COMPARISON_0.4.md#matts-natural-technical-dictation).
Only aggregate results and redacted sample excerpts are public. Original audio,
references, annotated manifests and raw per-clip transcripts remain outside Git.
The topics below are guidance for future extensions; this run does not cover the
suggested long-form recording or other speakers.

The audiobook corpus cannot establish names, numeric formatting or punctuation quality.
Record Matt's normal dictation locally, without a recognizer-generated reference. Keep
original audio, annotations, PCM and result transcripts outside Git. Do not substitute
TTS for a natural-speech measurement. No microphone is started by these scripts.

For future extensions, use at least 12 utterances spanning short commands, 10–30 second technical prose,
hesitations and one continuous passage over 30 seconds. Include names and jargon
actually used in daily work, decimals, versions, dates, amounts, URLs, negation and
sentence/list punctuation. Suggested prompts are topics, not scored reference text:

- Describe a change involving Greta Flow, Parakeet, FluidAudio and Core ML.
- Explain a Rust or Swift bug and its expected behavior.
- Dictate a pull-request review mentioning Matthew Kope and GitHub.
- Give a version, a decimal and an integer in the same sentence.
- Describe a meeting time and date, then correct one of them naturally.
- Compare two memory measurements and percentages.
- Dictate a list of three next steps with normal sentence punctuation.
- Describe a failed deployment, explicitly stating what must not happen.
- Include unfamiliar proper names chosen before running any candidate.
- Dictate an email address, a URL and a local path.
- Explain a design for 30–60 seconds without deliberate long pauses.
- Repeat one topic with a real microphone in a noisy room (separate condition).

Save mono 16 kHz WAVs. Listen and manually transcribe what was actually said;
choose intended numeric formatting and punctuation before inspecting model output.
For example, a local `annotations.json` contains:

```json
[
  {
    "id": "matt-001",
    "path": "matt-001.wav",
    "text": "Matt tested Core ML 2.5, using 24 GB of memory.",
    "names": ["Matt", "Core ML"],
    "numbers": ["2.5", "24"],
    "score_punctuation": true
  }
]
```

```sh
"$PYTHON" tools/asr-bench/prepare_dictation.py /private/path/annotations.json /private/path/run-01
# Use run-01/manifest.json with each ASRBench invocation documented in MODEL_COMPARISON_0.4.md.
"$PYTHON" tools/asr-bench/model_summary.py /private/path/run-01/manifest.json \
  /private/path/results/v2-old.jsonl /private/path/results/v2-new.jsonl \
  /private/path/results/v3.jsonl /private/path/results/unified.jsonl \
  /private/path/results/lite.jsonl /private/path/results/eou.jsonl \
  /private/path/results/v2-boost.jsonl > /private/path/aggregate-summary.json
```

The preparer makes the same clean/0.02-gain/20dB synthetic noise variants as the
public corpus. These are not three independent dictations or real noisy-room
recordings. Names/numbers are exact protected-span counts (case and formatting
included); inspect unrelated entity hallucinations manually as well. Punctuation
has two metrics: the original symbol-sequence edit distance, and a sentence-mark
score at aligned normalized-word boundaries. The latter excludes decimal, clock,
name-internal and phone separators; its counts and limitations are documented in
the report. Review local transcripts for meaning as well as these numeric scores.

For references supplied as JSONL `audio`/`text` rows, first create the annotated
JSON array above **locally**, preserving the reference text and adding checked
`names` and `numbers` lists before running any model. This private intermediate
was used in the November 4 comparison; it must not be copied into this repository.

Never publish the source audio, references, annotated manifests, audio hashes or
raw per-clip result files. Export only aggregate fields. Redacted sample excerpts
must remove personal names and phone numbers, including recognition variants;
omitting the whole sensitive context is safer than replacing only exact strings.
Check the staged diff before pushing. The November 4 samples omit the entire
name-bearing prefix of clip 03 and phone-bearing prefix of clip 05.

Use the agreed warm median/p95 latency budget before considering promotion. Then require
lower English WER, no names/numbers/punctuation regression, pinned artifact licensing
and hashes, and the safe two-engine switching/fallback tests from the plan. Until
these checks pass, retain v2 and keep streaming disabled.
