# Parakeet test build

Version 0.3.0-beta.1, macOS 14+ on Apple Silicon. Parakeet TDT 0.6B v2 is
the default in this test build. The source version now also defaults to Parakeet; this document describes the
separate 0.3.0-beta.1 test installation. No frontend files were changed for this integration.

## Try it

1. Quit the existing Gretchen Flow from its tray menu so both apps do not
   capture the same shortcut.
2. Open `/Applications/Gretchen Flow Test.app`.
3. Give **Gretchen Flow Test** Microphone, Accessibility, and Input Monitoring
   access in System Settings when requested. Its separate identity requires
   its own grants. Restart it after enabling Input Monitoring if necessary.
4. Wait for the model to finish loading, then use the normal dictation shortcut.
   Parakeet stays loaded between dictations. Avoid starting another recording
   while the tray shows transcription in progress.

Test preferences: `~/.config/gretchen-flow-test/config.json`.
Test history: `~/Library/Application Support/gretchen-flow-test/history.jsonl`.
The app shares only the downloaded model cache with the regular app. The
regular app and its preferences/history have not been replaced.

English only. Parakeet supplies its own punctuation; the configurable pause
punctuation threshold and Whisper's vocabulary prompt do not apply to it.
Filler cleanup and spoken-list formatting still apply. Select Whisper from
the model menu if you need its language or vocabulary behavior. Accuracy on
your speech still needs hands-on validation; the small earlier corpus is not
proof of generally better accuracy.

This local beta is ad-hoc signed, not notarized. The installed signing
certificate failed with `errSecInternalComponent`. Rebuilding can require
renewed permissions. No release has been published.

## Build and validate

Run `desktop/scripts/test-build.sh` with Swift 6.2+, the Rust/Tauri toolchain,
and Xcode tools installed. FluidAudio v0.18.2 is pinned to revision
`e396cf780c4abf8e395087467c0585d3b0d17f68`. For an existing verified source
checkout, `FLUID_AUDIO_PATH` can avoid fetching it again.

The helper is a release-optimized Swift executable bundled inside the app.
It holds CoreML models in memory, exchanges bounded PCM frames/JSON replies
through pipes, and never writes captured audio to disk. Startup has a 120s
timeout; transcription has a 60s timeout. Failed workers are killed/reaped,
and the next dictation restarts the worker. Dictations over 10 minutes are
rejected. Model downloads use a pinned revision and per-file SHA-256 checks.

Offline packaged-engine test (16 kHz mono little-endian float32 files):

```sh
'/Applications/Gretchen Flow Test.app/Contents/MacOS/gretchen-flow' \
  --self-test /path/to/manifest.json
```

Manifest format: `[{"id":"clip-1","path":"/absolute/path/clip.f32"}]`.
This command skips microphone capture, text injection, permissions, and history.
It prints transcripts and inference times to stdout. Therefore it validates
packaging and backend inference, not real microphone-to-cursor latency.

Independent reviewer: the separate `backend_review` agent reviewed the panic/
Unicode fixes and the Parakeet integration. Its two integration findings
(packaging scope and asynchronous model-selection consistency) were fixed;
final review reported no remaining beta blockers.

## Validation on this Mac (2026-09-18)

- Installed `/Applications/Gretchen Flow Test.app`, version 0.3.0-beta.1.
- `codesign --verify --deep --strict`: passed.
- Packaged offline self-test: 24/24 real LibriSpeech clips returned text;
  median backend inference 57.79 ms, first call 74.17 ms, maximum 90.23 ms.
  Load time excluded. Results: `tools/asr-bench/results/2026-09-18/packaged-parakeet-beta.jsonl`.
- 33 app unit tests and 27 shared benchmark tests passed, including malformed
  worker replies, closed pipes, and a startup timeout that kills a sleeping child.
- Clippy with warnings denied passed for all test-build targets. Normal-build
  targets also compile. Formatting passes.
- GUI microphone permissions and live typing into another app have not been
  exercised; these require hands-on testing with the separate beta identity.
