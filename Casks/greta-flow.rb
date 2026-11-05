cask "greta-flow" do
  version "0.3.0"
  sha256 "387fae7e5041d386c66015245725e1f3ee3f5e82ffb152c5843e507f695fcabf"

  url "https://github.com/matthewkope/greta-flow/releases/download/v#{version}/Gretchen.Flow_#{version}_aarch64.dmg"
  name "Greta Flow"
  desc "Greta Flow: push-to-talk voice dictation with Parakeet, fully on-device"
  homepage "https://github.com/matthewkope/greta-flow"

  depends_on macos: :sonoma
  depends_on arch: :arm64

  # The published 0.3.0 archive keeps its original name and checksum.
  # Homebrew removes the old cask's app artifact during migration/reinstall.
  app "Gretchen Flow.app", target: "Greta Flow.app"

  caveats <<~EOS
    Greta Flow ships without a speech model. On first launch a setup window
    guides you to download the recommended Parakeet v2 model (~464 MB).

    Grant Microphone, Accessibility, and Input Monitoring permissions when prompted
    (System Settings > Privacy & Security).

    This build is not yet notarized. If macOS refuses to open it, run:
      xattr -dr com.apple.quarantine "/Applications/Greta Flow.app"
  EOS
end
