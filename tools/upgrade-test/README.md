# Real 0.3.0 → 0.4.0 upgrade check

This is a read-only evidence collector, not an installer. Requires macOS and
Python 3.9+. It never signs code, reads signing credentials, changes permissions,
launches the app, or deletes data. Do not access `~/gretchen-flow-signing`.
Unit tests use temporary fixtures; they do **not** establish real upgrade success.

## Run on Matt's Mac

1. Use a genuine certificate-signed **0.3.0** release installed through the route
   being tested. Back up the app and user data separately before upgrading. Record
   macOS version, architecture, installation route, and release artifact/checksum.
   In 0.3.0, verify working dictation, all three permission grants, a distinctive
   setting, downloaded models, and a non-sensitive persisted history entry. Quit
   both app versions and their helpers before taking snapshots. Do not clear
   history, change settings, or download models between snapshots.
2. Create a private evidence directory outside the repository:

   ```sh
   evidence="$HOME/greta-upgrade-evidence-$(date +%Y%m%d-%H%M%S)"
   mkdir -m 700 "$evidence"
   python3 tools/upgrade-test/upgrade_test.py before --output "$evidence/before.json"
   ```

   The defaults scan `/Applications` and `~/Applications`, including subfolders,
   for both app names and any other `.app` with the production bundle ID. Mounted
   DMGs, Downloads, backups, and other users' folders are not scanned. Unmount
   release DMGs and inspect other install locations manually. To include another
   installation root, repeat `--app-root` for **all** roots (this replaces defaults).
   Symlinks encountered in scanned directories require manual review; they are
   never followed. Add `--extra-model /absolute/path` for each custom model outside
   the default cache. Missing config, empty model trees, and missing/empty history
   fail rather than producing a vacuous pass. Use real existing data, not fixtures.
3. Upgrade through the route under test using the approved **0.4.0 release**:
   Homebrew's renamed cask upgrade, re-running the release `install.sh`, or the
   documented manual DMG upgrade. Record the exact command/action and outcome.
   Do not manually remove a duplicate left by an automatic upgrade: capture it as
   a failure. If the route opens the app, quit it after launch has settled. Before
   new dictation, history trimming, or settings edits, run:

   ```sh
   python3 tools/upgrade-test/upgrade_test.py after \
     --baseline "$evidence/before.json" --output "$evidence/after.json"
   ```

   A pass requires one `Greta Flow.app` at version 0.4.0 in the scanned roots,
   valid signatures, unchanged bundle ID `com.matthewkope.gretchenflow`, identical
   designated signing requirement, and satisfaction of the old requirement by the
   new code. Original config, history, and every cached/custom model file must
   retain their paths and SHA-256 hashes. New model files are permitted. A changed
   history/config file is a failure to investigate, even if caused by legitimate
   retention or normalization; do not silently replace the baseline. SHA-256
   equality proves contents survived, not that a downloader never rewrote them.
   Evidence includes paths/hashes and public signing requirements, not file contents;
   keep it local because paths can still be private. Output files cannot overwrite
   existing evidence. Errors exit nonzero; an early error may leave no after file.
4. Launch the upgraded app and perform the manual checks below. If launch was not
   part of step 3, quit and repeat `after` with a new output filename before making
   new dictation entries, to check persistence after first launch too.

## Matt must confirm by hand

- [ ] **Microphone:** the existing grant remains enabled in System Settings →
  Privacy & Security → Microphone; no new consent prompt. Real microphone audio
  reaches the app and a short dictated phrase is recognized.
- [ ] **Accessibility:** the existing grant remains enabled; no re-grant/removal
  and re-add required. Dictate into a disposable native text document and verify
  insertion at the intended caret without duplication.
- [ ] **Input Monitoring:** the existing grant remains enabled; no new consent
  prompt. The configured global hold/toggle shortcut starts and stops dictation
  while another app has focus. Check Fn/Globe if that is the configured shortcut.
- [ ] Quit/relaunch and repeat shortcut, audio, and insertion checks. Record any
  permission prompt, relaunch requirement, disabled/stale/duplicate settings entry,
  or need to toggle a grant as a regression; do not reset TCC to make it pass.
- [ ] The distinctive setting and old Recent history entry are visible. With the
  network disconnected, the previously downloaded model loads and dictates without
  a download. Check each model used in the baseline, including custom paths.
- [ ] Only the intended app is installed in all actual installation locations;
  record any copies outside the scanner's roots separately.

Repeat with a fresh genuine 0.3.0 baseline per installation route and macOS version
being claimed. Record each checkbox as PASS/FAIL/NOT RUN with observed details,
including model, shortcut, microphone, target app, and whether grants were touched.
An automated PASS alone is **not** permission or hardware validation. Do not publish
transcripts, private config, or model files in the PR.

## Tool tests

```sh
python3 -m unittest discover -s tools/upgrade-test -p 'test_*.py' -v
```
