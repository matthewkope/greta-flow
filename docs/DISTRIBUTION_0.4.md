# Distribution 0.4 — Workspace 6

The default remains Parakeet v2, with no challenger. No app code, UI, bundle ID,
signing certificate, model selection, or storage paths change here.

## Homebrew policy check (2026-11-04)

The [Homebrew Cask Cookbook](https://docs.brew.sh/Cask-Cookbook#stanza-flight_steps)
documents `postflight_steps` and its `run` operation. Legacy Ruby flight blocks
are temporarily supported for third-party taps but rejected in official taps.
The cask prepared for the personal tap uses the structured hook, targeting `{{appdir}}/Greta Flow.app`
so custom app directories are honored. This is not a submission to the official
Homebrew cask repository. Quarantine removal is disclosed in the README/caveat;
a failure fails installation rather than silently claiming it succeeded.

## Release operation

1. Set the release version in the app's normal version files before tagging.
   The preparation script refuses a tag that differs from tauri.conf.json, a
   prerelease tag, or a differently named/architected DMG.
2. Configure `RELEASE_REPO_TOKEN` as a fine-grained token with Contents: write
   on `matthewkope/greta-flow` and `matthewkope/homebrew-greta-flow`. Both default
   branches are expected to be `main`. Branch protections must allow these
   release commits. The normal GITHUB_TOKEN cannot update the separate tap.
   Existing signing secrets are reused without changing the certificate.
3. Push a stable `vMAJOR.MINOR.PATCH` tag. The workflow builds the signed bundle,
   publishes the DMG, SHA256SUMS, and a version/hash-pinned install.sh, then
   commits the same generated cask and rename map to main and the tap, removing
   the legacy `Casks/gretchen-flow.rb` in the same commit.
4. Check both update commits after the job. A cross-repository update cannot be
   atomic: a failure may leave one cask updated. Fix access/protection problems
   and rerun; identical cask content produces no additional commit. Do not rerun
   an older release after a newer one has shipped. No release is cut by PR #5.

The current cask deliberately retains the published 0.3.0 DMG name and checksum
until release automation has an actual newly built artifact. The source installer
is unusable until preparation pins its values; the README explains availability.
The installer stages a complete copy on the destination volume, keeps the previous
Greta bundle until replacement succeeds, and leaves config/cache/history alone.
It rejects symlink/unrelated destination apps and asks the user to quit first.

## Validation and remaining field checks

Automated checks cover release artifact/hash/cask agreement, template rendering,
and rejection of invalid tags or artifact names. Mocked installer tests cover
successful updates, checksum rejection before mounting, copy failure, rollback
after replacement failure, old macOS rejection, and a running app. These mocks
do not exercise real hdiutil, codesign, quarantine, or sudo. Shell syntax and Homebrew style
are checked separately, along with the required Rust fmt, Clippy, and test suite.
These are not clean-machine or TCC permission tests.

Before shipping, perform this matrix on **clean macOS 14, macOS 15, and the current
release**, using separate test accounts for each route:

- Homebrew install and upgrade: inspect quarantine, launch, download Parakeet v2,
  grant all three permissions, and dictate into a disposable document.
- Release install.sh: first install, rerun/update, launch, and verify no duplicate
  old-name bundle. Check a checksum failure leaves the installed app untouched;
  check a non-writable /Applications path and interrupted copy/replacement.
- Browser DMG: verify checksum, copy, exercise Privacy & Security > Open Anyway,
  launch, and update by replacement.
- For all routes, upgrade a genuine 0.3.0 installation: verify Microphone,
  Accessibility, and Input Monitoring remain granted and functional, settings
  and history survive, and the existing model is reused without downloading.
- Test Homebrew with a custom --appdir, and test installer under Rosetta.

All clean-account/hardware checks above remain **pending**, including whether
curl downloads lack quarantine on those machines. The installer explicitly
clears quarantine rather than relying on that assumption. No installed app was
replaced during development. These limits also apply to the rename validation
in [RENAME_0.4.md](RENAME_0.4.md).

### Local results (2026-11-04)

- `cargo fmt --check`: passed.
- Locked all-target Clippy with `-D warnings`: passed.
- Locked all-target Rust tests: 63 application, 36 review-benchmark, and 47
  streaming-benchmark tests passed.
- Four distribution test methods (including five failure scenarios): passed.
- Homebrew 7.0.9 cask style, installer shell syntax, workflow YAML parsing, and
  workflow shell syntax: passed.
- Release publishing and two-repository pushes were not executed; they require
  the next release tag and configured release token. Clean-account checks remain
  pending as listed above.

### PR #5 review follow-up

The review found that the live tap still contains only `Casks/gretchen-flow.rb`.
The README now explicitly marks both the renamed Homebrew route (including
migration) and the installer as available from the next release, and directs
0.3.0 users to the manual DMG route. This PR does not publish the tap early.
Release automation removes the stale cask in the same commit that adds the new
cask and rename map; `--ignore-unmatch` makes this safe on reruns and in the main
repository where the old cask is already absent.
