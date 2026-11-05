# Local ASR benchmark

Tests the downloaded Parakeet TDT v2 Core ML model, Moonshine Medium Streaming model, and Greta Flow's Whisper Turbo engine. It does not capture microphone audio, inject text, or change app settings.

## Model comparison for 0.4

See [MODEL_COMPARISON_0.4.md](../../docs/MODEL_COMPARISON_0.4.md) for the new
same-machine comparison, pinned candidate artifacts, exact commands and limitations.
The current Swift harness pins FluidAudio v0.18.2 and uses local-only model loaders.
It accepts an optional model ID after REPEATS (`v2`, `v3`, `unified`, `lite`, `eou`),
or `v2-boost CTC_DIRECTORY GLOSSARY_JSON` for acoustic vocabulary rescoring.
Use `model_summary.py` for condition-specific scores and first-pass accuracy.
[DICTATION_0.4.md](DICTATION_0.4.md) documents the completed private-dictation comparison and local-only reproduction process.

## Reproduce the historical September smoke comparison

Check out the September benchmark revision for the original Swift API and raw baseline.
Use Python 3.12 and the pinned dependencies in `requirements.txt`. Download the following artifacts outside the repository:

- FluidAudio source revision `b68f484789d81fda21efbf81e2ca9fcfd9dc22aa` from `FluidInference/FluidAudio`.
- `FluidInference/parakeet-tdt-0.6b-v2-coreml` revision `ee09c569f73759e6d44c9bd16766f477b2b36d39` from Hugging Face. Name its directory `parakeet-tdt-0.6b-v2-coreml` because FluidAudio resolves that sibling directory internally. This experiment used its default `Encoder`, `Decoder`, `Preprocessor`, `JointDecision` and vocabulary; not the alternate 4-bit encoder.
- Moonshine English `ModelArch.MEDIUM_STREAMING`: `https://download.moonshine.ai/model/medium-streaming-en/quantized_26_08_21`. Use `get_model_for_language("en", ModelArch.MEDIUM_STREAMING, cache_root=...)` from `moonshine_voice.download`, then verify the selected model URL and file hashes against the recorded run. The downloader's defaults can change; do not assume a future download is identical.
- Existing `ggml-large-v3-turbo.bin`. The Rust example uses English, beam size 5, 700 ms pause punctuation, fillers/lists enabled, and the default Gretchen Flow glossary. It includes the app's state allocation and text cleanup; it does not benchmark a different Whisper Python implementation.

```sh
uv venv --python 3.12 /path/to/bench/.venv
uv pip install --python /path/to/bench/.venv/bin/python -r tools/asr-bench/requirements.txt
/path/to/bench/.venv/bin/python tools/asr-bench/prepare_corpus.py /path/to/bench/corpus

# Run from tools/asr-bench/swift:
FLUID_AUDIO_PATH=/path/to/FluidAudio swift build -c release
.build/release/ASRBench /path/to/parakeet-tdt-0.6b-v2-coreml /path/to/bench/corpus/manifest.json 3 > /path/to/bench/parakeet.jsonl

# Run from desktop/src-tauri:
cargo build --locked --release --example asr_bench
target/release/examples/asr_bench /path/to/ggml-large-v3-turbo.bin /path/to/bench/corpus/manifest.json 3 > /path/to/bench/whisper.jsonl

# Run from repository root:
/path/to/bench/.venv/bin/python tools/asr-bench/moonshine_bench.py /path/to/moonshine/model /path/to/bench/corpus/manifest.json --repeats 3 > /path/to/bench/moonshine.jsonl
/path/to/bench/.venv/bin/python tools/asr-bench/moonshine_bench.py /path/to/moonshine/model /path/to/bench/corpus/manifest.json --repeats 1 --stream > /path/to/bench/moonshine-stream.jsonl
/path/to/bench/.venv/bin/python tools/asr-bench/summarize.py /path/to/bench/corpus/manifest.json /path/to/bench/parakeet.jsonl /path/to/bench/whisper.jsonl /path/to/bench/moonshine.jsonl /path/to/bench/moonshine-stream.jsonl
/path/to/bench/.venv/bin/python tools/asr-bench/compare.py /path/to/bench/corpus/manifest.json /path/to/bench/whisper.jsonl /path/to/bench/parakeet.jsonl /path/to/bench/moonshine.jsonl /path/to/bench/moonshine-stream.jsonl
```

Run engines sequentially, after builds finish. Pass 0 is a first call/warm-up and is excluded from warm statistics. Audio is loaded before timing. For Moonshine, timed calls include the Python wrapper's conversion to C floats; Rust and Swift receive preloaded float arrays. This compares realistic local engine paths, not pure kernel speed.

The streaming test feeds real audio in 100 ms chunks at real-time deadlines, requests updates every 500 ms, then calls stop to flush. `release_to_final_seconds` measures from the end-of-file capture deadline, including processing backlog; it is not merely the duration of stop(). This excludes real recorder shutdown and cursor injection. Each stream is closed between clips. No frontend is involved.

Accuracy is reported both with simple lexical normalization and with `EnglishTextNormalizer`, which avoids penalizing equivalent `Mr./Mister`, `10/ten`, and contractions. WER does not score punctuation or prove entity preservation. Repeated transcripts are not independent samples: `compare.py` bootstraps paired unique clips from pass 1. Its interval is conditional on this narrow corpus, not a population guarantee.

The corpus is 24 evenly selected clips from the pinned [LibriSpeech dummy validation subset](https://huggingface.co/datasets/hf-internal-testing/librispeech_asr_dummy), revision `5be91486e11a2d616f4ec5db8d3fd248585ac07a`. These are real audiobook recordings from one speaker, not synthetic audio. The set totals 147.145 seconds and is appropriate for an initial smoke comparison only. Downloaded audio and model binaries stay outside Git.

Model cards and source licenses: [Parakeet v2](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2), [Core ML conversion](https://huggingface.co/FluidInference/parakeet-tdt-0.6b-v2-coreml), [FluidAudio](https://github.com/FluidInference/FluidAudio), [Moonshine](https://github.com/moonshine-ai/moonshine).
