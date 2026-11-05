#!/usr/bin/env bash
#
# Local dev build that keeps macOS permissions (Accessibility, Microphone,
# Input Monitoring) across rebuilds.
#
# Release and development bundles must use the existing stable signing identity.
# Keep the bundle ID and certificate unchanged when renaming the app; permission
# preservation across the path change still requires a real upgrade check.
# Never replace a missing certificate with a newly created one.
#
# Usage:  desktop/scripts/dev-build.sh        # build, sign, install to /Applications, launch
#         desktop/scripts/dev-build.sh --no-install   # build + sign only
set -euo pipefail

# Must match the identity CI signs releases with (release.yml's SIGNING_IDENTITY
# secret) so local builds and Homebrew installs share one code identity — then a
# single permission grant covers both. Backup of this cert: ~/gretchen-flow-signing.
CERT="Gretchen Flow Signing"
APP_NAME="Greta Flow.app"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"   # desktop/
TAURI_DIR="$HERE/src-tauri"
BUNDLE="$TAURI_DIR/target/debug/bundle/macos/$APP_NAME"
ENTITLEMENTS="$TAURI_DIR/entitlements.plist"
INSTALLED="/Applications/$APP_NAME"

# NOTE: no -v — a self-signed cert is "not trusted", so `-v` (valid only) hides
# it even though codesign signs with it fine.
if ! security find-identity -p codesigning | grep -q "$CERT"; then
  echo "error: code-signing certificate \"$CERT\" not found in the keychain." >&2
  echo "Restore access to the existing signing identity; do not create a replacement." >&2
  exit 1
fi

echo "==> building (cargo tauri build --debug)"
( cd "$TAURI_DIR" && cargo fmt && cargo tauri build --debug )

echo "==> signing $APP_NAME with \"$CERT\" (stable identity)"
codesign --force --deep --options runtime \
  --entitlements "$ENTITLEMENTS" \
  --sign "$CERT" "$BUNDLE"

echo "==> designated requirement:"
codesign -d -r- "$BUNDLE" 2>&1 | sed -n 's/^designated => /    /p'

if [[ "${1:-}" == "--no-install" ]]; then
  echo "==> done (not installed). Bundle: $BUNDLE"
  exit 0
fi

echo "==> installing to $INSTALLED"
osascript -e 'tell application id "com.matthewkope.gretchenflow" to quit' 2>/dev/null || true
sleep 1; pkill -f "$APP_NAME" 2>/dev/null || true; sleep 1
rm -rf "$INSTALLED"
cp -R "$BUNDLE" /Applications/
# Remove the legacy bundle only after the replacement has copied successfully.
rm -rf "/Applications/Gretchen Flow.app"
xattr -dr com.apple.quarantine "$INSTALLED" 2>/dev/null || true

echo "==> launching"
open "$INSTALLED"
echo "==> done. Verify permissions after the app-path rename."
