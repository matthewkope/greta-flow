// swift-tools-version: 6.0
import PackageDescription

// System-audio capture helper. Apple frameworks only; the Core Audio process-tap
// APIs need macOS 14.2 and are gated at runtime.
let package = Package(
    name: "GretaSystemAudio", platforms: [.macOS(.v14)],
    products: [.executable(name: "greta-system-audio", targets: ["GretaSystemAudio"])],
    targets: [
        .executableTarget(name: "GretaSystemAudio", swiftSettings: [.swiftLanguageMode(.v5)]),
    ]
)
