# Greta Flow summarizer runtime

The `greta-summarizer` helper uses the following open-source packages. Full
license texts are in `licenses/`.

## Direct dependencies

- ml-explore/mlx-swift-lm, release 3.33.0, revision
  ca661dc6885f8603e911746838b60d312439a368, MIT License.
  Source: https://github.com/ml-explore/mlx-swift-lm
  License: licenses/mlx-swift-lm-LICENSE.txt
- huggingface/swift-transformers (Tokenizers), release 1.4.0, revision
  402a8a1105f52038b0949d9596981fc9e32bed17, Apache License 2.0.
  Source: https://github.com/huggingface/swift-transformers
  License: licenses/swift-transformers-LICENSE.txt

## Transitive dependencies

Versions and revisions are pinned in `Package.resolved`.

- ml-explore/mlx-swift 0.33.0, revision 7ede4fd6d05f63305c1c0ebf4f8c1909a637c574,
  MIT License. Source: https://github.com/ml-explore/mlx-swift
  License: licenses/mlx-swift-LICENSE.txt
  Code bundled inside mlx-swift (Source/Cmlx):
  - ml-explore/mlx v0.32.4 (submodule 36969c094354086e15d2a327a9c50feb20b9cf11),
    MIT License: licenses/mlx-LICENSE.txt
  - ml-explore/mlx-c v0.8.0 (submodule a26f2d584a14ec619ae8db9a09be03becd47598a),
    MIT License: licenses/mlx-c-LICENSE.txt
  - fmtlib/fmt 12.1.0, MIT License: licenses/fmt-LICENSE.txt
  - nlohmann/json 3.11.3, MIT License: licenses/nlohmann-json-LICENSE.txt
  - Apple metal-cpp (macOS 15 / iOS 18), Apache License 2.0:
    licenses/metal-cpp-LICENSE.txt
- apple/swift-numerics 1.1.1, revision 0c0290ff6b24942dadb83a929ffaaa1481df04a2,
  Apache License 2.0: licenses/swift-numerics-LICENSE.txt
- swiftlang/swift-syntax 603.0.2, revision 79e4b74a295b6eb74a8b585e3a39d29e70c1dbd1
  (build-time macro support), Apache License 2.0 with Runtime Library Exception:
  licenses/swift-syntax-LICENSE.txt
- huggingface/swift-jinja 3.2.1, revision 3104efda56131c37dda52e9142f02cc2425c02e6,
  Apache License 2.0: licenses/swift-jinja-LICENSE.txt
- huggingface/swift-huggingface 1.0.1, revision 84ced3d4fa31b1bc8c74f420d346f690cb6b0f35,
  Apache License 2.0: licenses/swift-huggingface-LICENSE.txt
- apple/swift-collections 1.7.6, revision 88754449ab766a739610b660e117762115fabc9c,
  Apache License 2.0: licenses/swift-collections-LICENSE.txt
- apple/swift-crypto 4.5.2, revision da9d28d69ebe3894b18376c8f2395c2f37b8448f,
  Apache License 2.0: licenses/swift-crypto-LICENSE.txt, licenses/swift-crypto-NOTICE.txt
- apple/swift-asn1 1.7.4, revision 917b472dbe53ba699cbb0495f9a95a084bc6364d,
  Apache License 2.0: licenses/swift-asn1-LICENSE.txt, licenses/swift-asn1-NOTICE.txt
- ibireme/yyjson 0.13.0, revision 6447536015f3d600f3d65323b10976103b337ca7,
  MIT License: licenses/yyjson-LICENSE.txt
- mattt/EventSource 1.6.0, revision bb4b61273b5a976bb571859c17f903ebc3439df1,
  MIT License: licenses/EventSource-LICENSE.txt

## Model

The summarizer runs Qwen3-4B-Instruct-2507, 4-bit MLX conversion
mlx-community/Qwen3-4B-Instruct-2507-4bit, converted from
Qwen/Qwen3-4B-Instruct-2507, under the Apache License 2.0.
Original: https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507
Conversion: https://huggingface.co/mlx-community/Qwen3-4B-Instruct-2507-4bit
License: licenses/Qwen3-4B-Instruct-2507-LICENSE.txt
The weights are not bundled with this application. The user downloads them,
and they are stored in the local model cache. No changes are made to them.
