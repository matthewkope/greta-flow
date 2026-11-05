# Greta Flow rename — Workspace 2

## Scope and identity

The production Tauri product name is `Greta Flow`, producing `Greta Flow.app`
and the local file `Greta Flow_<version>_aarch64.dmg` (with a space).
Release uploads select it using `Greta*Flow_*.dmg`. GitHub sanitizes the uploaded
asset name to `Greta.Flow_<version>_aarch64.dmg` (with a dot); download URLs use
that published name, not the local build filename.
The cask token is `greta-flow`, with `cask_renames.json` mapping the old token.

Keep all of these unchanged:

- Bundle ID: `com.matthewkope.gretchenflow`.
- Signing identity: `Gretchen Flow Signing` (existing certificate).
- Config: `~/.config/gretchen-flow/config.json`.
- Model cache: `~/.cache/gretchen-flow/models/`.
- History: `~/Library/Application Support/gretchen-flow/history.jsonl`.
- Rust executable/package, Python module/package, Swift helper identifiers,
  benchmark fixtures and historical measurements, and the separate test app.

Window/menu labels, privacy prompt wording, sprites, and icons remain unchanged
for Claude in Workspace 5. The default vocabulary remains unchanged as well.
Historical documentation keeps the original paths, labels, and recorded names.

## GitHub and tap rollout

On 2026-11-04 (America/New_York), renamed the repositories to
`matthewkope/greta-flow` and `matthewkope/homebrew-greta-flow`.
Both old repository URLs return HTTP 301 to the new names. Following the old
0.3.0 DMG download URL returns HTTP 200. Updated the shared application origin
(covering all its worktrees) and the existing local Homebrew tap's origin.
Local checkout directory names intentionally remain unchanged.

PR #2 contains the cask and rename metadata. After review/merge, publish
`Casks/greta-flow.rb` and `cask_renames.json` together to the renamed tap and
remove its old `Casks/gretchen-flow.rb`. The tap repository name is already
changed; its cask contents are not published ahead of this PR's review.
Do not remove the old-name GitHub redirects or create repositories at those names.

Homebrew's documented [cask rename mechanism](https://docs.brew.sh/Rename-A-Formula)
requires an old-to-new token mapping in the same tap. Existing installations can
keep `matthewkope/gretchen-flow` as their local tap name; its remote redirects.
Fresh installations use `matthewkope/greta-flow/greta-flow`. Avoid adding both tap
names on an existing installation during the transition.

The current cask still downloads the immutable 0.3.0 asset with its original
filename and SHA-256. Its app artifact uses `target: "Greta Flow.app"` to install
the original signed bundle under the new path without modifying its contents.
At the next release, Workspace 6 must update version/hash, the DMG filename,
and the source app artifact to `Greta Flow.app` together in both repositories.
Never point version 0.3.0 at a nonexistent renamed DMG.

`brew migrate` updates package records but does not rename the installed app.
A same-version rename therefore requires `brew reinstall` as documented in the
README. Homebrew uses the saved old cask artifact to uninstall the previous app;
this avoids leaving both bundle paths. The development build script also removes
the legacy path only after copying the new bundle successfully.

## Real upgrade acceptance — pending

The installed production bundle reports version 0.3.0. It has not been replaced
as part of offline validation. Matching bundle ID/certificate is necessary but
is not proof that all three macOS permissions survive a path change.

On the real 0.3.0 installation:

1. Confirm working microphone capture, the configured hotkey (including Fn if
   used), and insertion into a disposable native text document before upgrading.
2. Record config and cached-model hashes locally without exposing transcript or
   configuration contents. Record the old bundle's designated signing requirement.
3. Quit the app. After publishing the reviewed tap contents, run the README's
   one-time reinstall command. Verify only `Greta Flow.app` remains, and compare
   its signature/requirement with the old bundle. Do not reset TCC or re-grant
   permissions to make this check pass.
4. Launch and dictate into the same document. Confirm Microphone, Accessibility,
   and Input Monitoring still work without new prompts; verify settings/history
   and the existing model are reused with no download.
5. Repeat with the next signed release containing the changed product name;
   0.3.0 installed under a new path alone does not validate a rebuilt 0.4 bundle.

Workspace 2's hardware acceptance gate remains open until these observations are
recorded. No permission-preservation or real-app behavior claim is made here.

## Offline validation

- Built the Parakeet helper from the unchanged pinned FluidAudio revision; a
  fresh worktree needs this staging resource before Rust validation.
- `cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --check`: passed.
- `cargo clippy --locked --all-targets --manifest-path desktop/src-tauri/Cargo.toml -- -D warnings`: passed.
- `cargo test --locked --all-targets --manifest-path desktop/src-tauri/Cargo.toml`:
  63 application tests and 36/47 example tests passed.
- Ruby syntax and Homebrew's content loader accepted the cask, version 0.3.0,
  and `Gretchen Flow.app -> Greta Flow.app` artifact mapping. This was a DSL
  load check, not an installation, migration, uninstall, or full cask audit.
- Shell syntax and `git diff --check` passed. Compared Tauri config structurally:
  only `productName` changed. No frontend, icons, runtime defaults, data-path
  code, or model dependency sources changed.
