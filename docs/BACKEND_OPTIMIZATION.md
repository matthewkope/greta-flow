# Greta Flow backend optimization audit

Reviewed 2026-09-18, local checkout at `949f398161234695c58f9697bfdf33421eefd411`, including existing uncommitted backend changes. Product scope: `desktop/src-tauri`, not the Python prototype. Existing changes to `main.rs` and `desktop/ui/hotkey.js` were preserved. This deliverable is an audit and implementation backlog, not a deployed optimization.

Tracking: [Personal Projects in Linear](https://linear.app/mattkope/project/personal-projects-5738a1655b42).

Implementation issues: MAT-25–MAT-34, all labeled **Gretchen Flow**. Follow-up implementation and local model tests are documented in [the benchmark report](ASR_BENCHMARK_2026-09-18.md). MAT-25 is ready for review; grammar work and model evaluation are in progress. The audit below records the original findings, including bugs subsequently fixed locally; nothing has shipped yet.

+- [MAT-25: fix Unicode panics and guarantee transcription cleanup](https://linear.app/mattkope/issue/MAT-25/gretchen-flow-fix-unicode-panics-and-guarantee-transcription-cleanup)
- [MAT-26: remove blocking work from hotkey callbacks and serialize jobs](https://linear.app/mattkope/issue/MAT-26/gretchen-flow-remove-blocking-work-from-hotkey-callbacks-and-serialize)
- [MAT-27: measure speech-end latency and establish an offline benchmark corpus](https://linear.app/mattkope/issue/MAT-27/gretchen-flow-measure-speech-end-latency-and-establish-an-offline)
- [MAT-28: preserve meaning in filler removal, lists, and pause punctuation](https://linear.app/mattkope/issue/MAT-28/gretchen-flow-preserve-meaning-in-filler-removal-lists-and-pause)
- [MAT-29: benchmark and tune the existing Whisper inference path](https://linear.app/mattkope/issue/MAT-29/gretchen-flow-benchmark-and-tune-the-existing-whisper-inference-path)
- [MAT-30: transcribe during recording and flush only the tail on release](https://linear.app/mattkope/issue/MAT-30/gretchen-flow-transcribe-during-recording-and-flush-only-the-tail-on)
- [MAT-31: compare open models on M4 Pro before choosing an ASR replacement](https://linear.app/mattkope/issue/MAT-31/gretchen-flow-compare-open-models-on-m4-pro-before-choosing-an-asr)
- [MAT-32: add vocabulary profiles, literal dictation, and spoken formatting](https://linear.app/mattkope/issue/MAT-32/gretchen-flow-add-vocabulary-profiles-literal-dictation-and-spoken)
- [MAT-33: prevent late or misplaced insertion and add recoverable retry](https://linear.app/mattkope/issue/MAT-33/gretchen-flow-prevent-late-or-misplaced-insertion-and-add-recoverable)
- [MAT-34: serialize history writes and add private retention controls](https://linear.app/mattkope/issue/MAT-34/gretchen-flow-serialize-history-writes-and-add-private-retention)

## Recommendation

First fix text-processing crashes and move blocking work out of hotkey callbacks. Then measure release-to-insertion latency, tune the existing Whisper engine, and add transcription during recording. Benchmark Parakeet and Moonshine before selecting a replacement. A second grammar LLM should be optional because it adds work after speech recognition.

Observed setup: Apple M4 Pro, 24 GiB RAM, `large-v3-turbo`, English pinned, pause punctuation at 700 ms, filler removal and list formatting enabled. Both the 1.5 GiB Turbo model and 547 MiB Q5 model are already cached. Model file size alone does not establish inference speed or peak memory.

## Why completion can feel slow

Current path: hotkey release → synchronous recorder stop → resample whole recording → RMS gate/gain → acquire engine mutex → allocate Whisper state → full-clip beam-search inference → grammar cleanup → inject → history/refresh.

The 700 ms pause setting is a punctuation threshold, **not an intentional 700 ms sleep**. Inference starts after release, so all recognition work contributes to the perceived wait. Quiet audio is gated only by whole-clip RMS; there is no speech segmentation or silence trimming.

| Finding | Evidence | Consequence and proposed change |
| --- | --- | --- |
| Hotkey handling waits for inference | `main.rs:134` locks the engine; `main.rs:191` holds that lock through transcription; the event tap calls `on_shortcut` directly | A new press can block behind an earlier clip. Use a dedicated inference worker, nonblocking readiness state, bounded queue, and session IDs. This is a code-confirmed blocking path, not a timed reproduction of the user's incident. |
| Recorder stop can wait indefinitely | `audio.rs`, `Recorder::stop`, uses `recv()`; caller is before worker spawn | Send stop requests outside event callbacks, acknowledge start/failure, and bound waits. Test stalled device setup, stream shutdown, and disconnected devices. |
| Entire clip decoded after release | `main.rs:146–213`; `transcribe.rs:53–70` | Stream audio to a worker while recording. Cache stable partial text internally; finalize the tail on release and inject only once. |
| Expensive decoder defaults | `transcribe.rs:54–65`: fresh state, beam size 5, token timestamps enabled for pause punctuation | Compare greedy best-of-1 against beam-5; reuse/warm state where supported; compare model-native punctuation without token timestamps. Maintain accuracy gates. |
| Adaptive retries may inflate tail latency | Locked `whisper-rs-sys 0.13.1` defaults to temperature increments of 0.2 and quality-triggered fallback | Instrument retry/fallback counts. Compare a bounded retry policy; do not silently trade intelligibility for speed. This is a hypothesis for intermittent stalls. |
| Reload recreates weights for grammar changes | `main.rs:1268–1310` always loads a new engine | Separate model lifetime from per-request language/vocabulary/format options. Serialize swaps with generation IDs so older requests cannot overwrite newer selections. |
| No overall processing ownership | `recording` only describes capture; completion always sets Idle | Define queued/running/cancelled/completed sessions. Preserve result ordering, reject stale completion, and avoid an old job overwriting a newer recording's status. |

## Grammar correctness: reproduced failures

A temporary Rust harness used the actual cleanup functions and actual `lists.rs`, without microphone input or model inference. All 21 existing cleanup/list tests pass. Additional direct invocations reproduced:

| Input / function | Actual behavior | Required behavior |
| --- | --- | --- |
| `éclair`, `你好`, `🙂 hello` in `fix_standalone_i` | Panics | Preserve Unicode; only inspect the suffix after a successful ASCII `i` prefix match. |
| `Hello 世界你好` in `format_lists` | Panics | Use checked UTF-8 slices for case-insensitive `number ` prefix handling. |
| `One, eat. Two, sleep. Three.` | Drops `Three.` | Do not consume a marker until a complete item has been accepted. |
| `Set ER to 5.` in filler removal | `Set to 5.` | Protect acronyms and dictionary entries. |
| `Use U-H here.` in filler removal | `Use here.` | Match lexical filler tokens; stripping all nonletters turns meaningful text into a filler. |
| `First class is full. Second class is empty.` | Rewrites as a numbered list | Prefer explicit list cues or a conservative mode for ambiguous ordinal prose. |

The first panic happens because `fix_standalone_i` slices byte 1 before checking whether the word starts with `i`. The second slices at byte 7 even if that position is inside a multibyte character. Cleanup runs while the engine mutex is held: a panic can poison that mutex and skip completion cleanup. Subsequent `unwrap()` calls can then panic. This could explain persistent failure, but it does not prove the cause of an observed slow session.

Further grammar improvements:

- Apply English filler/list rules only to confirmed English, including detected language when configuration is `auto`. Currently English pronoun correction requires explicit `en`, while English fillers/lists run for every language.
- Preserve model punctuation by default; treat pause-based sentence boundaries as an opt-in fallback. A thinking pause does not necessarily end a sentence. Token pieces are not reliable word boundaries.
- Protect names, URLs, paths, decimals, versions, acronyms, quoted text, and code identifiers. Avoid unconditional sentence punctuation in literal dictation profiles.
- Add a golden corpus for false edits, not only successful transformations. Measure name/number preservation separately from word error rate.
- Keep deterministic cleanup fast. An optional local rewrite model must preserve meaning and return the original transcript on timeout or invalid output.

## Open-source / open-weight model research

**Updated selection requirement:** a replacement must beat the current unquantized Turbo in both recognition accuracy and release-to-completed-insertion latency on the M4 Pro, with no regression on names, numbers, or punctuation. Require identical recordings, repeated paired trials, and uncertainty estimates. Separate raw-model gains from pipeline improvements. Smaller size or faster first-token output alone does not qualify. The tuning regression allowance below does not apply to replacement selection.

For English, prioritize **Parakeet TDT 0.6B v2** alongside v3: NVIDIA reports 6.05% average English WER for v2, and FluidAudio supports its Core ML conversion. These published results do not prove improvement over this app. [V2 model card](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2).

Specify **Moonshine v2 Medium streaming** for the second candidate. Its paper reports 6.65% average WER versus 7.8% for Whisper Turbo. The reported M3 first-token measurements are not final-insertion timings or a benchmark of this application's Metal implementation. [Paper, Tables 2–3](https://download.moonshine.ai/docs/moonshine_streaming_paper.pdf). Tiny/Small models and Whisper Q5 are not assumed accuracy upgrades.

These are candidates, not measured winners. Official documentation was checked on the review date. Licensing must be pinned to the exact model artifact separately from the runtime.

| Candidate | Fit for this app | Integration and tradeoff |
| --- | --- | --- |
| Whisper Turbo F16 vs Q5; smaller English Whisper baseline | Lowest-risk experiment with the current runtime | Existing Metal path and downloaded Turbo variants. Quantization reduces model storage and can improve efficiency depending on hardware; benchmark quality and latency. Current whisper.cpp also supports VAD and Core ML, but availability must be checked against the older pinned Rust binding. [Upstream](https://github.com/ggml-org/whisper.cpp). |
| Parakeet TDT 0.6B v3 | High-priority Mac experiment | 25 European languages, native punctuation and timestamps; model license CC-BY-4.0. [NVIDIA model card](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3). FluidAudio offers a Swift/Core ML implementation and sliding-window processing; integrate through a persistent native bridge/worker, retaining Whisper as fallback. Its batch throughput claims do not establish hotkey-release latency. [Runtime](https://github.com/FluidInference/FluidAudio). |
| Moonshine streaming | High-priority experiment for doing work during speech | On-device streaming, macOS support, and a C API offer a plausible Rust integration path. Current runtime and default model licensing are MIT, with exceptions for legacy non-English non-streaming models. Pin a streaming artifact and validate punctuation, technical vocabulary, and noisy speech. [Official project](https://github.com/moonshine-ai/moonshine). |
| Qwen3-ASR 0.6B | Secondary multilingual experiment | Apache-2.0; supports 30 languages. Official streaming currently requires vLLM and does not return timestamps, making it less direct for this local Rust/macOS product. Server throughput or first-token claims are not comparable to complete text insertion. [Model card](https://huggingface.co/Qwen/Qwen3-ASR-0.6B). |
| Qwen3 0.6B text model | Optional grammar rewrite experiment only | Apache-2.0, supports disabling thinking. Keep loaded, cap generation and time, and test meaning preservation. This is a general text model, not a demonstrated grammar-correction winner. [Model card](https://huggingface.co/Qwen/Qwen3-0.6B). |

Do not migrate the app to the Python prototype merely to try another runtime. Prototype experiments may use Python, but shipping needs a persistent local engine with predictable packaging and cancellation.

## Measurement and acceptance plan

Record monotonic timestamps for last speech sample, release event, capture-stop acknowledgment, queue entry/start, resample/VAD, state setup, inference, cleanup, injection start/end, and persistence. Log model/config identifiers, audio duration, fallback count, and outcome; exclude transcript text and raw audio from diagnostic logs.

Report p50/p95/p99 separately for cold start and warm use, 2/5/15/30/60-second recordings, built-in/Bluetooth microphones, quiet/noisy input, rapid repeat presses, and CPU/GPU contention. Include final-text latency and completed-insertion latency. Insertion may be slower than inference for large outputs, so timing only `state.full` would be misleading.

Proposed target, **not achieved or measured**: warm 2–15 second dictations on the M4 Pro reach final text at p50 ≤300 ms and p95 ≤700 ms after release; report last-speech-to-insertion separately. Keep callback execution under 10 ms in stress tests. Use at least 100 representative clips per main comparison and repeated warm runs; treat p99 as exploratory unless the sample size is sufficient.

Compare F16/Q5 × greedy/beam-5 × timestamp on/off using identical recordings. Report word error rate, punctuation quality, exact name/number accuracy, false edits, memory, and energy impact. Start with a ≤0.5 percentage-point WER regression budget, no regressions on critical name/number fixtures, and zero hangs/duplicate insertions in cancellation tests. These are proposed engineering gates, not empirical guarantees.

Streaming design: bounded audio buffers → incremental resampling/VAD → chunk inference with overlap or model-native stream state → stable-prefix tracking → tail flush on release → deterministic cleanup → one insertion. Preserve timestamps when trimming gaps. Do not re-decode the entire growing clip on every chunk. Avoid adding endpoint silence waits after an explicit release. Keep a whole-clip fallback for quality regressions, with a measured latency budget.

## Additional backend features users would notice

1. **Personal vocabulary profiles:** build on the existing glossary with application/domain-specific terms and protected spellings; update without reloading weights.
2. **Reliable cancel and retry:** session cancellation, no late insertion after cancellation, and retry from a bounded in-memory audio buffer. No audio persistence by default.
3. **Literal and prose modes:** backend config selects punctuation, filler behavior, and formatting for code, chat, or long-form writing.
4. **Spoken formatting commands:** opt-in `new paragraph`, `comma`, and `bullet point`, with a literal escape so quoted commands remain words.
5. **Safer output delivery:** verify the target application before insertion and preserve failed output for recovery. Newlines currently become Return keystrokes; test apps where Return submits a message. Compare native text insertion with an optional clipboard strategy that restores clipboard state.
6. **Private history controls:** configurable retention/history-off, serialized append/trim/clear, and removal of plaintext transcript logging. Existing history is capped at 200 entries but multiple workers can race and diagnostics currently log full text.

## Delivery order and limits

1. Unicode/data-loss fixes, callback isolation, session cleanup, and timing.
2. Warm-state/decoder/config improvements and speech segmentation.
3. Model comparison, then internal streaming with final-only insertion.
4. Vocabulary profiles, literal mode, command parsing, reliable retry, and history controls.

This audit did not reproduce a live microphone delay, run end-to-end model benchmarks, change the active model/config, or rebuild/install the app. The 21 passing tests cover extracted text-processing code only, not the full application. No frontend source was edited.
