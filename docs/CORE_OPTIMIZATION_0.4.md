# Workspace 4 — core optimization

PR #4, based on `cfc2628` after the Greta rename and model decision. Parakeet v2
remains the default; no challenger or streaming-default change is included.

## Result and validation boundary

The offline comparisons do not regress median or p95 latency. **The plan's live
release-to-inserted completion gate is still unverified:** these tests do not
capture microphone shutdown, Bluetooth behavior, or Accessibility insertion into
another app. This report does not substitute an in-memory write for successful
real-app insertion. The installed app and its permissions were not changed.

| Measurement (ms) | Before median | After median | Before p95 | After p95 |
|---|---:|---:|---:|---:|
| Full corpus, preparation + batch recognition/formatting | 59.77 | 58.20 | 104.31 | 95.40 |
| Offline release-path proxy, both runs pooled | 58.63 | 58.16 | 97.96 | 97.27 |
| Live release-to-inserted | Not measured | Not measured | Not measured | Not measured |

The full corpus has 120 public recordings / 40 speakers, each with clean, quiet,
and noisy variants: 720 warm batch calls per side. All 1,441 paired transcripts
(first call plus warm batch and segmented results) match exactly. The proxy uses
a deterministic 120-variant subset (40 recordings / 37 speakers), three passes
in each of two processes per side: 720 warm calls per side. All 722 paired proxy
transcripts match. Repetitions and condition variants are not independent speakers.
Small differences should not be treated as a universal speedup on a busy desktop.

Before source edits, the original release `review_bench` was built and the full
baseline captured with the original lockfile. The later proxy baseline imports
that same commit's production modules and uses the common final dependencies to
isolate source behavior. Both sides use the same helper binary and v2 weights.
The proxy starts its clock with preloaded PCM, then calls production resampling,
gating/gain, recovery retention, inference, formatting and `Sessions::deliver`.
It uses a `String` sink with no AX target. Its timing key is explicitly
`offline_release_to_sink`, not `release_to_inserted`. Existing `asr_stage` and
`pipeline_timing` logs are retained alongside the results.

Machine: Apple M4 Pro, 24 GiB, macOS 15.7.4. System Core ML caches were already
present; this is not pristine first-install timing. Desktop activity was not
isolated, and no own compilation ran during the measured corpus iterations.
See [raw evidence and reproduction](../tools/core-bench/README.md) and the
[validated summary](../tools/core-bench/results/2026-11-04/summary.json).

## Code changes

`main.rs` now contains application state and bootstrap wiring. Ownership is split
into `tray.rs`, `hotkey.rs`, `commands.rs`, `setup.rs`, and `dictation.rs`. Tauri
command names, window builders, menu ordering, labels and icon data are preserved.
A literal comparison found no removed UI strings; the only existing string
changed is the startup error log, which now includes the error instead of panicking.
No `desktop/ui/`, icon, sprite or icon-generation files were changed.

All application runtime `unwrap()` / `expect()` calls have been removed, with
Clippy denying new ones outside tests. A poison-recovery helper clears poison
only after repair while holding the lock:

- Session repair cancels uncertain delivery, preserving the monotonically
  increasing ID so a stale worker cannot insert into a new recording.
- Capture repair marks interrupted audio invalid; it cannot insert a partial
  recording. A fresh recording resets the buffer normally.
- Helper repair drops/kills the uncertain process and starts a new one.
- Config snapshots, complete engine slots and shortcut tables recover their
  last owned values. Native binding updates retain their existing rollback logic.
- Model-download serialization can recover because every reused artifact is
  still hash-verified and partial files are not accepted as complete models.

Invariant-dependent Option/iterator/path unwraps now use checked alternatives.
Startup build failure logs and exits unsuccessfully without a panic. Tests cover
session-ID safety, capture invalidation, lock reuse, protocol preservation and
helper recovery, in addition to the existing delivery and shortcut rollback tests.

## Helper startup and memory

Existing launch-time loading and the persistent helper are retained. FluidAudio
`loadModels` installs model references but does not execute inference, so the
backend now performs one second of silence inference before declaring readiness.
Its output is discarded and the Swift helper creates fresh decoder state for
every request. This also happens after a restart. It does not download anything.
One retained final run records 67.07 ms warm-up and 279.69 ms total model load;
these are cached-machine observations, not a first-install promise or a guarantee
that every later input shape has identical latency.

The existing five-second recovery-expiry thread also checks the idle helper,
without waiting behind active inference or model swaps. A dead helper is restarted
and warmed there; a crash found immediately before a request is handled as well.
An in-flight failure returns an error and drops the process, without automatically
retrying uncertain recognition. Startup/transcription timeouts remain 120/60 seconds.
Recovery can still add delay after a crash or while repeatedly failing to load.

