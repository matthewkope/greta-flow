#!/usr/bin/env python3
"""Prepare immutable release assets and the matching cask; no network or signing."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil


def prepare(root: Path, tag: str, dmg: Path, output: Path) -> None:
    if not re.fullmatch(r"v\d+\.\d+\.\d+", tag):
        raise ValueError("Only stable vMAJOR.MINOR.PATCH releases update the cask")
    version = tag[1:]
    config = json.loads((root / "desktop/src-tauri/tauri.conf.json").read_text())
    if config["version"] != version:
        raise ValueError("Tag and tauri.conf.json version differ")
    expected = f"Greta Flow_{version}_aarch64.dmg"
    if dmg.name != expected or not dmg.is_file():
        raise ValueError(f"Expected exactly {expected}")
    digest = hashlib.sha256(dmg.read_bytes()).hexdigest()
    # Stable URL spelling avoids spaces and matches the cask and installer.
    asset = expected.replace(" ", ".")
    output.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(dmg, output / asset)
    (output / "SHA256SUMS").write_text(f"{digest}  {asset}\n")
    installer = (root / "install.sh").read_text()
    # Replace declarations only, retaining the source-template guard.
    for key, value in (("VERSION", version), ("DMG", asset), ("SHA256", digest)):
        installer = installer.replace(f"{key}='@{key}@'", f"{key}='{value}'")
    (output / "install.sh").write_text(installer)
    (output / "install.sh").chmod(0o755)
    cask = (root / "Casks/greta-flow.rb").read_text()
    cask = re.sub(r'^  version ".*"$', f'  version "{version}"', cask, flags=re.M)
    cask = re.sub(r'^  sha256 ".*"$', f'  sha256 "{digest}"', cask, flags=re.M)
    cask = re.sub(r'^  url ".*"$', '  url "https://github.com/matthewkope/greta-flow/releases/download/v#{version}/Greta.Flow_#{version}_aarch64.dmg"', cask, flags=re.M)
    cask = cask.replace('  # The published 0.3.0 archive keeps its original name and checksum.\n  # Homebrew removes the old cask\'s app artifact during migration/reinstall.\n', '')
    cask = re.sub(r'^  app .*$', '  app "Greta Flow.app"', cask, flags=re.M)
    (output / "greta-flow.rb").write_text(cask)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--dmg", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    prepare(Path(__file__).resolve().parents[2], args.tag, args.dmg, args.output)
