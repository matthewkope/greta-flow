# Greta Flow (GF)

**Press a hotkey, speak, and your words appear wherever your cursor is.**

Greta Flow is an open-source voice dictation app in the spirit of Wispr Flow:
a global hotkey starts recording, your speech is transcribed locally with
Parakeet v2 by default, and the text is typed straight into whatever app has focus — your
editor, browser, chat, anywhere.

- 👧 **Greta lives in the menu bar** — a white Greta when idle; while recording she shows a black badge, next to macOS's own orange mic indicator
- 🎙️ **Push-to-talk or toggle** — hold a key to talk, or tap to start/stop; the **Fn / 🌐 Globe** key works as a hotkey, and you can have up to 3 shortcuts
- ✍️ **Speech-aware grammar** — pausing while you speak inserts a period; filler
  words ("um", "uh") are removed automatically
- 📋 **Spoken lists** — "one, buy milk… two, walk the dog" becomes a real
  numbered list, typed line by line
- 📖 **Personal dictionary** — add names and jargon so they're recognized and
  spelled right
- 🕘 **History** — recent dictations live in the tray menu; click one to type it again
- 🔒 **Local-first** — audio never leaves your machine
- 🎯 **Accurate and lightweight** — Parakeet TDT v2 (~464 MB), with optional Whisper models
  with Metal acceleration on Apple Silicon
- 🦀 **Native** — Tauri 2 + Rust; small binary, low latency

## Repository layout

| Directory | What it is |
|---|---|
| [`desktop/`](desktop/) | **The app** — Tauri 2 + Rust menu-bar application |
| [`python/`](python/) | The original Python prototype (still works; great for experimenting with models) |
| [`docs/`](docs/) | Architecture and design decisions |

## Install

