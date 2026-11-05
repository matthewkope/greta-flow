# Distribution validation

CI runs the control-flow tests, builds an ad-hoc-signed Apple Silicon Tauri DMG,
then runs `check_built_dmg.py --dmg <path>` against that artifact. No signing
certificate or release credentials are needed.

The unit tests simulate platform detection, including Intel rejection, Rosetta's
Apple Silicon fallback, macOS 13 rejection, and macOS 14/15/26 acceptance. These
are branch tests, not runs on those operating systems or processors.

The built-DMG check prepares the release installer/checksum/cask, verifies the
checksum, mounts the actual DMG, verifies code signatures, bundle ID, version,
minimum macOS version and arm64 executable, and installs into a temporary app
directory. A deliberately wrong checksum must leave the installed copy intact.
Re-running must remove a stale marker, clear an injected quarantine attribute,
and leave a valid signed bundle. Only download transport, process detection and
`open` are stubbed; `/Applications` is redirected to the temporary directory.

The generated cask receives Ruby syntax, Homebrew style and offline audit checks
in a disposable tap. Install/reinstall uses the same DMG via a local file URL and
a unique test cask token, with a temporary `--appdir`. This exercises Homebrew's
checksum, app artifact and postflight steps without touching the real app or its
receipt. Reinstall is a same-version replacement test, not a cross-version
`brew upgrade` or legacy tap migration. Audit does not use `--online` because the
candidate release has not been published. No audit exceptions are suppressed.

## What CI cannot prove

This does **not** prove Gatekeeper will allow first launch on a clean Mac. The
runner is not a clean user account, the test artifact is ad-hoc signed rather
than release-certificate signed, downloads are local, and the app is never
launched. Signature verification and removal of a synthetic quarantine attribute
are not Gatekeeper approval, notarization, or evidence of browser-download
behavior. Neither HTTPS release availability nor the installer sudo path is
covered by this harness.

Before claiming clean installation works, manually test the published release
through the shell installer, Homebrew and browser/DMG routes on clean macOS 14,
15 and current-release accounts. Record Gatekeeper prompts and first launch.
Test an actual prior-version upgrade and verify Microphone, Accessibility and
Input Monitoring grants, settings, history and model cache survive. CI here
makes no claims about those grants, audio hardware, dictation or real-app input.
