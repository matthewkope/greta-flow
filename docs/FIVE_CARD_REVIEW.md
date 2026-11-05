# Five-card implementation review

Branch: `codex/gretchen-five-card-improvements`, based on `f6163668`.
Scope: MAT-33, MAT-32, MAT-27, MAT-30, MAT-34. No frontend changes or installed-app migration.

## MAT-33 — delivery and recovery

Each recording owns its captured Accessibility element, process ID, selected range, configuration, and session ID. Before delivery, the destination and selection must still match. Delivery writes `AXSelectedText` directly; it does not send Return or paste into a terminal. Unsupported/secure fields fail closed and retain the transcript for manual copy. A native setter failure is considered an uncertain delivery: automatic retry is suppressed to avoid duplicate text. An explicit copy is still available so the user can inspect and recover.

Optional config keys `cancel_shortcut`, `retry_shortcut`, and `recovery_shortcut` take ordinary accelerator strings; default is null to avoid taking existing system/app shortcuts. Example: `Ctrl+Alt+Escape`, `Ctrl+Alt+R`, `Ctrl+Alt+C`. Reload Config applies them. Recovery copies the last transcript; it deliberately replaces the clipboard only on that explicit action. Retry reruns recognition for an unattempted session using its original configuration and destination. It cannot redirect a failed result into a newly focused field. Cancel invalidates the session and clears recovery.

Audio recovery is capped at 120 seconds and expires 120 seconds after capture, swept at most five seconds later. It is never written to disk. A new recording replaces the old recovery slot. Successful insertion frees audio immediately. Recordings over 120 seconds may still transcribe, but cannot retry from audio. Capture itself is limited to ten minutes; exceeding it rejects the recording rather than inserting a truncated result.

The automatic clipboard-paste alternative was deliberately not enabled: asynchronous paste cannot guarantee consumption before clipboard restoration, and terminal multiline paste can execute commands. Native insertion timing is recorded separately. Actual app support must be checked during review: Accessibility setters are application-dependent and some apps report success without honoring writes. No claim of universal application compatibility or atomic compare-and-set across a user's simultaneous cursor movement is made.

Review in a disposable document: dictate while moving to another app/field; move the caret within the same field; cancel during recognition; inject a multiline list; deny Accessibility; retry twice; copy a retained failure. Terminals must not receive synthetic Return. Validate supported native text fields before release. The installed app has not been replaced.

## MAT-32 — vocabulary, modes, and commands

`vocabulary` now protects configured spellings during cleanup on both engines. Whisper also uses its glossary as a recognition prompt. Parakeet uses deterministic whole-term spelling/case corrections, not acoustic keyword boosting or fuzzy name substitution. Explicit aliases go in `profile.spellings`; they are applied once, longest match first, without substring or cascading replacements.

Example additional configuration:

```json
{
  "profile": {
    "mode": "prose",
    "vocabulary": ["ER", "Gretchen Flow"],
    "spellings": {"gretchen flo": "Gretchen Flow"},
    "spoken_commands": true
  },
  "profiles": {
    "code": {"mode": "literal"},
    "writing": {"mode": "prose", "spoken_commands": true}
  },
  "active_profile": "writing",
  "app_profiles": {"com.microsoft.VSCode": "code"}
}
```

An application mapping overrides `active_profile`, which overrides the base profile. Global vocabulary is inherited. Settings are captured once for the recording; formatting-only reloads do not recreate weights. Literal mode preserves the recognizer's output without app-added periods, filler removal, list formatting, or substitutions. It does not promise that the speech model itself will omit punctuation.

Spoken commands are opt-in: `comma`, `new paragraph`, `bullet point`. `literal comma` emits the words, and double-quoted commands stay text. Literal mode bypasses command parsing entirely. No grammar LLM was added.

## MAT-27 — measurement and corpus

Local `pipeline_timing` logs report capture-stop, resampling/gate/gain, engine wait, recognition/formatting, streaming-tail wait, insertion, history enqueue, release-to-final, and release-to-inserted. `asr_stage` separates model load, Whisper state setup/inference, Parakeet IPC/inference, and formatting. Whisper decoder fallback time is included in inference; individual internal temperature retries are not exposed by this binding. Whole-clip streaming fallbacks and worker restarts are identified. Routine logs exclude transcript text, audio, and vocabulary.

