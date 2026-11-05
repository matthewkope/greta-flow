# Bundled release field check

Run with Python 3.9+ on macOS, from a terminal in a graphical login session.
This launches the supplied bundle executable as a child of the terminal and
terminates that launched app when finished. Quit other copies first. Use the
default Parakeet engine; a single direct Parakeet helper is required.
It does not install a build, modify permissions/configuration, or record audio.
The app itself still follows its existing history settings; use disposable text.

**Permission limitation:** this executes the binary directly, not through
LaunchServices. macOS TCC may attribute Microphone, Accessibility, or Input
Monitoring access to Terminal (or the terminal host) rather than the app bundle.
Permission attribution has not been verified in a real run. Do not treat these
measurements as evidence that a Finder-launched app has working permissions or
that upgrade permissions survive. Keep the launch method and permission context
consistent for both measurements; validate normal app-launch permissions separately.

```sh
# Optional: authorize Apple's power sampler in this terminal first.
# The script uses sudo -n and never collects a password.
sudo -v
python3 tools/release-check/release_check.py \
  --app '/path/to/Greta Flow.app' \
  --output /tmp/greta-candidate-normal-mic.json
```

Wait for readiness, make a warm-up dictation, then follow the terminal prompts.
During the dictation window, perform at least 20 hold/release dictations into a
disposable supported native field. Check that text actually appears: an AX setter
returning success is not proof that the destination honored it. Wait for the last
insertion before ending the window. Then stop dictation and other busy work for
the idle window (30 seconds by default). Run normal mic and Bluetooth separately.
Do not include warm-up or mix microphone types in a single report. Keep the same
phrases, order, model, configuration, destination, hardware and background load
for both builds. These conditions and visual verification are operator duties;
the tool cannot verify them. Capture at least the same number of samples per side.

To obtain the missing matched baseline, run the same procedure against a bundled
PR #4 **before** build (source `d393dc6`, as documented in the core report), then
repeat against the candidate and pass `--baseline /tmp/greta-before.json`.
Only use reports from this tool for that option. Do not pass the offline benchmark
summary. Reports identify the executable by SHA-256; keep a separate record of
which bundle/commit, mic and destination each filename represents. No build or
hardware equivalence is inferred from the baseline filename.

The JSON contains only fixed labels, numeric measurements and the executable
hash. Raw stdout is discarded; stderr is read in bounded chunks and only numeric
`pipeline_timing` data is retained. No source log lines, transcripts, vocabulary,
audio, command lines, environment, or paths are copied to the report. Files are
created exclusively with mode 0600; existing evidence is never overwritten.

Measurements and interpretation:

- Only version 1 `outcome=inserted` with finite, nonnegative
  `stages_ms.release_to_inserted` contributes to median and nearest-rank p95.
  Failed/cancelled/offline outcomes and malformed timing rows received during
  dictation or the pause before idle sampling are counted and fail the run. Late
  successful insertions during that pause also contribute to the latency sample.
  Too few insertions are incomplete. Every accepted numeric sample is retained;
  there is no outlier removal.
- `top` samples app and direct helper together; the first non-interval sample is
  discarded. MEM is rounded process footprint in MiB, **not RSS**. CPU and POWER
  are interval samples; POWER is an indicator, **not joules**. Missing samples,
  a changed helper, or pipeline activity during idle invalidates that interval.
- Optional `powermetrics` captures CPU/GPU/ANE estimated mean mW over the idle
  window. These are **system-wide rail estimates**, not app-attributed energy or
  battery drain. Permission failures, missing samplers or incomplete samples are
  reported as unavailable, never as zero. No raw sampler output is saved.
- PR #4's before proxy 58.63/97.96 ms is retained as context only. It cannot gate
  live release-to-inserted. Its installed 0.3.0 174 MiB / 0% CPU / 0 POWER reference
  gets clearly labeled reference-only pass/fail lines, not a matched-build claim.
  PR #4 did not establish a live latency or privileged energy baseline.
- With a supplied matched live report, strict no-increase checks compare median,
  p95, mean total MEM/CPU/POWER, and system rail means. There is no tolerance or
  statistical significance claim. A PASS is conditional on the operator's matched
  conditions and visual checks. Without comparable evidence, status is INCOMPLETE.

Exit codes: 0 PASS, 1 FAIL, 2 INCOMPLETE/cancelled/collection error. `--idle-seconds`
(minimum 5) and `--minimum-samples` (default 20) are explicit collection controls.
Missing helper or early process exit may prevent a report; re-run after fixing the
setup. This tool does not establish microphone/AX behavior through offline tests.

```sh
python3 -m unittest discover -s tools/release-check -v
```
