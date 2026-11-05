# Greta Flow 0.4 plan

Handoff for GPT-6 Astra (Codex CLI in cmux). Written 2026-11-04 against branch
`codex/gretchen-five-card-improvements` (base `f6163668`).

**Goals:** rename Gretchen Flow to Greta Flow, make the code faster and easier to
maintain, adopt a newer speech model only if it wins on measurements, and make
free, open-source installation as easy as possible.

## Ground rules

- GitHub account: `matthewkope`. Run `gh auth status` before any push or PR.
- No AI attribution footers or co-author trailers in commits or PRs.
- Never read, print, or copy `~/gretchen-flow-signing/`. Never replace the signing
  certificate or change the bundle ID. Either one makes users re-grant permissions.
- Run `cargo fmt --check`, Clippy with `-D warnings`, and `cargo test --locked
  --all-targets` before each PR. The commands are in `docs/FIVE_CARD_REVIEW.md`.
- Don't claim hardware or real-app behaviour that was only tested offline.
- Don't change defaults inside a refactor.
- **Astra owns the back end; Claude owns the front end.** Astra does all back-end
  coding, including optimization, models, release, and distribution. Astra must not
  edit `desktop/ui/`, the `icons/` folder, `desktop/scripts/gen_icons.py`, sprites, or
  the layout and wording of windows and menus, including tray menu labels in Rust.
  If a job needs a front-end change, it stops and leaves a note in its PR.
- `CLAUDE.md` is stale: it describes ad-hoc signing and asks for a Co-Authored-By
  trailer. Follow this plan instead, and refresh that file in Workspace 1.

## Decisions

Settled:

- No notarization and no graphical installer. Stay free and open source.
- Homebrew stays, alongside a free `install.sh`.
- No in-app updater. `brew upgrade` or re-running `install.sh` handles updates, so
  the app's no-network rule is unchanged.
- English only. Multilingual support is not a goal.
- The mascot becomes Greta.
- Approved: rename the `gretchen-flow` repo and the Homebrew tap to `greta-flow`.

## Handoff: cmux workspaces

Six workspaces, one job each, in three waves. Start a wave only when the previous
one has merged. Don't start anything until Matt says to hand off.

| # | Title | Job | Effort | Wave |
|---|---|---|---|---|
| 1 | `GF 🧱 · PR #NN` | Land the existing branch | medium | 1 |
| 2 | `GF 🏷️ · PR #NN` | Greta Flow rename | medium | 2 |
| 3 | `GF 🧪 · PR #NN` | FluidAudio upgrade and model benchmark | high | 2 |
| 4 | `GF ⚡ · PR #NN` | Core code optimization | high | 3 |
| 5 | `GF 🎨 · PR #NN` | All design, run in Claude: sprite, menus, setup window | medium | 3 |
| 6 | `GF 📦 · PR #NN` | Cask, `install.sh`, release automation, README | medium | 3 |

Title format is `GF <emoji> · PR #<number>`, using the real PR number.

**Why waves:** Workspace 1's uncommitted code is the base for everything. The
rename touches most files, so it merges before the refactor. Workspace 3 stays
mostly in `desktop/parakeet` and `tools/asr-bench`, so it can run beside it.

**Launch steps for each workspace:**

1. Create a git worktree and branch from updated `main`. Workspace 1 instead uses
   the existing checkout and branch, because the uncommitted work lives there.
2. Push the branch and open a draft PR, which supplies the number for the title.
   Use an empty commit if the branch has no changes yet.
3. Create the workspace, setting the effort explicitly because
   `~/.codex/config.toml` defaults to low:

```sh
cmux new-workspace --name "GF 🧪 · PR #<number>" --cwd "<worktree>" \
  --command 'codex -m gpt-6-astra -c model_reasoning_effort="high"'
```

   Workspace 5 is the exception: launch Claude Code with
   `--command 'claude --effort medium'`.
4. Give the session this file and its section below. It works only on that section.

**When a PR merges:** close its workspace, then remove its worktree. Confirm the
merge first and check the worktree has no uncommitted work; if it does, leave both
open and tell Matt. Closed-without-merging PRs keep their workspace.

