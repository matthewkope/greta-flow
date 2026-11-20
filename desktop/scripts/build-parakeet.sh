#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [ "$(uname -m)" != arm64 ]; then
  echo 'The Parakeet app build currently requires Apple Silicon.' >&2
  exit 1
fi

# mlx-swift (summarizer) needs Swift 6.3+. Honor TOOLCHAINS when set; otherwise
# use the per-user Swift 6.4.2 toolchain if installed. Nothing global changes.
if [ -z "${TOOLCHAINS:-}" ]; then
  user_toolchain="$HOME/Library/Developer/Toolchains/swift-6.4.2-RELEASE.xctoolchain"
  if [ -d "$user_toolchain" ]; then
    TOOLCHAINS=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$user_toolchain/Info.plist")
    export TOOLCHAINS
  fi
fi
swift_version=$(swift --version 2>&1 | sed -nE 's/.*Swift version ([0-9]+)\.([0-9]+).*/\1 \2/p' | head -n 1)
read -r swift_major swift_minor <<<"${swift_version:-0 0}"
if [ "$swift_major" -lt 6 ] || { [ "$swift_major" -eq 6 ] && [ "$swift_minor" -lt 3 ]; }; then
  echo "error: Swift 6.3 or newer is required to build greta-summarizer (found: $(swift --version 2>&1 | head -n 1))." >&2
  echo "Install swift-6.4.2-RELEASE to ~/Library/Developer/Toolchains or set TOOLCHAINS." >&2
  exit 1
fi
swift build -c release --package-path parakeet
mkdir -p parakeet/staging
cp parakeet/.build/release/gretchen-parakeet parakeet/staging/
for resource in parakeet/.build/release/*.bundle; do
  [ ! -d "$resource" ] || ditto "$resource" "parakeet/staging/$(basename "$resource")"
done
cp parakeet/THIRD_PARTY_NOTICES.md parakeet/FluidAudio-LICENSE.txt parakeet/staging/

# Meeting helpers, staged beside gretchen-parakeet. Signing happens with the
# app bundle (dev-build.sh / release workflow), as for gretchen-parakeet.
swift build -c release --package-path summarizer
cp summarizer/.build/release/greta-summarizer parakeet/staging/
for resource in summarizer/.build/release/*.bundle; do
  [ ! -d "$resource" ] || ditto "$resource" "parakeet/staging/$(basename "$resource")"
done
mkdir -p parakeet/staging/summarizer-licenses
cp summarizer/THIRD_PARTY_NOTICES.md parakeet/staging/summarizer-THIRD_PARTY_NOTICES.md
cp summarizer/licenses/* parakeet/staging/summarizer-licenses/

swift build -c release --package-path system-audio
cp system-audio/.build/release/greta-system-audio parakeet/staging/

# SwiftPM checkouts can supply read-only resources; bundling strips xattrs.
chmod -R u+w parakeet/staging
