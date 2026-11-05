// swift-tools-version: 6.2
import Foundation
import PackageDescription

let local = ProcessInfo.processInfo.environment["FLUID_AUDIO_PATH"]
let runtime: Package.Dependency = local.map {
    .package(name: "FluidAudio", path: $0, traits: [])
} ?? .package(url: "https://github.com/FluidInference/FluidAudio.git",
              revision: "e396cf780c4abf8e395087467c0585d3b0d17f68", traits: [])
let package = Package(
    name: "GretchenParakeet", platforms: [.macOS(.v14)],
    products: [.executable(name: "gretchen-parakeet", targets: ["GretchenParakeet"])],
    dependencies: [runtime],
    targets: [.executableTarget(name: "GretchenParakeet", dependencies: [.product(name: "FluidAudio", package: "FluidAudio")])]
)