```sh
gh pr view <number> --json state --jq .state   # must print MERGED
cmux close-workspace --workspace <id|ref>
git worktree remove "<worktree>"
```

## Workspace 1: land the existing work 🧱

About 930 added lines plus new modules (`session.rs`, `streaming.rs`,
`profile.rs`, `metrics.rs`) are **uncommitted** and have never been pushed.

- Confirm tests pass, commit in logical chunks, push, and take the PR out of draft.
- Refresh `CLAUDE.md`.
- List the checks from "Remaining field validation" in
  `tools/asr-bench/REVIEW_BENCHMARK.md` for Matt. They need real hardware: mics,
  Bluetooth, insertion in real apps, first-install Core ML time.
- Keep `streaming: false` until those checks pass.

**Done when:** merged, with the field checks listed for Matt.

## Workspace 2: Greta Flow rename 🏷️

About 42 tracked files mention Gretchen.

**Change:** app display name (`productName`), so the app becomes
`Greta Flow.app`; README and docs; cask name and description; DMG name; release
workflow; the GitHub repo and Homebrew tap names. Window and menu text, the sprite,
and the mascot's name in the UI are left for Workspace 5.

**Do not change:**

- Bundle ID `com.matthewkope.gretchenflow`. It is part of the code identity, so
  changing it resets Microphone, Accessibility, and Input Monitoring grants.
- Certificate and its name, "Gretchen Flow Signing". Users never see it.
- `~/.config/gretchen-flow`, `~/.cache/gretchen-flow`, and the history folder,
  unless a tested migration moves them. A move must not force a 464 MB re-download.

**Handle:**

- Upgrading must not leave both `Gretchen Flow.app` and `Greta Flow.app` installed.
  Add a cask rename entry so `gretchen-flow` users move to `greta-flow`.
- Confirm permissions survive the app path change on a real upgrade from 0.3.0.
- After the repo rename, confirm the old GitHub URLs redirect, old release download
  links still work, and local remotes point at the new name.

**Done when:** a real 0.3.0 upgrade keeps permissions, settings, and the model.

## Workspace 3: models 🧪

Current default: Parakeet TDT 0.6B v2 (Core ML through FluidAudio), 2.196% WER
on the clean set, about 61 ms median warm call on an M4 Pro.

First upgrade FluidAudio from the pinned `b68f484` (Sept 14) to v0.18.2, 132
commits ahead, and re-run the benchmark. The upgrade alone could change results.

| Priority | Candidate | Why test it | Watch for |
|---|---|---|---|
| 1 | Nemotron Unified ASR 0.6B | New in FluidAudio v0.18.0; offline and streaming in one model | Confirm English support, license, size |
| 1 | Parakeet TDT 0.6B v3 | Drop-in; shortlisted in September but never benchmarked | Its advantage is multilingual, so it must beat v2 on English alone |
| 2 | Nemotron 3.5 Transcribe Lite | New in v0.18.0, word timings | Built for long audio; punctuation unknown |
| 2 | Parakeet EOU 120M | Small streaming model with end-of-utterance detection | Likely accuracy loss |
| 3 | Bark 3.0, Canary 1B v2 | Available through FluidAudio | Check license and speed |
| 3 | Qwen3-ASR 1.7B, Granite Speech 4.1 2B, Cohere Transcribe | Reported near the top of the Open ASR Leaderboard | Larger, no confirmed Core ML path here; latency risk. Verify on the leaderboard first |

Do not plan around "Parakeet Ultra": it appears only on an aggregator page and
was not confirmed in official sources.

**Also evaluate FluidAudio vocabulary boosting.** Today the Parakeet personal
dictionary only corrects text after recognition. Acoustic boosting could improve
names and jargon. Check which models it requires and whether it adds a download.

**Method:** use `tools/asr-bench` and the 120-clip corpus (clean, quiet, noisy).
Add natural technical dictation with names, numbers, and punctuation, because
audiobook speech doesn't cover those.

