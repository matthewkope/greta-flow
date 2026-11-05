# Five-card implementation review

Branch: `codex/gretchen-five-card-improvements`, based on `f6163668`.
Scope: MAT-33, MAT-32, MAT-27, MAT-30, MAT-34. No frontend changes or installed-app migration.

## MAT-33 — delivery and recovery

Each recording owns its captured destination, configuration, and session ID. A usable Accessibility text target retains its element, process ID, and selected range; before delivery, all three must still match. Delivery writes `AXSelectedText` directly. If the focused element has no usable text target at capture (for example, Herdr's `AXWindow`), the recording instead retains the foreground process ID for Unicode keystroke delivery. This fallback requires that same external app to remain frontmost and checks both secure event input and secure-field focus at delivery. Missing/unknown focus, secure fields, and Greta Flow itself fail closed. It never switches from a captured native target to keystrokes after a stale target or failed setter. Neither path sends Return or pastes from the clipboard; the keystroke path replaces runs of line breaks/control characters with a single space, while native insertion preserves newlines. Failed delivery retains the original transcript for manual recovery. Successful and failed delivery results both enter the existing Recent history surface under the default history policy; cancellation/stale results do not. Repeating the same result for a session does not add duplicate history entries. A native setter failure is considered an uncertain delivery: automatic retry is suppressed to avoid duplicate text. An explicit copy is still available so the user can inspect and recover.

Optional config keys `cancel_shortcut`, `retry_shortcut`, and `recovery_shortcut` take ordinary accelerator strings; default is null to avoid taking existing system/app shortcuts. Example: `Ctrl+Alt+Escape`, `Ctrl+Alt+R`, `Ctrl+Alt+C`. Reload Config applies them. Startup logs and skips malformed, colliding, or unavailable recovery controls without aborting launch. Reload validates one configuration snapshot before changing bindings and acquires new native registrations before removing old ones; a failed acquisition preserves the working bindings. Bare modifiers are supported only for dictation, not recovery controls. Recovery copies the last transcript; it deliberately replaces the clipboard only on that explicit action. Retry reruns recognition for an unattempted session using its original configuration and destination. It cannot redirect a failed result into a newly focused field. Cancel invalidates the session and clears recovery.

Audio recovery is capped at 120 seconds and expires 120 seconds after capture, swept at most five seconds later. It is never written to disk. A new recording replaces the old recovery slot. Successful insertion frees audio immediately. Recordings over 120 seconds may still transcribe, but cannot retry from audio. Capture itself is limited to ten minutes; exceeding it rejects the recording rather than inserting a truncated result.

The automatic clipboard-paste alternative was deliberately not enabled: asynchronous paste cannot guarantee consumption before clipboard restoration, and terminal multiline paste can execute commands. Insertion timing is recorded separately. Unicode event delivery is process-bound and rechecks foreground and secure input before each event. macOS does not acknowledge whether the destination consumed those events; a failure after partial typing remains an uncertain delivery with no automatic replay. See `docs/INSERTION_FALLBACK.md` for the fallback limits and live checks. Actual app support must be checked during review: Accessibility setters are application-dependent and some apps report success without honoring writes. No claim of universal application compatibility or atomic compare-and-set across a user's simultaneous cursor movement is made.

Review in a disposable document: dictate while moving to another app/field; move the caret within the same field; cancel during recognition; inject a multiline list; deny Accessibility; retry twice; copy a retained failure. Terminals must not receive synthetic Return. Validate supported native text fields before release. The installed app has not been replaced.

## MAT-32 — vocabulary, modes, and commands

`vocabulary` now protects configured spellings during cleanup on both engines. Whisper also uses its glossary as a recognition prompt. Parakeet uses deterministic whole-term spelling/case corrections, not acoustic keyword boosting or fuzzy name substitution. Explicit aliases go in `profile.spellings`; they are applied once, longest match first, without substring or cascading replacements.

Example additional configuration:

```json
{
  "profile": {
    "mode": "prose",
    "vocabulary": ["ER", "Greta Flow"],
    "spellings": {"greta flo": "Greta Flow"},
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

An application mapping overrides `active_profile`, which overrides the base profile. Global vocabulary is inherited. Microphone start is queued at key-down before Accessibility lookup. Target/profile lookup runs independently using the key-down configuration snapshot; streaming buffers early audio while a per-app profile is pending. If the target context is not ready by release, the full audio uses the base/active profile snapshot, with no automatic insertion into a later focus. The transcript remains reachable in Recent history with the default policy. Settings are captured once for the recording; formatting-only reloads do not recreate weights. Literal mode preserves the recognizer's output without app-added periods, filler removal, list formatting, or substitutions. It does not promise that the speech model itself will omit punctuation.

Spoken commands are opt-in: `comma`, `new paragraph`, `bullet point`. `literal comma` emits the words, and double-quoted commands stay text. Literal mode bypasses command parsing entirely. No grammar LLM was added.

## MAT-27 — measurement and corpus

Local `pipeline_timing` logs report capture-stop, resampling/gate/gain, engine wait, recognition/formatting, streaming-tail wait, insertion, history enqueue, release-to-final, and release-to-inserted. `asr_stage` separates model load, Whisper state setup/inference, Parakeet IPC/inference, and formatting. Whisper decoder fallback time is included in inference; individual internal temperature retries are not exposed by this binding. Whole-clip streaming fallbacks and worker restarts are identified. Routine logs exclude transcript text, audio, and vocabulary.

Speech-end timing is an explicitly labelled energy estimate from microphone callbacks, not a semantic VAD ground truth. Offline benchmarks cannot measure microphone shutdown, Bluetooth latency, or cursor insertion. The committed results distinguish first call/model load, warm inference, offline segmentation, and paced release-to-final. Hardware microphone, destination-app, system-contention, and energy measurements remain review tasks; no end-to-end performance target is asserted from engine-only timings.

The new pinned public corpus has 120 distinct recordings from 40 speakers, each in clean, reduced-gain, and synthetic 20dB noise conditions. Variants/repetitions are not counted as independent speakers or accuracy samples. Code/Unicode/formatting preservation is tested separately in Rust. Audiobook speech does not establish performance on natural technical dictation or all names/numbers.

## MAT-30 — work during recording

Set `streaming: true` to opt in (default false). A bounded channel sends native-rate audio to an incremental resampler. The worker recognizes completed speech regions after one second of sustained near-silence, retaining 200ms boundary padding. It accumulates text internally and inserts only the final result. Explicit release closes input and flushes the tail immediately; there is no added endpoint wait after release.

Long continuous speech (over 30 seconds without a safe split), queue overflow, sample-rate changes, or worker failure triggers one whole-recording decode. The fallback does not repeatedly decode an ever-growing buffer. The original pre-gain RMS gate of **0.0015** is restored for both batch and streaming decode preparation. Below-threshold clips are discarded before gain/padding or inference to reduce near-silence hallucinations. The segmenter still uses its separate 0.0001 boundary threshold to avoid cutting low-energy speech; passing that boundary detector does not bypass the decoder gate. This conservative energy segmenter can miss useful splits in background noise, and very faint speech below the threshold can still be gated out. Streaming stays opt-in pending real microphone and application checks.

## MAT-34 — history

A bounded queue with one persistence thread orders append, clear, trim, configuration, and reads. Files are replaced atomically, written with mode 0600, and kept within count and byte bounds. Filesystem failures are logged by error kind and do not stop later dictation. History enqueue happens after the delivery attempt, including failed attempts; disk persistence is off the insertion path.

`history` configuration accepts `enabled` (default true), `max_entries` (default 200; hard cap 10,000), and `max_age_days` (default 0, unlimited age). Retained text also has an 8MiB cap. Setting enabled false or count zero clears existing history when configuration is applied and stops new writes. Clear and append follow the persistence queue's ordering. A full queue drops a new history append with a metadata-only warning. Atomic replacement preserves the previous file if a rewrite fails; unsaved queued entries may be lost on process exit. The recovery slot is separate, temporary, and memory-only even with history disabled. Distinct recognized results also follow the history policy, including failed insertions; disabling history still prevents that persistence.

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

## PR #1 review follow-up

Validation: 63 application tests and 36/47 example tests pass; locked all-target Clippy with `-D warnings` and `cargo fmt --check` pass. The ten added application tests cover failure retention, pending/late capture context, pre-gain gating, and shortcut validation/rollback. No installed app or hardware behavior was tested.

- Failed insertions are available through the existing Recent copy actions by default. History remains subject to its retention policy, queue bounds, and filesystem availability; disabling history is respected.
- Mic startup no longer waits on AX calls or bundle lookup. An unresolved/late target fails closed, with full-recording fallback and no later retargeting. This is covered with offline context/worker tests, not a hardware latency claim.
- The original 0.0015 pre-gain silence gate is restored. September benchmark results used the earlier 0.0001 gate and are historical evidence, not measurements of this corrected revision.
- Shortcut validation covers bare recovery modifiers and collisions; startup skips bad/unavailable bindings, and reload preserves the old dispatch table on native registration failure. Regression tests use a fake native registrar; actual macOS hotkeys remain a field check.

### Frontend handoff for Claude (Workspace 5)

The dedicated recovery surface is **deferred to Claude** under the plan's frontend ownership rule. Add a visible delivery-failure state and explicit manual-copy action for `recover_dictation`, with cancellation/retry controls as appropriate. This is needed particularly when history is disabled and no recovery shortcut is configured. Explain that an uncertain native setter may already have inserted text, so the user must inspect the destination before copying/retrying. The recovery slot expires after 120 seconds and is replaced by a new recording. No window/menu wording or layout is changed in this backend PR; default-history users can use the existing Recent actions now.
