# Greta Flow (Python prototype)

Press a hotkey, speak, and your words appear wherever your cursor is —
open-source, local-first voice dictation powered by Whisper.

This package is the **Python prototype** of [Gretchen Flow](https://github.com/matthewkope/greta-flow).
It validates the full pipeline (global hotkey → record → transcribe → type) and
is handy for experimenting with models and engines. The shipping desktop app is
a Tauri 2 + Rust menu-bar application; see the project README for that.

## Install

```bash
uv sync                 # or: pip install gretchen-flow
uv run gf               # start dictating
```

On Apple Silicon, add the faster MLX engine with `uv sync --extra mlx`.

## Engines

- `faster-whisper` (default, cross-platform, CTranslate2)
- `mlx-whisper` (fastest on Apple Silicon)

## Configuration

Settings live in `~/.config/gretchen-flow/config.json` (engine, model, hotkey,
language, injection mode). Run `gf --help` for command-line overrides.

## macOS permissions

Grant **Microphone** (to hear you) and **Accessibility** (to type the text) to
the terminal/app running Gretchen Flow, under
System Settings → Privacy & Security.

## License

[MIT](LICENSE)
