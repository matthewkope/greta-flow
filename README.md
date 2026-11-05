# Gretchen Flow (GF)

**Press a hotkey, speak, and your words appear wherever your cursor is.**

Gretchen Flow is an open-source voice dictation app in the spirit of Wispr Flow:
a global hotkey starts recording, your speech is transcribed locally with
Parakeet v2 by default, and the text is typed straight into whatever app has focus — your
editor, browser, chat, anywhere.

- 👧 **Gretchen lives in the menu bar** — a white Gretchen when idle; while recording she shows a black badge, next to macOS's own orange mic indicator
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

## Install with Homebrew

Apple Silicon Mac, macOS 14+ — one command (it taps and installs):

```bash
brew install --cask matthewkope/gretchen-flow/gretchen-flow
```

This copies **Gretchen Flow.app** into `/Applications`. Because the build isn't
notarized, macOS quarantines it — so after installing, clear the quarantine flag
and launch it:

```bash
brew list --cask gretchen-flow      # confirm it installed
xattr -dr com.apple.quarantine "/Applications/Gretchen Flow.app"
open "/Applications/Gretchen Flow.app"
```

On first launch, download a model in the setup window and grant **Microphone**,
**Accessibility**, and **Input Monitoring** in System Settings — see
[macOS permissions](#macos-permissions) below.

### Updating

```bash
brew update
brew upgrade --cask gretchen-flow
xattr -dr com.apple.quarantine "/Applications/Gretchen Flow.app"
open "/Applications/Gretchen Flow.app"
```

Releases are signed with a stable certificate, so your Microphone /
Accessibility / Input Monitoring grants carry over — updating does **not** make
you re-approve them.

> The cask installs from the latest [GitHub Release](https://github.com/matthewkope/gretchen-flow/releases).
> Builds are not notarized, which is why the one-time `xattr` command above is
> needed if macOS blocks the app on launch.

## Build from source

Requires Apple Silicon, macOS 14+, Swift 6.2+, [Rust](https://rustup.rs),
cmake (`brew install cmake`), and Tauri CLI (`cargo install tauri-cli --locked`).

```bash
git clone https://github.com/matthewkope/gretchen-flow.git
cd gretchen-flow/desktop/src-tauri
cargo tauri dev
```

## First run — download a model

**Gretchen Flow ships with no speech model**, so you pick the one you want.
On first launch a **setup window** opens (the menu bar icon shows a small **!**
until a model is ready). To get going:

1. Click **Download Recommended Model (464 MB)** in the setup window — or open
   the menu bar icon's **Model** submenu and choose one. The recommended
   **Parakeet TDT v2** is the English default; Whisper remains available.
   The Gretchen icon shows a **↓** while it downloads.
2. **Grant permissions** under System Settings ▸ Privacy & Security — the setup
   window has a button for each: **Microphone** (to hear you), **Accessibility**
   (to type the text), and **Input Monitoring** (to detect the Fn key).
3. **Dictate:** click into any text field, **hold the 🌐 Globe/Fn key** (the
   default) — Gretchen switches to her recording badge while she listens — then
   **let go**; your words type out where the cursor is.

You can reopen the guide anytime from the menu's **Getting Started…** item.

| Menu bar (Gretchen icon) | Meaning |
|---|---|
| white Gretchen | idle, ready |
| with ! | no model yet — open the setup guide / Model menu |
| with ↓ | downloading a model |
| **black badge** (+ macOS orange mic) | recording — release the key to finish |
| with ✕ | model failed to load (check the log) |

The macOS **orange microphone** indicator that appears while recording is the
system's own privacy indicator — it's not part of Gretchen Flow and can't be
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
  "vocabulary": ["Gretchen Flow"]
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

You grant these **once**. Released builds (the Homebrew cask) are signed with a
stable certificate, so macOS keeps your grants across `brew upgrade` — updating
the app does **not** make you re-approve them.

> Building from source is different: a plain `cargo run` / `cargo tauri build`
> is ad-hoc signed, so every rebuild is a new identity and the grants reset. If
> you're hacking on the app, use `desktop/scripts/dev-build.sh`, which signs
> each build with a stable local certificate so your grants persist between
> rebuilds (one-time setup: a self-signed "Gretchen Flow Signing" code-signing
> certificate in your keychain).

### The Fn / 🌐 Globe key vs. macOS Dictation

By default macOS binds the **🌐 Globe (Fn)** key to its own **Dictation**
feature, which intercepts the key and pops up Apple's dictation bubble — so
Gretchen never sees the press. If holding Fn does nothing (or a macOS dictation
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

The Homebrew cask downloads the signed v0.3.0 release DMG and verifies its
SHA-256 checksum. Existing explicit model choices are kept; select Parakeet v2
from the Model menu to change an older Whisper configuration.

### Backend changes awaiting review

The local five-card implementation, configuration examples, validation evidence, and remaining field checks are documented in [the review notes](docs/FIVE_CARD_REVIEW.md). Streaming is opt-in; these notes do not describe a newly published release.
