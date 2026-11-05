# Core optimization measurements

See [the Workspace 4 report](../../docs/CORE_OPTIMIZATION_0.4.md) for results and
limits. `core_bench` is an **offline proxy**, not a live microphone/AX benchmark.
It never opens a microphone, targets an app, changes configuration, or writes
history. Its sink is a local `String`; routine timing logs contain no transcript.
The result JSONL contains only transcripts of the public benchmark recordings.

## Reproduce

Use the pinned corpus and v2 model preparation from
[the ASR benchmark](../asr-bench/REVIEW_BENCHMARK.md). Run on the same Mac/helper,
sequentially, with compilation finished before timed calls. From repository root:

```sh
export GRETCHEN_PARAKEET_HELPER="$PWD/desktop/parakeet/staging/gretchen-parakeet"
MODEL="$HOME/.cache/gretchen-flow/models/parakeet-tdt-0.6b-v2-coreml"
CORPUS="$HOME/.cache/gretchen-flow/benchmarks/review-corpus"
RESULTS="$PWD/tools/core-bench/results/2026-11-04"

# Write an absolute-path proxy manifest outside tracked evidence.
python3 - "$RESULTS/manifest.json" "$CORPUS" /tmp/greta-core-manifest.json <<'PY'
import json, sys
from pathlib import Path
clips = json.load(open(sys.argv[1]))
for clip in clips:
    clip['path'] = str(Path(sys.argv[2]) / Path(clip['path']).name)
Path(sys.argv[3]).write_text(json.dumps(clips))
PY

# Compile baseline source with the same dependency graph as the final harness.
python3 tools/core-bench/prepare_baseline.py d393dc6
cargo build --locked --release --manifest-path desktop/src-tauri/Cargo.toml --example core_before
rm desktop/src-tauri/examples/core_before.rs
cargo build --locked --release --manifest-path desktop/src-tauri/Cargo.toml \
  --example core_bench --example review_bench

# Use a fresh output directory to preserve the recorded evidence.
mkdir -p /tmp/greta-core-results
for side in before after; do
  binary=core_bench
  if [ "$side" = before ]; then binary=core_before; fi
  for suffix in '' '-repeat'; do
    desktop/src-tauri/target/release/examples/"$binary" \
      "$MODEL" /tmp/greta-core-manifest.json 3 \
      > "/tmp/greta-core-results/proxy-$side$suffix.jsonl" \
      2> "/tmp/greta-core-results/proxy-$side$suffix.log"
  done
done
```

An optional fifth argument to `core_bench` is idle seconds after model readiness,
for sampling the parent/helper with `top`. It is outside the timed iterations.
The recorded subset was selected deterministically as `manifest[::9] +
manifest[1::9] + manifest[2::9]`; it has 40 recordings from 37 speakers.

For the full-corpus baseline, build `review_bench` from `d393dc6` with its original
lockfile in a separate worktree before editing. The retained `before.jsonl` was
captured this way. Run it and the final release `review_bench` with identical
`MODEL` and full `CORPUS/manifest.json`, retaining stdout as `before.jsonl` and
`after.jsonl`. Each run includes one first call and two warm batch/segmented
passes. Do not run both binaries at the same time.

Validate and summarize the retained evidence:

```sh
python3 -m unittest discover -s tools/core-bench -v
python3 tools/core-bench/summarize.py \
  tools/core-bench/results/2026-11-04 \
  tools/asr-bench/results/2026-11-04/manifest.json
```

The scorer rejects missing, unexpected or duplicate rows. It reports nearest-rank
p95 and keeps real release-to-inserted explicitly null. `chunked-*` artifacts are
intermediate development measurements excluded from the final summary. All runs
use pre-existing system/Core ML caches on a normally active desktop.

For the real-helper crash check, `restart-action.json` records the verified direct
child of a separate offline process killed during its idle window. `restart.log`
contains the subsequent worker restart and warm-up; both `restart.jsonl`
transcripts must match the same public reference run. Never target the user's
installed helper when reproducing this check.
