// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "GretaSummarizer", platforms: [.macOS(.v14)],
    products: [
        .executable(name: "greta-summarizer", targets: ["GretaSummarizer"]),
    ],
    dependencies: [
        .package(url: "https://github.com/ml-explore/mlx-swift-lm",
                 revision: "ca661dc6885f8603e911746838b60d312439a368"),
        // Tokenizer for local model folders (tokenizer.json); mlx-swift-lm 3.x leaves
        // tokenizer loading to an integration package. Release 1.4.0.
        .package(url: "https://github.com/huggingface/swift-transformers",
                 revision: "402a8a1105f52038b0949d9596981fc9e32bed17"),
    ],
    targets: [
        .executableTarget(name: "GretaSummarizer", dependencies: [
            .product(name: "MLXLLM", package: "mlx-swift-lm"),
            .product(name: "MLXLMCommon", package: "mlx-swift-lm"),
            .product(name: "Tokenizers", package: "swift-transformers"),
        ]),
    ]
)
