#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [ "$(uname -m)" != arm64 ]; then
  echo 'The Parakeet app build currently requires Apple Silicon.' >&2
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
if swift build -c release --package-path summarizer; then
  cp summarizer/.build/release/greta-summarizer parakeet/staging/
  for resource in summarizer/.build/release/*.bundle; do
    [ ! -d "$resource" ] || ditto "$resource" "parakeet/staging/$(basename "$resource")"
  done
else
  echo "warning: greta-summarizer did not build (mlx-swift needs Swift 6.3+); summaries will report unsupported" >&2
fi
mkdir -p parakeet/staging/summarizer-licenses
cp summarizer/THIRD_PARTY_NOTICES.md parakeet/staging/summarizer-THIRD_PARTY_NOTICES.md
cp summarizer/licenses/* parakeet/staging/summarizer-licenses/

swift build -c release --package-path system-audio
cp system-audio/.build/release/greta-system-audio parakeet/staging/

# SwiftPM checkouts can supply read-only resources; bundling strips xattrs.
chmod -R u+w parakeet/staging
