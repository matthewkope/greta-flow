#!/bin/bash
# Release automation replaces these three values with the exact release artifact.
set -euo pipefail
VERSION='@VERSION@'
DMG='@DMG@'
SHA256='@SHA256@'
fail() { printf 'Greta Flow: %s\n' "$*" >&2; exit 1; }
[[ $(uname -s) == Darwin ]] || fail 'macOS is required.'
[[ $(uname -m) == arm64 || $(sysctl -n hw.optional.arm64 2>/dev/null || true) == 1 ]] || fail 'Apple Silicon is required.'
major=$(sw_vers -productVersion); major=${major%%.*}
[[ $major =~ ^[0-9]+$ && $major -ge 14 ]] || fail 'macOS 14 or later is required.'
[[ $VERSION != '@VERSION@' ]] || fail 'Download install.sh from a GitHub release, not the source tree.'
[[ $EUID -ne 0 ]] || fail 'Run as your normal user; sudo is requested only if /Applications requires it.'
# Avoid replacing a running app/helper. Quit from the menu bar first.
if pgrep -x gretchen-flow >/dev/null || pgrep -x greta-flow >/dev/null; then
  fail 'Quit Greta Flow (or Gretchen Flow), then run this installer again.'
fi
work=$(mktemp -d "${TMPDIR:-/tmp}/greta-flow.XXXXXXXX")
stage=''
mounted=false
installed=false
needs_sudo=false
[[ -w /Applications ]] || needs_sudo=true
privileged() {
  if $needs_sudo; then sudo "$@"; else "$@"; fi
}
cleanup() {
  if $mounted; then hdiutil detach "$work/mount" -quiet || true; fi
  if [[ -n $stage ]]; then
    if ! $installed && privileged test -d "$stage/previous.app"; then
      if [[ -e '/Applications/Greta Flow.app' ]] || ! privileged mv "$stage/previous.app" '/Applications/Greta Flow.app'; then
        printf 'Previous app retained for recovery at %s/previous.app\n' "$stage" >&2
        stage=''
      fi
    fi
    [[ -z $stage ]] || privileged rm -rf "$stage"
  fi
  rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
url="https://github.com/matthewkope/greta-flow/releases/download/v$VERSION/$DMG"
printf 'Downloading Greta Flow %s…\n' "$VERSION"
curl --fail --location --proto '=https' --proto-redir '=https' --retry 3 "$url" -o "$work/$DMG"
actual=$(shasum -a 256 "$work/$DMG"); actual=${actual%% *}
[[ $actual == "$SHA256" ]] || fail 'SHA-256 mismatch; nothing was installed.'
mkdir "$work/mount"
hdiutil attach "$work/$DMG" -mountpoint "$work/mount" -nobrowse -readonly -quiet
mounted=true
source_app="$work/mount/Greta Flow.app"
[[ -d $source_app ]] || fail 'The release does not contain Greta Flow.app.'
/usr/bin/codesign --verify --deep --strict "$source_app"
bundle_id=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$source_app/Contents/Info.plist")
[[ $bundle_id == com.matthewkope.gretchenflow ]] || fail 'Unexpected app identity.'
for existing in '/Applications/Greta Flow.app' '/Applications/Gretchen Flow.app'; do
  if [[ -e $existing || -L $existing ]]; then
    [[ ! -L $existing ]] || fail "Refusing to replace a symlink: $existing"
    existing_id=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$existing/Contents/Info.plist")
    [[ $existing_id == "$bundle_id" ]] || fail "Refusing to replace an unrelated app: $existing"
  fi
done
stage=$(privileged mktemp -d /Applications/.greta-flow.XXXXXXXX)
privileged ditto "$source_app" "$stage/Greta Flow.app"
# Clear only quarantine, preserving the signed bundle and other attributes.
privileged xattr -dr com.apple.quarantine "$stage/Greta Flow.app"
if [[ -d '/Applications/Greta Flow.app' ]]; then
  privileged mv '/Applications/Greta Flow.app' "$stage/previous.app"
fi
privileged mv "$stage/Greta Flow.app" '/Applications/Greta Flow.app'
installed=true
# Only remove the legacy name after the replacement succeeds.
privileged rm -rf '/Applications/Gretchen Flow.app'
open '/Applications/Greta Flow.app'
printf 'Installed Greta Flow %s. Settings, history, and model cache were preserved.\n' "$VERSION"
