# Gretchen Flow — working notes for Claude

Open-source macOS push-to-talk dictation: hold a hotkey, speak, and insert the
transcript at the captured cursor destination. Recognition is local: no API keys,
cloud recognition, or telemetry. Network access is limited to model downloads.

## Scope and ownership

Follow `docs/PLAN_0.4.md`, including its Ground rules and workspace boundaries.
Astra owns backend code, models, optimization, release, and distribution. Claude
owns `desktop/ui/`, icons, sprites, `desktop/scripts/gen_icons.py`, and window and
menu layout/wording (including Rust tray labels). Backend work that needs a UI
change must leave a note in its PR for Claude.

Workspace 1 lands the existing five-card work. Renaming, model upgrades,
optimization, design, and distribution belong to the later workspaces. Keep
`streaming: false` until the field checks in
`tools/asr-bench/REVIEW_BENCHMARK.md` pass. Do not change defaults in a refactor.

## Layout

| Path | What |
|---|---|
| `desktop/src-tauri/` | Tauri 2 + Rust desktop app. |
| `desktop/parakeet/` | Swift/FluidAudio Core ML helper for the default Parakeet v2 engine. |
| `desktop/ui/` | Webview windows; HTML loads external JS under the app CSP. |
| `desktop/scripts/gen_icons.py` | Tray and app icon generation; Claude-owned. |
| `tools/asr-bench/` | Offline benchmark tools, public corpus provenance, and results. |
| `python/` | Original prototype, separate from the desktop product. |
| `Casks/gretchen-flow.rb` | Homebrew cask. |
| `.github/workflows/release.yml` | Tagged release build and signing. |

## Build and validation

From the repository root, before each PR:

```sh
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --check
cargo clippy --locked --all-targets --manifest-path desktop/src-tauri/Cargo.toml -- -D warnings
cargo test --locked --all-targets --manifest-path desktop/src-tauri/Cargo.toml
```

Use `cargo fmt --manifest-path desktop/src-tauri/Cargo.toml` if formatting is
needed. `cargo run --manifest-path desktop/src-tauri/Cargo.toml` runs a development
binary; it does not establish bundled-app behavior. See `docs/FIVE_CARD_REVIEW.md`
for implementation details and `tools/asr-bench/REVIEW_BENCHMARK.md` for benchmark
reproduction and remaining real-hardware checks. Offline tests do not validate
microphones, Bluetooth, app insertion, or first-install Core ML initialization.

## Architecture (`desktop/src-tauri/src/`)

- `main.rs`: application state, tray/windows, commands, hotkeys, and orchestration.
- `engine.rs`, `parakeet.rs`, `transcribe.rs`: engine selection, persistent Parakeet
  helper, and Whisper fallback. Configuration is captured per recording.
- `audio.rs`, `streaming.rs`: capture, bounded audio streaming, segmentation,
  preparation, and whole-recording fallback. Streaming is opt-in.
- `session.rs`, `inject.rs`: session identity, bounded memory-only recovery, and
  validated Accessibility insertion. Uncertain writes are not automatically retried.
- `profile.rs`, `config.rs`: formatting modes, vocabulary/spellings, spoken commands,
  and per-app profiles. Parakeet spelling correction is not acoustic boosting.
- `history.rs`: bounded persistence queue, retention policy, atomic private files.
- `metrics.rs`: local stage timing without transcripts, audio, or vocabulary.
- `model.rs`, `lists.rs`, `self_test.rs`: model resolution, lists, and offline checks.

Tray menu items are mutated in place to avoid hover glitches. Icons are embedded
with `include_bytes!`, so changing an icon requires recompilation.

## macOS and identity

- Preserve bundle ID `com.matthewkope.gretchenflow` and the existing certificate
  named `Gretchen Flow Signing`. Never replace the certificate, read/print/copy
  `~/gretchen-flow-signing/`, or change code identity to work around permissions.
- Releases use stable self-signed signing through the release workflow. The base
  Tauri config uses `signingIdentity: "-"`; this alone does not describe release
  signing. Do not replace the release identity with ad-hoc signing.
- Raw development binaries and differently signed bundles are not evidence that
  permissions survive a real upgrade. Do not replace the installed app as part
  of offline validation.
- Fn/Globe uses a `CGEventTap` and requires Input Monitoring. Keep keyboard-layout
  calls out of that event tap. Ordinary accelerators use the global-shortcut plugin.
- Run AppKit UI/clipboard operations on the main thread. Window dragging requires
  `core:window:allow-start-dragging` in the capabilities file.
- macOS controls tray placement and the orange microphone indicator. Users can
  Command-drag a tray item out from behind the notch.

## Conventions

- GitHub account: `matthewkope`; run `gh auth status` before any `gh` command, push,
  or PR operation, switching to that account if necessary.
- No AI attribution footers or co-author trailers in commits or PRs.
- Use the assigned branch/worktree; Workspace 1 uses the existing checkout.
  Follow the user's authorization for committing/pushing. Do not merge unless asked.
- Preserve config at `~/.config/gretchen-flow/config.json`, models at
  `~/.cache/gretchen-flow/models/`, and history at
  `~/Library/Application Support/gretchen-flow/history.jsonl` without a tested migration.
