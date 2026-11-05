cask "greta-flow" do
  version "0.4.0"
  sha256 "2a4261e9666e64e9a7d78ce95b0957779382f5b5fcaef2f66656e92c12966cfd"

  url "https://github.com/matthewkope/greta-flow/releases/download/v#{version}/Greta.Flow_#{version}_aarch64.dmg"
  name "Greta Flow"
  desc "Push-to-talk voice dictation with Parakeet, fully on-device"
  homepage "https://github.com/matthewkope/greta-flow"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Greta Flow.app"

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
