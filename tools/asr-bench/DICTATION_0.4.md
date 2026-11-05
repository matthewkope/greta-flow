# Natural technical dictation gate

The audiobook corpus cannot establish names, numeric formatting or punctuation quality.
Record Matt's normal dictation locally, without a recognizer-generated reference. Keep
original audio, annotations, PCM and result transcripts outside Git. Do not substitute
TTS for a natural-speech measurement. No microphone is started by these scripts.

Use at least 12 utterances spanning short commands, 10–30 second technical prose,
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
"$PYTHON" tools/asr-bench/model_summary.py /private/path/run-01/manifest.json /private/path/results/*.jsonl
```

The preparer makes the same clean/0.02-gain/20dB synthetic noise variants as the
public corpus. These are not three independent dictations or real noisy-room
recordings. Names/numbers are exact protected-span counts (case and formatting
included); inspect unrelated entity hallucinations manually as well. Punctuation
is an edit distance over punctuation marks in order, not their word positions.
Review transcripts side by side for placement and meaning before accepting them.
Report aggregate scores only with Matt's permission to publish his content.

Agree the warm median/p95 latency budget before considering promotion. Then require
lower English WER, no names/numbers/punctuation regression, pinned artifact licensing
and hashes, and the safe two-engine switching/fallback tests from the plan. Until
these checks pass, retain v2 and keep streaming disabled.