Speech-end timing is an explicitly labelled energy estimate from microphone callbacks, not a semantic VAD ground truth. Offline benchmarks cannot measure microphone shutdown, Bluetooth latency, or cursor insertion. The committed results distinguish first call/model load, warm inference, offline segmentation, and paced release-to-final. Hardware microphone, destination-app, system-contention, and energy measurements remain review tasks; no end-to-end performance target is asserted from engine-only timings.

The new pinned public corpus has 120 distinct recordings from 40 speakers, each in clean, reduced-gain, and synthetic 20dB noise conditions. Variants/repetitions are not counted as independent speakers or accuracy samples. Code/Unicode/formatting preservation is tested separately in Rust. Audiobook speech does not establish performance on natural technical dictation or all names/numbers.

## MAT-30 — work during recording

Set `streaming: true` to opt in (default false). A bounded channel sends native-rate audio to an incremental resampler. The worker recognizes completed speech regions after one second of sustained near-silence, retaining 200ms boundary padding. It accumulates text internally and inserts only the final result. Explicit release closes input and flushes the tail immediately; there is no added endpoint wait after release.

Long continuous speech (over 30 seconds without a safe split), queue overflow, sample-rate changes, or worker failure triggers one whole-recording decode. The fallback does not repeatedly decode an ever-growing buffer. Silence produces no decoder call; quiet audio above the documented energy threshold is retained. This conservative energy segmenter can miss useful splits in background noise, and very faint speech below the threshold can still be gated out. Streaming stays opt-in pending real microphone and application checks.

## MAT-34 — history

A bounded queue with one persistence thread orders append, clear, trim, configuration, and reads. Files are replaced atomically, written with mode 0600, and kept within count and byte bounds. Filesystem failures are logged by error kind and do not stop later dictation. History enqueue happens after insertion; disk persistence is off the insertion path.

`history` configuration accepts `enabled` (default true), `max_entries` (default 200; hard cap 10,000), and `max_age_days` (default 0, unlimited age). Retained text also has an 8MiB cap. Setting enabled false or count zero clears existing history when configuration is applied and stops new writes. Clear and append follow the persistence queue's ordering. A full queue drops a new history append with a metadata-only warning. Atomic replacement preserves the previous file if a rewrite fails; unsaved queued entries may be lost on process exit. Recovery audio/text is separate, temporary, and memory-only even with history disabled.

## Reproduce validation

```sh
cargo test --locked --all-targets --manifest-path desktop/src-tauri/Cargo.toml
cargo clippy --locked --all-targets --manifest-path desktop/src-tauri/Cargo.toml -- -D warnings
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --check
```

See `tools/asr-bench/REVIEW_BENCHMARK.md` for the production-module benchmark commands, data provenance, results, and remaining hardware checks.

## Validation outcome

- 53 application Rust tests pass; both example targets also pass their shared-module test suites (36 and 44 tests, mostly duplicates of application tests).
- All-target Clippy with warnings denied and formatting checks pass.
- Five existing Python scoring tests pass; summaries reject incomplete/duplicate warm passes.
- 1,440 warm recognition calls across the 360-file clean/quiet/noisy comparison; raw results, checksums, provenance, and summaries are retained.
- Ten paced streaming runs (five durations, two passes) finish without fallback, with the same aggregate normalized word errors as batch on these fixtures.
- Workspace 1 revalidated the source on 2026-11-04 for PR #1: all 53 application tests and the 36/44 example tests pass, as do locked all-target Clippy with warnings denied and formatting checks. The benchmark measurements above remain the recorded 2026-09-19 results, not new hardware measurements.
- PR #1 lands this work on the existing branch. No release or installed-app replacement is part of Workspace 1; the PR must remain unmerged until Matt chooses to merge it. Streaming remains disabled by default pending the field checks in `tools/asr-bench/REVIEW_BENCHMARK.md`.
