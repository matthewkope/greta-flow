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

- ml-explore/mlx-swift 0.33.x (0.33.0 is revision
  7ede4fd6d05f63305c1c0ebf4f8c1909a637c574), MIT License.
  Source: https://github.com/ml-explore/mlx-swift
  License: licenses/mlx-swift-LICENSE.txt
  - Bundles ml-explore/mlx (submodule 36969c094354086e15d2a327a9c50feb20b9cf11),
    MIT License: licenses/mlx-LICENSE.txt
  - Bundles ml-explore/mlx-c (submodule a26f2d584a14ec619ae8db9a09be03becd47598a),
    MIT License: licenses/mlx-c-LICENSE.txt
- apple/swift-numerics, Apache License 2.0: licenses/swift-numerics-LICENSE.txt
- swiftlang/swift-syntax 602.x-603.x (build-time macro support), Apache License 2.0
  with Runtime Library Exception: licenses/swift-syntax-LICENSE.txt
- huggingface/swift-jinja, Apache License 2.0: licenses/swift-jinja-LICENSE.txt
- huggingface/swift-huggingface, Apache License 2.0: licenses/swift-huggingface-LICENSE.txt
- apple/swift-collections, Apache License 2.0: licenses/swift-collections-LICENSE.txt
- apple/swift-crypto, Apache License 2.0: licenses/swift-crypto-LICENSE.txt,
  licenses/swift-crypto-NOTICE.txt
- apple/swift-asn1, Apache License 2.0: licenses/swift-asn1-LICENSE.txt,
  licenses/swift-asn1-NOTICE.txt
- ibireme/yyjson 0.12.0, MIT License: licenses/yyjson-LICENSE.txt
- mattt/EventSource, MIT License: licenses/EventSource-LICENSE.txt

TODO before release: this list was assembled before `Package.resolved` could be
generated (the local Swift 6.2 toolchain cannot resolve mlx-swift 0.33, which
needs swift-tools 6.3). Once resolved, reconcile every entry against
`Package.resolved` and add any packages it pulls in through swift-huggingface
(for example swift-nio, async-http-client, swift-xet, Replay) that are not
listed here.

## Model

The summarizer runs Qwen3-4B-Instruct-2507, 4-bit MLX conversion
mlx-community/Qwen3-4B-Instruct-2507-4bit, converted from
Qwen/Qwen3-4B-Instruct-2507, under the Apache License 2.0.
Original: https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507
Conversion: https://huggingface.co/mlx-community/Qwen3-4B-Instruct-2507-4bit
License: licenses/Qwen3-4B-Instruct-2507-LICENSE.txt
The weights are not bundled with this application. The user downloads them,
and they are stored in the local model cache. No changes are made to them.