Requires **Apple Silicon and macOS 14 or later**. Greta Flow is free and open
source. Releases use a stable self-signed certificate and are **not notarized**.
**For the published 0.3.0 release, use the [manual DMG route](#manual-dmg-download).**
The renamed Homebrew cask, rename map, and release installer will be available
from the next release after its distribution workflow finishes successfully.
The commands below for those two routes are for that release, not 0.3.0.

Choose one route and use that same route for updates. The upcoming cask and
installer clear the app's quarantine attribute; use them only if you trust this
project.

### Homebrew (available from the next release)

```bash
brew update
brew install --cask matthewkope/greta-flow/greta-flow
open "/Applications/Greta Flow.app"
```

After publication, the personal tap will clear quarantine on install and upgrade. Update
Homebrew first so it supports the cask's `postflight_steps` stanza.

To update, quit Greta Flow, then:

```bash
brew update
brew upgrade --cask greta-flow
open "/Applications/Greta Flow.app"
```

After the next release publishes the rename map, for an old `gretchen-flow`
installation, keep your existing tap (its URL redirects),
quit the app, and run `brew update` followed by
`brew reinstall --cask matthewkope/gretchen-flow/greta-flow`. The tap's
`cask_renames.json` migrates the token, and Homebrew removes the old app artifact.
The migration command cannot resolve until the new cask and rename map are published.

### Release installer (no Homebrew)

Starting with the next release, each release includes `install.sh`, pinned to
that release's DMG and SHA-256. Download it, inspect it if desired, and run it:

```bash
curl -fL --proto '=https' --proto-redir '=https' \
  https://github.com/matthewkope/greta-flow/releases/latest/download/install.sh \
  -o /tmp/greta-flow-install.sh
bash /tmp/greta-flow-install.sh
```

Quit the app and repeat these commands to update. The script checks the platform,
verifies the DMG checksum before mounting it, verifies the bundle signature,
installs into `/Applications`, clears quarantine, and opens Greta Flow. It asks
for your administrator password only if `/Applications` is not writable. It
removes the legacy `Gretchen Flow.app` only after installing the replacement.
The checksum detects damaged or mismatched downloads; it is not independent
verification of the GitHub release publisher.

The source-tree `install.sh` is a template and intentionally refuses to install.
The existing 0.3.0 release does not have this installer; use the manual route
until the next release is published.

### Manual DMG download

Download the Apple Silicon DMG from [Releases](https://github.com/matthewkope/greta-flow/releases).
For new releases, download `SHA256SUMS` beside the DMG and run
`shasum -a 256 -c SHA256SUMS` in that download directory before opening it.
Open the DMG, drag **Greta Flow.app** to **Applications**, eject the DMG, then
open the installed app. If macOS blocks it, open **System Settings → Privacy &
Security → Open Anyway**, authenticate, and confirm Open. Try opening the app
once first so the Open Anyway option appears. Do not rely on Control-click on
macOS 15 or later. On an update, quit first and replace the existing app.
The 0.3.0 DMG still contains **Gretchen Flow.app**; the next release uses Greta.
If upgrading manually from that name, remove the old app after copying the new
one, leaving your settings and model cache in place.

### First launch and update validation

Download the recommended **Parakeet v2** model (~464 MB) in setup and grant
**Microphone**, **Accessibility**, and **Input Monitoring**; see
[macOS permissions](#macos-permissions). There is no in-app updater.
Settings, history, and model paths retain `gretchen-flow`, so updates do not need
a model re-download. The bundle ID and signing certificate remain unchanged.
Permission preservation and launch behavior on clean macOS 14, 15, and current
release accounts still require [field validation](docs/DISTRIBUTION_0.4.md).

### Uninstall

Quit the app. For Homebrew, run `brew uninstall --cask greta-flow`. For the other
routes, move `/Applications/Greta Flow.app` (and any old `Gretchen Flow.app`) to
Trash. This keeps your data for a later reinstall.

To **permanently delete settings, downloaded models, and dictation history** too:

```bash
rm -rf "$HOME/.config/gretchen-flow" "$HOME/.cache/gretchen-flow" \
  "$HOME/Library/Application Support/gretchen-flow"
```

## Build from source

Requires Apple Silicon, macOS 14+, Swift 6.2+, [Rust](https://rustup.rs),
cmake (`brew install cmake`), and Tauri CLI (`cargo install tauri-cli --locked`).

```bash
git clone https://github.com/matthewkope/greta-flow.git
cd greta-flow/desktop/src-tauri
cargo tauri dev
```

## First run — download a model

**Greta Flow ships with no speech model**, so you pick the one you want.
On first launch a **setup window** opens (the menu bar icon shows a small **!**
until a model is ready). To get going:

1. Click **Download Recommended Model (464 MB)** in the setup window — or open
   the menu bar icon's **Model** submenu and choose one. The recommended
   **Parakeet TDT v2** is the English default; Whisper remains available.
   The Greta icon shows a **↓** while it downloads.
2. **Grant permissions** under System Settings ▸ Privacy & Security — the setup
   window has a button for each: **Microphone** (to hear you), **Accessibility**
   (to type the text), and **Input Monitoring** (to detect the Fn key).
3. **Dictate:** click into any text field, **hold the 🌐 Globe/Fn key** (the
   default) — Greta switches to her recording badge while she listens — then
   **let go**; your words type out where the cursor is.

You can reopen the guide anytime from the menu's **Run Setup Again…** item.

| Menu bar (Greta icon) | Meaning |
|---|---|
| white Greta | idle, ready |
| with ! | no model yet — open the setup guide / Model menu |
| with ↓ | downloading a model |
| **black badge** (+ macOS orange mic) | recording — release the key to finish |
| with ✕ | model failed to load (check the log) |

The macOS **orange microphone** indicator that appears while recording is the
system's own privacy indicator — it's not part of Greta Flow and can't be
hidden.

### Configuration

`~/.config/gretchen-flow/config.json`:

```json
{
  "model": "parakeet-tdt-0.6b-v2",
  "language": "en",
  "shortcuts": ["Fn"],
  "hotkey_mode": "hold",
  "pause_punctuation_ms": 700,
  "remove_fillers": true,
  "auto_lists": true,
  "vocabulary": ["Greta Flow"]
}
```

- `model`: defaults to `parakeet-tdt-0.6b-v2` (~464 MB, English only).
  Existing explicit model selections are preserved; select Parakeet from the
  **Model** menu to switch. Whisper files are not deleted. Also accepts ggml model names from
  [whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp) —
  `large-v3-turbo-q5_0` (~547 MB), `large-v3-turbo` (~1.6 GB),
  `small` (~466 MB), `base` (~142 MB) — or an absolute path to a local
  `.bin`/`.gguf` model file (also selectable via **Model ▸ Model from File…**)
- `hotkey_mode`: `"hold"` (push-to-talk — records while held, default) or `"toggle"` (tap to start/stop)
- `shortcuts`: up to **3** push-to-talk hotkeys; the default is the **Fn / 🌐
  Globe** key (`"Fn"`). Each is a [Tauri accelerator](https://v2.tauri.app/learn/global-shortcut/)
  like `"Cmd+Shift+D"`, except `"Fn"` which is the special Globe key. Add, change
  (click a shortcut to re-record it), or remove them from the window's
  **Shortcuts** section — changes save here immediately
- `pause_punctuation_ms` (Whisper only): pausing this long while speaking inserts a period and
  capitalizes the next sentence (smaller models often skip punctuation; this
  recovers it from your speech rhythm). Set `0` to disable.
- `remove_fillers`: strips "um", "uh", "hmm", etc. from the output
- `auto_lists`: formats spoken lists ("one, … two, …" / "first, … second, …" /
  "number one, …") as numbered lines; conservative, so narrative sentences like
  "One day I went out" are left alone
- `vocabulary` (Whisper only): words and phrases (names, jargon, brands) that recognition is
  biased toward, so they come out spelled the way you wrote them

Most settings can also be changed from the window/tray menu (model, shortcuts).
After hand-editing `config.json`, choose **Reload Config** from the menu to
apply every change live — no restart needed.

Dictation history is saved to `~/Library/Application Support/gretchen-flow/history.jsonl`;
the five most recent entries appear in the tray menu — click one to type it again.

### macOS permissions

System Settings → Privacy & Security — the app (or your terminal, when using
`cargo run`) needs:

1. **Microphone** — to hear you
2. **Accessibility** — to type the transcribed text into other apps
3. **Input Monitoring** — to detect the Fn / 🌐 Globe key

Released builds retain a stable signing certificate and bundle ID. The app-path
rename still needs the real upgrade validation described above.

> Building from source is different: a plain `cargo run` / `cargo tauri build`
> is ad-hoc signed, so every rebuild is a new identity and the grants reset. If
> you're hacking on the app, use `desktop/scripts/dev-build.sh`, which signs
> each build with a stable local certificate so your grants persist between
> rebuilds. Use the existing "Gretchen Flow Signing" identity; do not create a
> replacement certificate for this rename.

### The Fn / 🌐 Globe key vs. macOS Dictation

By default macOS binds the **🌐 Globe (Fn)** key to its own **Dictation**
feature, which intercepts the key and pops up Apple's dictation bubble — so
Greta never sees the press. If holding Fn does nothing (or a macOS dictation
popup appears), turn Apple's Dictation off and free the key:

- **System Settings ▸ Keyboard ▸ Dictation** → **Off**
- **System Settings ▸ Keyboard ▸ "Press 🌐 key to"** → anything except
  *Start Dictation* (e.g. *Do Nothing*)

Or just use one of your own custom shortcuts instead of Fn.

## Model size and memory

Parakeet v2 uses about **464 MB of model files** (0.46 GB) on disk. Its native
CoreML helper stays running alongside the Tauri app, keeping the model loaded
between dictations. Runtime memory is higher than the model's disk size and
varies with audio length and CoreML caches; a full app-plus-helper memory
benchmark has not yet been established.

Whisper models remain optional and are never deleted by switching defaults.
The unquantized large-v3-turbo model is about 1.62 GB on disk.

## Roadmap

- [ ] **Fix Fn / 🌐 Globe dictation** — make hold-to-talk on the Globe key work
      without having to turn off macOS Dictation first (today the system
      intercepts the key)
- [ ] **Coexist with the Globe key's built-in functions** — hold Globe to
      dictate while a normal tap still opens the Emoji & Symbols picker (and
      input-source switching), so you keep both
- [ ] Settings window (model picker, shortcut recorder, dictionary editor)
- [ ] Streaming transcription (text appears while you speak)
- [ ] Self-correction cleanup ("Tuesday — no wait, Wednesday" → "Wednesday")
- [ ] Per-app tone/formatting awareness
- [ ] Signed .dmg releases
- [ ] Linux and Windows support

## Contributing

PRs welcome! See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the design.

```bash
cd desktop/src-tauri && cargo build   # the app
cd python && uv sync && uv run pytest # the prototype
```

## License

[MIT](LICENSE)


### Building the Parakeet version

Version 0.3.0 requires Apple Silicon, macOS 14+, and Swift
6.2+ (Xcode 26.2 works). `cargo tauri build` / `cargo tauri dev` builds and
bundles the pinned native Parakeet helper automatically. For raw `cargo run`,
first run `desktop/scripts/build-parakeet.sh` from the repository root.
Model weights download separately and remain local. Parakeet controls its own
punctuation; thinking pauses can still produce unwanted periods. This update
does not claim to fix that behavior. See [benchmark results](docs/ASR_BENCHMARK_2026-09-18.md).

The upcoming Homebrew cask will download the signed release DMG and verify its
SHA-256 checksum. Existing explicit model choices are kept; select Parakeet v2
from the Model menu to change an older Whisper configuration.

### Backend changes awaiting review

The local five-card implementation, configuration examples, validation evidence, and remaining field checks are documented in [the review notes](docs/FIVE_CARD_REVIEW.md). Streaming is opt-in; these notes do not describe a newly published release.
