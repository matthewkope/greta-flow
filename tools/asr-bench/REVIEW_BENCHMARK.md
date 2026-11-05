# Five-card benchmark (2026-09-19)

Run on the user's M4 Pro, 24GiB Mac. Recognition stays local. No private dictation history or microphone recordings were used. The existing installed app was not replaced.

## Revision caveat

These retained September 19 results used a 0.0001 pre-gain RMS gate. PR #1's
review follow-up restores the original **0.0015** gate in both batch and streaming
preparation to reject near-silent microphone noise before amplification/inference.
The segmenter's separate 0.0001 boundary detector remains unchanged. The numbers
below have not been rerun against the restored decoder gate and must not be cited
as current-revision accuracy or quiet-input acceptance evidence. Reproduction on
the corrected revision can produce different transcripts and timing.

## Reproduction

Use the pinned Python environment/dependencies from `README.md`, plus the existing pinned Parakeet worker and model files. Here `PYTHON`, `CORPUS`, `SMOKE`, `MODEL`, and `HELPER` are task-specific paths of your choosing; `SMOKE` is the original 24-clip manifest.

```sh
"$PYTHON" tools/asr-bench/prepare_review_corpus.py "$CORPUS"
"$PYTHON" tools/asr-bench/prepare_paced_corpus.py "$CORPUS" "$SMOKE"
cargo build --locked --release --manifest-path desktop/src-tauri/Cargo.toml --example review_bench
GRETCHEN_PARAKEET_HELPER="$HELPER" desktop/src-tauri/target/release/examples/review_bench "$MODEL" "$CORPUS/manifest.json" > accuracy.jsonl
GRETCHEN_PARAKEET_HELPER="$HELPER" desktop/src-tauri/target/release/examples/review_bench "$MODEL" "$CORPUS/paced.json" paced > paced.jsonl
"$PYTHON" tools/asr-bench/review_summary.py "$CORPUS/manifest.json" accuracy.jsonl
"$PYTHON" tools/asr-bench/review_summary.py "$CORPUS/paced.json" paced.jsonl
```

Committed manifests replace absolute audio paths with `CORPUS/`; preparation regenerates actual paths and verifies source/audio checksums. Source: [OpenSLR LibriSpeech](https://huggingface.co/datasets/openslr/librispeech_asr), CC-BY-4.0, commit `71cacbfb7e2354c4226d01e70d77d5fca3d04ba1`, `clean/test/0000.parquet`, SHA-256 recorded in `source.json`. The 2-second paced sample comes from the previously pinned dummy validation subset. No audio binaries are committed.

## Accuracy and warm calls

120 evenly selected distinct speech clips, 40 speakers, 2,368 normalized reference words. Each has clean, 0.02-gain, and synthetic 20dB white-noise versions. Two warm passes follow a separate first call. Accuracy below uses only pass one; repetitions and variants do not add independent reference speech.

| Condition | Batch WER | Segmented WER | Batch median / p95 |
|---|---:|---:|---:|
| Clean | 2.196% | 2.196% | 61.3 / 97.8ms |
| Quiet | 2.618% | 2.449% | 60.7 / 98.9ms |
| Added noise | 2.745% | 2.745% | 60.6 / 98.2ms |

`segmented_offline` runs the production segmenter without real-time pacing. Its total processing time is not release latency. Clean/noisy aggregate word-error rates did not worsen; that does not prove every utterance, proper name, or punctuation mark stayed identical. No whole-clip fallback was needed in this short-clip accuracy corpus. Long continuous speech and queue-overflow fallbacks are tested separately.

See `results/2026-09-19/accuracy-summary.json`, raw `accuracy.jsonl`, and `runtime.log`. The process was freshly launched with cached CoreML artifacts; this is not a pristine first-install cold-cache measurement. Brief compilation/checking overlapped the early part of the accuracy run; timing is descriptive of this desktop session rather than a controlled idle-system result.

## Paced release tests

`paced.jsonl` sends 100ms input chunks against real wall-clock deadlines through the actual bounded worker. The 5/15/30/60-second files concatenate complete public utterances and inserted silence; they deliberately exercise the opportunity to finish recognition during pauses. The 2-second file retains a complete short utterance. They are synthetic timing scenarios made from real speech, not five natural dictations.

Only worker completion after the last input chunk is timed. Microphone shutdown, focus validation, native insertion, and persistence are excluded. A sub-millisecond tail flush on a recording with a trailing pause means recognition already completed before release; it does not mean the model transcribed the entire recording in that time. Production stage logs are needed to measure actual cursor latency.

## Remaining field validation

- Built-in and Bluetooth microphone capture/stop latency, quiet/noisy rooms, device disconnect, and permission denial.
- Native insertion in each supported app, multiline chat behavior, cursor/focus changes, cancellation, and uncertain setter failures.
- Natural technical dictation with independently checked names, numbers, punctuation, hesitations, and long continuous speech.
- Repeat latency under deliberate CPU/GPU contention, true first-install CoreML initialization, total app/helper RSS, and device energy use. `/usr/bin/time` process metrics are included but are not model RAM or energy measurements.

The code and offline evidence are ready for review. These remaining field checks must not be represented as completed acceptance tests or used to justify enabling streaming by default.

## Paced results and cold-process observations

Across the four constructed paused recordings (5/15/30/60 seconds, two passes), batch recognition median was **122.2ms**, versus **0.20ms** of remaining worker time after release. Both produced 13 normalized errors across 257 reference words in the first pass. The 2-second short clip had no word errors; its paced completion median was **88.2ms**, versus **68.6ms** batch, showing that streaming overhead can hurt short utterances without an exploitable pause. These tiny scenario counts are functional evidence, not population latency estimates.

Fresh-process model loading with existing CoreML caches took 333ms in the accuracy run and 160ms in the paced run. First recognition calls were 71.8ms and 70.2ms respectively. They are kept separate from warm calls. `runtime.log` includes `/usr/bin/time -l` counters; it reported 236,552,192 bytes maximum RSS for the benchmark process. This is not a combined app/helper peak or an energy measurement.
