"""Rebuild core_bench against a chosen commit's production modules, in target/."""
import subprocess
import sys
from pathlib import Path

root = Path(__file__).resolve().parents[2]
revision = subprocess.check_output(
    ["git", "rev-parse", "--verify", "--end-of-options", f"{sys.argv[1]}^{{commit}}"],
    cwd=root, text=True).strip()
source = root / "desktop/src-tauri/target/core-before/src"
source.mkdir(parents=True, exist_ok=True)
files = subprocess.check_output(
    ["git", "ls-tree", "-r", "--name-only", revision, "desktop/src-tauri/src"],
    cwd=root, text=True).splitlines()
for file in files:
    (source / Path(file).name).write_bytes(subprocess.check_output(
        ["git", "show", f"{revision}:{file}"], cwd=root))
example = root / "desktop/src-tauri/examples/core_bench.rs"
text = example.read_text().replace("../src/", "../target/core-before/src/")
# The old modules don't use this trait. The common harness still declares it.
text = text.replace("../target/core-before/src/locks.rs", "../src/locks.rs")
example.with_name("core_before.rs").write_text(text)
print("Prepared core_before.rs; remove it after building the baseline executable.")
