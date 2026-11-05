"""Greta Flow entry point: hotkey -> record -> transcribe -> type."""

from __future__ import annotations

import argparse
import sys
import threading

from . import __version__
from .config import CONFIG_PATH, Config
from .engines import create_engine
from .hotkey import HotkeyListener
from .injector import inject
from .recorder import Recorder


class GretchenFlow:
    def __init__(self, config: Config):
        self.config = config
        self.engine = create_engine(config.engine, config.model)
        self.recorder = Recorder(sample_rate=config.sample_rate)

    def run(self) -> None:
        print(f"Gretchen Flow v{__version__}")
        print(f"  engine:  {self.config.engine} ({self.config.model})")
        print(f"  hotkey:  {self.config.hotkey} ({self.config.hotkey_mode})")
        print("Loading model (first run downloads it — this can take a few minutes)...")
        self.engine.warm_up()
        print("Ready. Press the hotkey and speak. Ctrl+C to quit.")

        listener = HotkeyListener(
            self.config.hotkey,
            self.config.hotkey_mode,
            on_start=self._start_recording,
            on_stop=self._stop_and_transcribe,
        )
        listener.start()
        try:
            listener.join()
        except KeyboardInterrupt:
            print("\nBye.")

    def _start_recording(self) -> None:
        print("● recording...", flush=True)
        self.recorder.start()

    def _stop_and_transcribe(self) -> None:
        audio = self.recorder.stop()
        seconds = len(audio) / self.config.sample_rate
        if seconds < 0.3:
            print("(too short, ignored)")
            return
        print(f"○ transcribing {seconds:.1f}s...", flush=True)
        # Off the listener thread so the hotkey stays responsive.
        threading.Thread(target=self._transcribe, args=(audio,), daemon=True).start()

    def _transcribe(self, audio) -> None:
        try:
            text = self.engine.transcribe(audio, self.config.sample_rate, self.config.language)
        except Exception as exc:  # noqa: BLE001 - surface engine errors, keep running
            print(f"transcription failed: {exc}", file=sys.stderr)
            return
        if text:
            print(f"→ {text}")
            inject(text, self.config.injection)
        else:
            print("(no speech detected)")


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(prog="gf", description="Push-to-talk voice dictation.")
    parser.add_argument("--engine", help="faster-whisper | mlx-whisper")
    parser.add_argument("--model", help="e.g. large-v3-turbo, small, base")
    parser.add_argument("--language", help="language code, e.g. en (default: from config)")
    parser.add_argument("--hotkey", help="e.g. '<ctrl>+<alt>+<space>'")
    parser.add_argument("--mode", choices=["toggle", "hold"], help="hotkey behavior")
    parser.add_argument("--version", action="version", version=f"%(prog)s {__version__}")
    parser.add_argument(
        "--write-config", action="store_true", help=f"save current settings to {CONFIG_PATH}"
    )
    args = parser.parse_args(argv)

    config = Config.load()
    if args.engine:
        config.engine = args.engine
    if args.model:
        config.model = args.model
    if args.language:
        config.language = args.language
    if args.hotkey:
        config.hotkey = args.hotkey
    if args.mode:
        config.hotkey_mode = args.mode
    if args.write_config:
        config.save()
        print(f"Saved config to {CONFIG_PATH}")

    GretchenFlow(config).run()


if __name__ == "__main__":
    main()