**Comparison report (required):** publish `docs/MODEL_COMPARISON_0.4.md` for Matt.
Compare the current setup (Parakeet v2 on pinned FluidAudio) with v2 on v0.18.2 and
each tested candidate. Use the same clips and machine, and show WER for clean,
quiet, and noisy audio; names, numbers, and punctuation; median and p95 latency;
first-load time; download size; and memory. Include sample transcripts side by
side, plus recordings of Matt's own dictation, with any audio kept local.

**Promote a new default only if** it beats v2 on WER with no regression on names,
numbers, or punctuation, stays within an agreed latency budget, has a license
pinned to the exact artifact, and has hashes in the model manifest. If nothing
wins, keep v2. Judge every candidate on English only.

**Parakeet stays.** Parakeet v2 is never removed, even if another model becomes the
default. Add the winning challenger as a second engine and make switching safe:

- Switching needs no reinstall and keeps both downloaded models on disk.
- Switching back to Parakeet works offline and needs no re-download.
- If the selected model fails to load, fall back to Parakeet and log why.
- Expose the backend switch; Workspace 5 designs the menu option for it.

## Workspace 4: code optimization ⚡

Measure first. `pipeline_timing` and `asr_stage` logs already exist; capture a
baseline for release-to-inserted before changing anything.

1. Split `main.rs` (1,743 lines) into tray, hotkey, commands, and setup modules
   with no behaviour change.
2. Audit the `unwrap()`/`expect()` calls (about 60 in `main.rs`, 113 across
   `src/`, some in tests). Runtime paths must not panic; a poisoned lock must not
   end dictation.
3. Review Parakeet warm-up: load the model at launch and keep the helper alive,
   so no dictation pays for start-up. Confirm helper restart after a crash.
4. Measure total app and helper memory, then idle CPU and energy.
5. Check whether `whisper-rs` 0.14 and other dependencies are behind; upgrade
   only where there is a measured or security benefit.
6. Fix the `python/` packaging break (`license = { file = "LICENSE" }` with no
   `python/LICENSE`), or remove the prototype from CI.

**Done when:** tests pass and median and p95 release-to-inserted are no worse
than baseline, with before/after numbers in the PR.

## Workspace 5: design 🎨 (Claude only)

Owns the Greta sprite and tray icons, menus and all window design, the mascot's
name in the UI, and the Parakeet/challenger model switch in the menu. Start after
Workspace 4's `main.rs` split merges if possible, to avoid conflicts.

Rebuild `desktop/ui/setup.html` as resumable steps:

1. Welcome and privacy summary.
2. Model download with progress, retry, and a disk-space check.
3. "Preparing model" progress. First Core ML load took 33 seconds in testing and
   must not look like a hang.
4. Microphone, Accessibility, and Input Monitoring: one at a time, each with a
   deep link to System Settings and live status that updates when granted.
   Handle any permission that needs a relaunch.
5. Mic level check.
6. Practice box: hold the key, dictate, see the text appear.
7. Finish, with a "Run setup again" menu item.

**Done when:** someone new to the app reaches working dictation without the README.

## Workspace 6: free distribution 📦

macOS blocks non-notarized apps downloaded in a browser, and macOS 15+ removed the
Control-click bypass, so each route below works around that.

1. **Homebrew:** add a cask `postflight` step that clears quarantine on install and
   upgrade, so users skip the manual `xattr` command. Confirm Homebrew's current
   rules still allow this in a personal tap.
2. **`install.sh`:** hosted with each release. Checks for Apple Silicon and macOS
   14+, downloads the DMG, verifies its SHA-256, copies the app to `/Applications`,
   and opens it. Re-running it updates the app. Test on a clean Mac: curl downloads
   normally carry no quarantine flag, but this is unverified for this app.
3. **Manual route:** document opening the DMG app, then Privacy & Security >
   Open Anyway.
4. **Release automation:** `release.yml` publishes the DMG and checksum and updates
   the cask version and hash in both the main repo and the tap.
5. **Uninstall:** document removing the app, config, and model cache.
6. **README:** rewrite the install section around these routes.

**Done when:** on clean macOS 14, 15, and current-release test accounts, each route
launches the app, and an update keeps all three permissions.

## Release order

Wave 1, then wave 2, then wave 3, then ship 0.4.0 under the Greta Flow name.