Tests use real child processes for protocol failure, crash and poison recovery.
A separate real-v2 test kills only the direct child of the offline harness, then
verifies that the next request restarts, warms and returns the expected transcript.
See `restart-action.json`, `restart.log` and `restart.jsonl` in the evidence folder.

On little-endian Apple Silicon, IPC now borrows the f32 byte slice with `bytemuck`
instead of allocating a second full PCM byte vector. This removes up to 38.4 MB
of transient byte-conversion storage at the existing ten-minute limit. The required
owned request sample copy remains. `bytemuck` was already in the dependency graph;
its version is unchanged and no new package is introduced for this optimization.
The big-endian path uses a bounded conversion buffer and preserves the wire format.

Resource samples use macOS `top` MEM (rounded process footprint) and a ten-second
idle interval, except the installed-app sample's five-second interval:

- Existing installed 0.3.0 app: 106M app + 68M helper, approximately 174M total;
  both sampled at 0.0% CPU and 0.0 POWER. This is an installed-app reference,
  **not** a before/after measurement of a newly installed PR build.
- Original offline parent after load: 2576K parent + 16M helper; both 0.0% CPU
  and 0.0 POWER. This is a benchmark process, not the GUI application.
- Final warmed offline parent: 2912K parent + 37M helper, approximately 40M
  total; both 0.0% CPU and 0.0 POWER (`final-idle.txt`).
  Warm-up intentionally pays the first inference's allocation and CPU work before
  readiness. Resident measurements depend on macOS compression and other processes.

`top` POWER is a CPU/wakeup indicator, not energy in joules. Privileged
`powermetrics` could not run (`sudo -n`: a password is required). Battery/energy
under sustained real use and before/after total GUI-app footprint remain field
checks; no energy improvement is claimed.

## Dependencies and Python packaging

The live registry check found newer whisper-rs (0.16.0 versus locked 0.14.4), cpal,
enigo, ureq, Tauri and shortcut-plugin releases. They were not upgraded just for
version freshness: no measured benefit for this default Parakeet pipeline or
applicable audited fix required those API changes. Versions and registry URLs
are retained in `dependencies.json`.

The RustSec audit did identify actionable transitive fixes. Compatible updates:

| Dependency | Before | After | Reason |
|---|---|---|---|
| plist / quick-xml | 1.9.0 / 0.39.4 | 1.10.3 / 0.42.0 | RUSTSEC-2026-0194, -0195 |
| rustls | 0.23.40 | 0.23.46 | RUSTSEC-2026-0285, -0346, -0348, -0349 |
| rustls-webpki | 0.103.13 | 0.103.16 | RUSTSEC-2026-0382 |
| webpki-roots | 1.0.7 | 1.0.11 | RUSTSEC-2026-0381 |
| anyhow | 1.0.102 | 1.0.104 | RUSTSEC-2026-0190 |
| memmap2 | 0.9.10 | 0.9.12 | RUSTSEC-2026-0186 |
| tokio | 1.52.3 | 1.54.1 | RUSTSEC-2026-0386 |

Sources: the [RustSec advisory database](https://rustsec.org/advisories/),
`audit-before.json` and `audit.json`. Eight vulnerability entries are resolved,
along with three unsoundness warnings. The final audit has zero reported
vulnerabilities, six upstream unmaintained-package warnings, and one glib
unsoundness warning. The glib dependency is absent from the supported
`aarch64-apple-darwin` graph; it belongs to Tauri's Linux GTK stack. This is not a
claim that auditing proves the absence of every security issue.

The plan's missing `python/LICENSE` was already repaired in base commit `9cbe191`.
`uv build` succeeds and both the wheel and sdist contain the license. CI now
explicitly builds both distributions, so an editable-only install cannot mask a
packaging regression. The prototype remains in CI; its behavior is unchanged.

## Checks and remaining field work

- Locked all-target Rust tests: 69 application, 36 ASR example, 61 core example,
  and 51 review example tests pass (shared-module tests repeat across targets).
- Locked all-target Clippy with `-D warnings`, `cargo fmt --check`, and the release
  application build pass.
- Python prototype: 17 tests, Ruff, wheel and sdist build pass.
- Measurement summary: two tests reject incomplete/duplicate runs and invalid
  timings; all final evidence validates with zero transcript differences.
- No bundle ID, certificate, signing directory, installed app, user settings,
  model default, streaming default or frontend assets were changed.

Before treating the plan's full completion gate as satisfied, run matched live
hold/release dictations on both builds into a disposable supported native field,
verify the inserted text, and summarize actual `pipeline_timing` rows with
`outcome=inserted` and `release_to_inserted`. Include normal mic and Bluetooth
runs, plus an app+helper memory/power interval after warm-up. Offline evidence
above must not be relabeled as those measurements. PR readiness is separate from
this unverified field gate; no merge is authorized in this workspace.
