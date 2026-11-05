# Architecture

## Goal

A Wispr Flow-style dictation tool: global hotkey → record → transcribe as
accurately as possible → inject text into the focused app. Open source,
local-first, hackable.

## Options considered

### Option 1 — Electron + whisper.cpp (TypeScript)

Web-stack desktop app bundling [whisper.cpp](https://github.com/ggml-org/whisper.cpp)
via native bindings.

- ✅ Familiar stack for many contributors; easy to build polished UI
- ✅ Cross-platform installers out of the box
- ❌ ~200 MB baseline memory and large bundles for what is mostly a background utility
- ❌ Native audio capture + global hotkeys + whisper.cpp bindings in Electron are
  fiddly; the hardest parts of the app end up outside the web stack anyway

### Option 2 — Tauri 2 + Rust (whisper-rs)

Native Rust core (audio, hotkeys, inference via whisper-rs/CTranslate2) with a
tiny webview for settings UI.

- ✅ Best end product: ~10 MB binary, low latency, real menu-bar app, signed installers
- ✅ Rust is a great fit for the audio/hotkey/inference pipeline
- ❌ Slowest to first working version; smaller contributor pool
- ❌ Iterating on STT quality (models, VAD, prompting, formatting) is slower in Rust

### Option 3 — Python + faster-whisper (chosen)

Python package: `pynput` (hotkeys + typing), `sounddevice` (mic),
`faster-whisper` (inference), optional `mlx-whisper` on Apple Silicon.

- ✅ Fastest path to a genuinely working tool (the whole pipeline is ~300 lines)
- ✅ The entire speech/ML ecosystem is Python — swapping models, adding VAD,
  cloud engines, or custom vocab is trivial
- ✅ `faster-whisper` (CTranslate2) is among the most accurate local options and
  ~4× faster than reference Whisper; `mlx-whisper` adds Apple Silicon speed
- ❌ Users need Python/uv (mitigated: `uv` makes install a one-liner; packaged
  app planned)
- ❌ A polished menu-bar UI is harder in Python (acceptable for now; see below)

## Decision

**Option 2 (Tauri 2 + Rust) is the app**, in `desktop/`. The Python prototype
(Option 3, in `python/`) validated the pipeline end-to-end in an afternoon and
remains useful for fast model experiments, but the real product needs a menu-bar
presence, low latency, and an installable binary — Rust territory.

## Desktop app components (`desktop/src-tauri/src/`)

| Component | File | Responsibility |
|---|---|---|
| App / tray | `main.rs` | Tray icon (¿ / ↓¿ / ● / …), shortcut wiring, state machine |
| Hotkey | (tauri-plugin-global-shortcut) | Toggle & hold modes via Pressed/Released events |
| Recorder | `audio.rs` | cpal capture on a dedicated thread; mono downmix + 16 kHz resample |
| Engine | `transcribe.rs` | whisper-rs (whisper.cpp), Metal-accelerated, beam search |
| Models | `model.rs` | ggml model download to `~/.cache/gretchen-flow/models/` |
| Injector | `inject.rs` | Validated Accessibility insertion; capture-time, process-bound Unicode keystroke fallback for unsupported fields |
| Config | `config.rs` | Shared JSON config in `~/.config/gretchen-flow/` |

The tray icon doubles as the recording indicator — the gap that made the
Python prototype's toggle mode confusing (no way to tell if it was recording).

## Python prototype components (`python/src/gretchen_flow/`)

Mirrors the same pipeline: `hotkey.py`, `recorder.py`, `engines/`
(faster-whisper, mlx-whisper), `injector.py`, `config.py`, `app.py`.

## Accuracy levers (in priority order)

1. **Model**: `large-v3-turbo` default; `large-v3` for max accuracy, `small`/`base` for speed
2. **VAD filter**: trims silence so Whisper doesn't hallucinate on dead air
3. **Beam search** (`beam_size=5`) over greedy decoding
4. **Language pinning**: skip auto-detect when the user knows their language
5. Planned: initial-prompt vocabulary biasing, post-formatting pass, cloud engines
