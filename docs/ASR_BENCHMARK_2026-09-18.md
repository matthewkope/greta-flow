# Greta Flow: first local model comparison

**Parakeet TDT v2 is the strongest candidate from this pilot.** It completed warm engine calls about 25× faster than the current Whisper configuration and had slightly lower normalized word error. Moonshine was faster than Whisper but less accurate here. Neither candidate is established as a production replacement yet.

## Measured results

Apple M4 Pro, 24 GiB. Same 24 real English audiobook clips, 147.145 seconds total, 1.64–18.29 seconds per clip, one speaker. Three warm passes per batch engine after a separate first-call warm-up; 72 warm calls per model. No microphone capture or cursor injection.

| Engine / runtime | Warm median | Warm p95 | Normalized word error |
| --- | ---: | ---: | ---: |
| Current Whisper large-v3-turbo, whisper-rs/Metal, beam 5 | 1,409 ms | 2,020 ms | 3.16% |
| Parakeet TDT 0.6B v2, FluidAudio/Core ML | **57 ms** | **65 ms** | **2.59%** |
| Moonshine v2 Medium Streaming model, native runtime via Python, batch API | 383 ms | 1,190 ms | 5.08% |

These are complete **engine call** timings on preloaded audio, not an in-app release-to-cursor benchmark. Whisper includes state creation, token timestamps and the app's text cleanup; its decoder/model settings match the audited product defaults. Parakeet uses FluidAudio defaults with a fresh per-clip decoder state and persistent weights. Moonshine includes Python-to-native sample conversion. These are different practical runtime paths, not controlled kernel-only measurements.

Moonshine was also tested with real audio paced in 100 ms chunks, with partial processing every 500 ms. Across one warm pass over the same clips, simulated end-of-capture to final transcript was **147 ms median / 326 ms p95**, with **4.31% normalized word error**. This includes any processing backlog beyond the last capture deadline, but excludes recorder shutdown and injection. It is a different latency metric from batch calls and should not be ranked directly in the table.

## Accuracy assessment

English normalization treats equivalent `Mr./Mister`, `10/ten`, and contractions consistently. Without that normalization, lexical WER is 4.64% Whisper, 2.61% Parakeet, 5.99% Moonshine batch, and 5.51% Moonshine streaming. The normalized results are the primary recognition metric; neither metric scores punctuation quality.

There are only **348 normalized reference words**, repeated three times for timing. Parakeet makes 9 errors per pass versus Whisper's 11: two fewer errors, not enough evidence for a broad accuracy claim. A paired bootstrap over the 24 unique clips gives a Parakeet-minus-Whisper WER difference of −0.57 percentage points, with a 95% interval of **−2.29 to +0.59 points**. That interval crosses zero. This estimate is conditional on one speaker and is not evidence of population-wide superiority.

Parakeet improves two clips, worsens one, and ties 21 on normalized word errors. For example, it changes `you were sleeping` to `you are sleeping` where Whisper retains `were`. Both struggle with unusual fictional names. Moonshine batch transcripts vary on three clips across repeated calls; all outputs are retained for inspection. Its aggregate WER above averages all three passes; paired bootstrap results use only pass 1 to avoid counting repeats as independent accuracy evidence.

## Startup and downloaded artifacts

The first Parakeet model load took **33.33 seconds**, including first-use Core ML setup. A fresh process after that cache existed loaded it in **183 ms**. Its first transcription calls were about 80–100 ms. Model initialization must remain outside the per-dictation path. These are observed process starts, not rigorously controlled cold-disk tests.

Both requested models are downloaded:

- `~/.cache/gretchen-flow/models/parakeet-v2-coreml/` — full pinned conversion snapshot, about 2.4 GiB including alternate exports. The default required exports were tested; this disk total is not runtime RAM usage.
- `~/.cache/gretchen-flow/models/moonshine/` — English Medium Streaming `quantized_26_08_21`, about 268 MiB including downloader metadata/optional spelling assets. The benchmark did not enable spelling mode.

Exact runtime/package versions, model hashes, dataset identifiers, transcripts and timings are in [the result directory](../tools/asr-bench/results/2026-09-18/). [Reproduction instructions](../tools/asr-bench/README.md) include the release-build benchmark programs and scoring scripts. Model inference stayed local; no personal recordings or dictation history were uploaded or used.

## Backend work completed locally

- Fixed unsafe UTF-8 slicing in pronoun correction and list-prefix parsing.
- Contained Rust inference/cleanup panics before they poison the engine mutex, allowing normal error completion and a subsequent dictation. Native process aborts cannot be caught by this mechanism.
- Preserved unfinished list markers instead of discarding them.
- Preserved acronyms, hyphenated/numbered identifiers and quoted fillers during filler removal.
- Applied English filler/list cleanup only to explicit or detected English.
- Added regression tests and reproducible offline model/scoring tools.

Validation: **31 application Rust tests**, **27 shared tests compiled through the benchmark example**, **5 scoring tests**, `cargo fmt --check`, and all-target Clippy with warnings denied passed. Swift release benchmark built and ran successfully. The shared example tests duplicate the relevant app tests; they are not 27 additional unique regression cases.

Linear: all ten issues have the **Gretchen Flow** label in [Personal Projects](https://linear.app/mattkope/project/personal-projects-5738a1655b42). MAT-25 is In Review. MAT-28 remains In Progress for broader grammar protection; MAT-31 remains In Progress because the model-replacement acceptance gate is not met. The pilot also advances MAT-27, but app-stage instrumentation remains outstanding.

## Next decision

Proceed with a backend-only Parakeet adapter experiment behind configuration, retaining Whisper fallback. Before changing the default, expand to multiple speakers, microphone noise, natural dictation, technical vocabulary, names/numbers, and punctuation fixtures, then measure actual release-to-completed-insertion. Moonshine remains experimental because it failed the accuracy-improvement requirement in this pilot. No app installation, active-model change, commit, or push occurred in this work.
