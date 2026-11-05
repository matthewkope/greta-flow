cask "greta-flow" do
  version "0.3.0"
  sha256 "387fae7e5041d386c66015245725e1f3ee3f5e82ffb152c5843e507f695fcabf"

  url "https://github.com/matthewkope/greta-flow/releases/download/v#{version}/Gretchen.Flow_#{version}_aarch64.dmg"
  name "Greta Flow"
  desc "Push-to-talk voice dictation with Parakeet, fully on-device"
  homepage "https://github.com/matthewkope/greta-flow"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  # The published 0.3.0 archive keeps its original name and checksum.
  # Homebrew removes the old cask's app artifact during migration/reinstall.
  app "Gretchen Flow.app", target: "Greta Flow.app"

  # Personal tap: the self-signed release is intentionally not notarized.
  postflight_steps do
    run "/usr/bin/xattr", args: ["-dr", "com.apple.quarantine", "{{appdir}}/Greta Flow.app"]
  end

  caveats <<~EOS
    Greta Flow ships without a speech model. On first launch a setup window
    guides you to download the recommended Parakeet v2 model (~464 MB).

    Grant Microphone, Accessibility, and Input Monitoring permissions when prompted
    (System Settings > Privacy & Security).

    This build is not notarized. This personal tap clears quarantine after
    installation and upgrades; install only if you trust this release.
  EOS
end
