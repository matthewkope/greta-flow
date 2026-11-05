# Model comparison for 0.4

Workspace 3 / PR #3, measured on November 4, 2026.

**Decision after the public corpus and Matt's 14 recordings: ship FluidAudio
v0.18.2 and keep Parakeet TDT 0.6B v2 as the default. No challenger is promoted;
streaming and acoustic boosting stay off.** Matt agreed a warm-call budget of
**median ≤100 ms and p95 ≤150 ms**. Unified meets that budget but regresses on
natural clean dictation and protected names/numbers. V3, Lite and EOU also fail
quality gates. Boosting helps the technical recordings but worsens public-corpus
WER and exceeds the p95 budget there. The previously blocking natural-dictation
comparison and latency-budget decision are now completed; no waiver was used.

Private audio, references, annotations, hashes and raw per-clip transcripts are
kept outside this public repository. Only aggregate private-corpus results and
redacted sample excerpts are published below.

The shipped helper now uses `AsrModels.loadLocal`, so it loads the exact existing
model directory and cannot invoke FluidAudio's model downloader. The v2 weight
revision, hashes, cache/config paths, engine choice, worker protocol and formatting
are unchanged. No app was installed, signed or renamed in this workspace. There are
no frontend changes. The Apache-2.0 FluidAudio license file is byte-identical to the
new upstream license.

## Method and evidence

- Apple M4 Pro, 24 GiB, macOS 15.7.4 (24G517), Swift 6.2.4. Same machine and
  identical SHA-256-verified inputs for every run.
- Existing 120 distinct LibriSpeech test-clean recordings from 40 speakers,
  2,368 normalized words, each in clean, 0.02-gain quiet, and synthetic 20 dB white
  noise conditions: 360 files. Audio and weights remain outside Git. Source,
  attribution, immutable revision and audio hashes are in
  [source.json](../tools/asr-bench/results/2026-11-04/source.json) and
  [manifest.json](../tools/asr-bench/results/2026-11-04/manifest.json).
- One separate first call, followed by two complete warm passes. WER uses only
  pass 1; both warm passes supply latency. Repetitions and noisy variants are not
  new independent speech. The scorer rejects incomplete/duplicate passes.
- Engine runs are sequential with release builds finished first. Ordinary desktop
  activity and model downloads were not disabled; these are session observations,
  not randomized idle-machine or sustained-contention measurements.
- The direct Swift comparison uses preloaded raw 16 kHz PCM, a fresh decoder state
  per utterance, persistent weights, and no application gain/gate/formatting. EOU
  consumes the complete buffer through its chunk processor and flushes the tail;
  its timing is offline processing, **not** paced streaming or release latency.
  Unified is explicitly English with the fp16 offline encoder. Lite uses its GPU
  encoder/CTC head without the separate speaker-diarization model.
- The separate Rust production-path comparison runs the current preparation,
  restored 0.0015 RMS gate, formatting, IPC and segmentation with old/new helpers.
  September's production results used the lower gate and are historical, not the
  baseline for this change. Raw-engine quiet WER and production quiet WER answer
  different questions: the former sends every clip to ASR, the latter rejects
  below-gate clips before gain.
- “Load” is a fresh process with existing system Core ML caches. Candidate bundles
  were first used in this experiment, but caches were not purged. Neither this nor
  the first-call column measures pristine first-install time. Max RSS and peak
  footprint come from `/usr/bin/time -l` for the benchmark process, include its
  preloaded corpus, and exclude separate ANE services. They are not isolated model
  RAM, total app/helper memory or energy.

Raw JSONL, process logs, machine metadata and generated scores are in
[results/2026-11-04](../tools/asr-bench/results/2026-11-04). No microphone, Bluetooth,
real-app insertion, first-install or private-dictation result is inferred from them.

## Measured results

| Model/runtime | Clean WER | Quiet WER | Noisy WER | Clean median / p95 | Quiet median / p95 | Noisy median / p95 |
|---|---:|---:|---:|---:|---:|---:|
| v2 / b68f484 | 2.196% | 2.829% | 2.745% | 60.6 / 94.0 ms | 60.5 / 95.0 ms | 60.3 / 96.1 ms |
| v2 / 0.18.2 | 2.196% | 2.829% | 2.745% | 45.9 / 78.1 ms | 45.9 / 78.7 ms | 46.0 / 80.9 ms |
| v3 / 0.18.2 | 2.365% | 2.534% | 2.576% | 50.8 / 122.6 ms | 50.7 / 119.7 ms | 50.8 / 122.5 ms |
| Nemotron Unified fp16, English | 1.943% | 2.238% | 2.196% | 54.1 / 132.1 ms | 54.3 / 133.9 ms | 54.2 / 132.5 ms |
| Nemotron 3.5 Lite | 6.123% | 7.221% | 7.052% | 164.3 / 167.7 ms | 164.3 / 167.5 ms | 164.4 / 167.6 ms |
| EOU 120M / 1280 ms | 6.419% | 12.247% | 7.855% | 57.2 / 184.9 ms | 57.5 / 180.5 ms | 57.5 / 183.7 ms |
| v2 + CTC 110M boost | 2.365% | 2.956% | 2.829% | 71.6 / 152.7 ms | 71.4 / 152.6 ms | 71.4 / 154.6 ms |

| Model/runtime | Load | First call | Download (decimal MB) | Max RSS (MiB) | Peak footprint (MiB) |
|---|---:|---:|---:|---:|---:|
| v2 / b68f484 | 0.332 s | 114.6 ms | 464.41 | 341.4 | 229.5 |
| v2 / 0.18.2 | 0.188 s | 53.5 ms | 464.41 | 346.9 | 240.1 |
| v3 / 0.18.2 | 14.231 s | 57.0 ms | 483.11 | 681.3 | 244.8 |
| Nemotron Unified fp16, English | 15.246 s | 59.8 ms | 1222.48 | 1366.0 | 241.2 |
| Nemotron 3.5 Lite | 4.615 s | 2761.0 ms | 1363.72 | 2752.2 | 1899.8 |
| EOU 120M / 1280 ms | 1.178 s | 1714.3 ms | 224.53 | 821.6 | 611.2 |
| v2 + CTC 110M boost | 11.289 s | 102.9 ms | 567.23 | 401.9 | 225.1 |

Download totals count selected files plus their retained card/license metadata, not every optional artifact in the repository. Runtime/library downloads and machine-specific compiled caches are excluded.

Old/new v2 transcripts are byte-identical across all 360 clips in pass 1. The new runtime reduces clean median engine time by about 24% in this session. Lite and EOU lose substantially on accuracy. Boosting raises clean WER from 2.196% to 2.365%, changes nine condition-specific transcripts across four unique recordings, and increases median/p95 latency. False replacements include “rest” → “Rust” and “soft” → “Swift”; it remains disabled.

### Transcript samples (public audio, selected to show both gains and regressions)

These are exact complete outputs, not punctuation ground truth. Old/new v2 are identical. The reference columns are the lexical LibriSpeech text.

| Model | Southey sample | Lou sample | Rest/change sample |
|---|---|---|---|
| Reference | I CANNOT DENY MYSELF THE GRATIFICATION OF INSERTING SOUTHEY'S REPLY | I NEVER SEE LOU'S SCYTHE OVER HERE | HE IS NOT WELL HE HAS COME OVER THE OCEAN FOR REST AND CHANGE OF SCENE |
| v2 / 0.18.2 | I cannot deny myself the gratification of inserting Southey's reply. | I never see Lew's scythe over here. | He is not well. He has come over the ocean for rest, and change is seen |
| v3 / 0.18.2 | I cannot deny myself the gratification of inserting Sothy's reply. | I never see Lou's scythe over here. | He is not well. He has come over the ocean for rest, and change has seen. |
| Nemotron Unified fp16, English | I cannot deny myself the gratification of inserting Sothe's reply | I never see Lou's scythe over here | He is not well he has come over the ocean for rest and change of scene. |
| Nemotron 3.5 Lite | I cannot deny myself the gratification of inserting Sy's reply. | I never see Lse shhe over here. | he is not well. He has come over the ocean for rest and change as seen. |
| EOU 120M / 1280 ms | i cannot deny myself the gratification of inserting so these reply | i never see lose sighed over here | he is not well he has come over the ocean for rest and change a scene |
| v2 + CTC 110M boost | I cannot deny myself the gratification of inserting Southey's reply. | I never see Lew's scythe over here. | He is not well. He has come over the ocean for Rust, and change is seen |

Clip IDs: `3575-170457-0027-clean`, `237-134493-0010-clean`, `5142-36377-0010-clean`.

Unified corrects “Lou” and “change of scene” but regresses “Southey” to “Sothe”. Another clean clip (`61-70970-0024-clean`) changes v2’s correct “Warrenton” to “Warrington”. These observed proper-name regressions already argue against promotion, even before the missing natural technical-dictation scores.

### Current production path (same Rust executable, old/new helper)

| Mode/condition | Old WER | New WER | Old median / p95 | New median / p95 |
|---|---:|---:|---:|---:|
| batch / clean | 2.196% | 2.196% | 61.219 / 93.4 ms | 45.930 / 80.5 ms |
| batch / quiet | 81.841% | 81.841% | 0.000 / 69.3 ms | 0.000 / 52.0 ms |
| batch / noise_20db | 2.745% | 2.745% | 59.714 / 99.4 ms | 45.207 / 85.0 ms |
| segmented_offline / clean | 2.196% | 2.196% | 61.019 / 96.5 ms | 45.583 / 80.4 ms |
| segmented_offline / quiet | 81.377% | 81.377% | 0.210 / 69.7 ms | 0.227 / 52.2 ms |
| segmented_offline / noise_20db | 2.745% | 2.745% | 60.031 / 100.5 ms | 45.265 / 86.5 ms |

All 1,441 old/new result transcripts (first call plus warm batch/segmented results) are byte-identical. No segmentation fallbacks occurred. **102/120 quiet clips produce empty batch transcripts under the restored gate**, explaining the 81.841% quiet WER in both versions. The near-zero quiet median is a skip, not fast recognition. The old September low-gate quiet result cannot be substituted for this baseline. No threshold was changed.

Batch timing here is engine/IPC/formatting after preparation; `prepare_seconds` is retained separately in the raw rows. Segmented timing includes offline segmentation and segment preparation. Neither is release-to-inserted latency. The new helper completed every result with networking denied.

The clean WER difference between Unified and v2 is six word errors on this narrow
120-clip corpus. That is a reason to test natural dictation, not evidence that names,
numbers or punctuation are safe. The LibriSpeech references are uppercase lexical
transcripts without authoritative punctuation or technical-entity annotations;
**names, numbers and punctuation scores are unmeasured on the public corpus**.
Its scorer emits `null` for those fields, not zero errors. The annotated natural
dictation below supplies those measurements, using Matt's supplied reference text
rather than a model-generated reference.

## Matt’s natural technical dictation

Completed November 4 on the same M4 Pro, 24 GiB, macOS 15.7.4. The 14 supplied
mono 16 kHz WAVs contain **79.381 seconds** of speech from one speaker (individual
clips 4.256–9.003 seconds), with **182 normalized reference words**. Each original
was converted to float PCM without resampling, then given the same 0.02-gain and
20 dB white-noise variants (seed 33): 42 inputs, not 42 independent recordings.
All seven configurations ran sequentially with network access denied, no concurrent
builds, and the same pinned artifacts as the public experiment. Each had one
separate first call and two complete warm passes (84 warm calls); accuracy uses
pass 1. Ordinary desktop activity was not disabled.

Before running inference, the supplied references were annotated locally with
**16 name/jargon spans and 8 numeric spans per condition**. Name scoring requires
exact spelling and case; it includes product/technology names, jargon and person
names. Number scoring requires the intended reference formatting, including time,
date, currency, decimal and phone formats. Thus spelled-out equivalents may pass
normalized WER but fail the written-form check. These metrics do not claim semantic
errors for every formatting mismatch. These counts cover annotated targets, not
arbitrary new entities. No extra expected-span occurrences were observed. The glossary for the boost run is unchanged from the public experiment;
it was not tuned to these recordings or augmented with the private person's name.

Punctuation is additionally measured **at aligned normalized-word boundaries**:
commas, sentence-ending marks, semicolons and colons are compared after deterministic
minimum-edit word alignment. Internal decimal/name dots, numeric grouping commas,
clock colons, currency signs, apostrophes and phone separators are excluded from
this sentence-punctuation score. There are **37 reference marks** per condition.
A moved mark can count as deletion plus insertion; rates can exceed 100%.
Lexical alignment errors can influence this measure, so it is reported alongside
WER rather than as a grammar-quality guarantee. The earlier symbol-sequence score
(including numeric/name formatting) is retained separately in the aggregate JSON.
Tests cover marks moved to the wrong word, normalized number formats, internal
separators, word insertions, missing/extra marks and empty transcripts.

[dictation-aggregate.json](../tools/asr-bench/results/2026-11-04/dictation-aggregate.json)
contains only aggregates; raw evidence and annotations remain private. The sample
excerpts below remove the entire name/phone-bearing context, including recognition
variants, rather than relying on exact-string replacement.

### WER and latency

| Model/runtime | Clean WER | Quiet WER | Noisy WER | Clean median / p95 | Quiet median / p95 | Noisy median / p95 |
|---|---:|---:|---:|---:|---:|---:|
| v2 / b68f484 | 8.791% | 12.637% | 12.088% | 57.6 / 63.2 ms | 57.4 / 58.8 ms | 57.2 / 58.6 ms |
| v2 / 0.18.2 | 8.791% | 12.637% | 12.088% | 43.8 / 51.9 ms | 44.4 / 49.0 ms | 43.9 / 48.7 ms |
| v3 | 10.440% | 15.385% | 9.341% | 50.5 / 58.3 ms | 50.4 / 57.2 ms | 49.5 / 57.3 ms |
| Unified (English fp16) | 12.088% | 10.989% | 9.890% | 53.0 / 58.3 ms | 52.2 / 57.1 ms | 52.2 / 58.6 ms |
| Lite | 18.681% | 20.879% | 21.978% | 164.5 / 167.2 ms | 164.3 / 166.1 ms | 164.5 / 166.6 ms |
| EOU 1280 ms | 22.527% | 57.692% | 27.473% | 54.1 / 80.7 ms | 53.5 / 79.4 ms | 54.1 / 80.2 ms |
| v2 + CTC boost | 7.692% | 12.088% | 10.989% | 69.3 / 76.2 ms | 71.9 / 76.9 ms | 69.6 / 76.9 ms |

### Names, numbers and sentence punctuation

Each cell lists **clean / quiet / noisy** results. Higher preserved-span counts are better; lower punctuation edit counts are better. These are pass-1 counts, not inflated by repeats.

| Model/runtime | Names/jargon preserved (of 16 each) | Numeric formats preserved (of 8 each) | Sentence punctuation edits (37 marks each) |
|---|---:|---:|---:|
| v2 / b68f484 | 11 / 10 / 10 | 3 / 3 / 4 | 18 / 8 / 16 |
| v2 / 0.18.2 | 11 / 10 / 10 | 3 / 3 / 4 | 18 / 8 / 16 |
| v3 | 10 / 9 / 11 | 0 / 1 / 5 | 14 / 12 / 14 |
| Unified (English fp16) | 7 / 8 / 10 | 2 / 3 / 3 | 16 / 14 / 16 |
| Lite | 7 / 6 / 5 | 0 / 0 / 0 | 13 / 11 / 12 |
| EOU 1280 ms | 0 / 0 / 0 | 0 / 0 / 0 | 37 / 37 / 37 |
| v2 + CTC boost | 12 / 11 / 11 | 3 / 3 / 4 | 18 / 8 / 16 |

### Load and process memory

Fresh processes with already-used Core ML artifacts/caches; these are **not pristine first-install times**. Memory includes the preloaded 42-file corpus and process runtime, excludes separate ANE services, and is not isolated model RAM. Artifact download sizes are unchanged from the table above.

| Model/runtime | Load | First call | Max RSS (MiB) | Peak footprint (MiB) |
|---|---:|---:|---:|---:|
| v2 / b68f484 | 0.275 s | 100.2 ms | 118.5 | 52.0 |
| v2 / 0.18.2 | 12.681 s | 57.7 ms | 512.8 | 61.1 |
| v3 | 0.156 s | 93.6 ms | 112.8 | 49.0 |
| Unified (English fp16) | 0.409 s | 121.1 ms | 91.0 | 66.5 |
| Lite | 2.111 s | 335.1 ms | 2448.7 | 1827.2 |
| EOU 1280 ms | 0.396 s | 209.8 ms | 446.1 | 434.9 |
| v2 + CTC boost | 0.474 s | 118.3 ms | 130.3 | 56.3 |

The 12.681 s upgraded-v2 load shows that startup can still require substantial
preparation even with previously used artifacts. The warm-call budget excludes
load/first-call time; these results do not establish a cold-start improvement.

Old/new v2 transcripts match exactly for all **85 direct-engine results** (first call plus two warm passes). The upgrade changes timing, not recognition output, on these recordings.

### Redacted side-by-side excerpts

Names and phone numbers are not published, including approximate recognitions. These are selected excerpts from pass-1 clean results, not the private references or raw transcript files. The omitted context can contain recognition errors; its accuracy is included in the aggregate metrics above.

| Model | Clip 03 excerpt | Clip 05 excerpt |
|---|---|---|
| Reference | [NAME CONTEXT REDACTED] … moved to 3:45 on Tuesday, November 18th. | [PHONE CONTEXT REDACTED] … if the server goes down after 9 PM. |
| v2-old | [NAME CONTEXT REDACTED] … moved to 345 on Tuesday, November 18th. | [PHONE CONTEXT REDACTED] … if the server goes down after 9pm. |
| v2-new | [NAME CONTEXT REDACTED] … moved to 345 on Tuesday, November 18th. | [PHONE CONTEXT REDACTED] … if the server goes down after 9pm. |
| v3 | [NAME CONTEXT REDACTED] … moved to three forty five on Tuesday, november eighteenth. | [PHONE CONTEXT REDACTED] … if the server goes down after nine PM. |
| unified | [NAME CONTEXT REDACTED] … moved to three forty five on Tuesday, november eighteenth. | [PHONE CONTEXT REDACTED] … if the server goes down after 9 p.m. |
| lite | [NAME CONTEXT REDACTED] … moved to three forty five on Tuesday, November, eighteenth. | [PHONE CONTEXT REDACTED] … if the server goes down after nine pm. |
| eou | [NAME CONTEXT REDACTED] … moved today november eighteenth | [PHONE CONTEXT REDACTED] … if the service go down after nine pm |
| v2-boost | [NAME CONTEXT REDACTED] … moved to 345 on Tuesday, November 18th. | [PHONE CONTEXT REDACTED] … if the server goes down after 9pm. |

### Promotion gate

The agreed budget applies to warm **engine calls**, not release-to-cursor or real
microphone latency. It is checked per condition on both English corpora; figures
are session measurements, not a performance guarantee. No test outcome was used
to relax the 100/150 ms thresholds.

| Candidate | Latency gate across both corpora | Quality gate and decision |
|---|---|---|
| v2 on v0.18.2 | Pass | Same transcripts as pinned v2; clean natural median/p95 43.8/51.9 ms. Retain as default. |
| v3 | Pass | Natural clean WER 10.440% vs 8.791%; fewer names and numeric formats preserved. Public clean WER also regresses. Do not promote. |
| Nemotron Unified | Pass | Public WER improves in all conditions, but natural clean WER rises from 16 to 22 word errors (8.791% → 12.088%). Clean name/jargon preservation falls from 11/16 to 7/16 and numeric-format preservation from 3/8 to 2/8; quiet punctuation edits rise from 8 to 14. No-regression gate fails even though noisy/quiet natural WER improves. Do not promote. |
| Lite | Fail | Natural median ~164 ms and p95 ~167 ms; worse WER and name/number preservation. Do not promote. |
| EOU 120M | Fail on public p95 | Natural short-clip latency passes, but public p95 reaches ~185 ms. WER, case-sensitive names, numeric formatting and punctuation regress. Do not promote. |
| v2 + CTC boosting | Fail on public p95 | Natural clean WER improves to 7.692%, but public clean WER worsens to 2.365% and p95 reaches ~155 ms, with false replacements. Keep boosting off. |

No challenger satisfies the promotion requirements, so the plan's “if nothing
wins, keep v2” outcome applies. There is no winning second engine to integrate;
retained dual caches, load-failure fallback and the Workspace 5 switch are not
introduced in this PR. Existing Parakeet availability and cache behavior are
unchanged. This is a measured rejection for this release, not a conclusion that
Unified can never win with a different artifact or future runtime.

The small, single-speaker natural set consists entirely of short recordings. It
supports this gate decision but does not establish long-form, other-speaker,
Bluetooth, live-insertion, first-install or contention behavior. The synthetic
variants are not real quiet-room/noisy-room recordings.

## Vocabulary boosting

The current application dictionary still performs deterministic post-recognition
corrections. The new library offers acoustic CTC rescoring for v2/v3, but those TDT
models need a **separate CTC encoder, mel model and tokenizer**. This experiment pins
Parakeet CTC 110M, an additional 102,810,506 bytes including its card/tokenizer, under
CC-BY-4.0. The hybrid TDT-CTC-110M can instead reuse its encoder with a small CTC head;
that is a different main ASR model, not a free add-on to v2. See the pinned
[upstream guide](https://github.com/FluidInference/FluidAudio/blob/e396cf780c4abf8e395087467c0585d3b0d17f68/Documentation/ASR/CustomVocabulary.md).

The `v2-boost` experiment runs the same spotting/rescoring operations as
`VocabularyBoostingSession`, with explicit local tokenizer paths, vocabulary-size
thresholds (CBW 4.5, minimum similarity 0.50 for 12 terms), 0.5 s margin and default
rescorer safety rules. It fails on CTC errors instead of silently reporting unboosted
output as a successful boost. It preserves original text when no replacement was
applied. No network loader or production dictionary change is introduced.

The [12-term glossary](../tools/asr-bench/technical-glossary.json) was chosen before
the boost run, from application/developer names rather than its output errors. This
public audiobook run is primarily an off-domain false-replacement and overhead
check; it does not measure recall for naturally spoken technical terms. The natural
recordings above now provide positive technical examples: boosting
improves their aggregate WER, but the public-corpus regressions and latency gate
still reject enabling it by default.

## Artifacts, licensing and lower-priority screening

[model-manifest-0.4.json](../tools/asr-bench/model-manifest-0.4.json) records exact
conversion repository commits, selected files, byte counts, SHA-256s and model-card
license declarations. The downloader verifies every file. It does not replace the
application's existing v2 manifest. No challenger is shipped or automatically fetched.

| Tested artifact | Conversion revision | License at that revision |
|---|---|---|
| Parakeet v2 | `ee09c569f73759e6d44c9bd16766f477b2b36d39` | [CC-BY-4.0 card](https://huggingface.co/FluidInference/parakeet-tdt-0.6b-v2-coreml/blob/ee09c569f73759e6d44c9bd16766f477b2b36d39/README.md) |
| Parakeet v3 | `7dd20fe6b1797d35f5e3307e8b1732d9a178edfe` | [CC-BY-4.0 card](https://huggingface.co/FluidInference/parakeet-tdt-0.6b-v3-coreml/blob/7dd20fe6b1797d35f5e3307e8b1732d9a178edfe/README.md) |
| Nemotron Unified | `bd13ddde81d3134719c520845346b3252daf3175` | [OpenMDW-1.1 license](https://huggingface.co/FluidInference/nemotron-labs-unified-asr-0.6b-coreml/blob/bd13ddde81d3134719c520845346b3252daf3175/LICENSE.md); English is one of its 25 languages |
| Nemotron 3.5 Transcribe Lite | `31ecb73ab8a541001c0952802b478384cf9f0691` | [OpenMDW-1.1 license](https://huggingface.co/FluidInference/nemotron-3.5-transcribe-lite-coreml/blob/31ecb73ab8a541001c0952802b478384cf9f0691/LICENSE); “Lite” is FluidInference's encoder/CTC extraction, not the full NVIDIA 30B model |
| EOU 120M, 1280 ms | `40a23f4c0b333aa17ad8c0f2ea47ec2347f2f355` | [Pinned card](https://huggingface.co/FluidInference/parakeet-realtime-eou-120m-coreml/blob/40a23f4c0b333aa17ad8c0f2ea47ec2347f2f355/README.md) declares NVIDIA Open Model License via an external URL; a frozen license-text provenance record is still required before promotion |
| CTC 110M | `accdafd8cf8a2ff1cabe3c11e54416b405d409aa` | [CC-BY-4.0 card](https://huggingface.co/FluidInference/parakeet-ctc-110m-coreml/blob/accdafd8cf8a2ff1cabe3c11e54416b405d409aa/README.md) |

Priority-3 candidates were screened, **not benchmarked locally**:

| Candidate | Verified finding and disposition |
|---|---|
| Bark 3.0 / Clean | [Pinned conversion card](https://huggingface.co/FluidInference/bark-3.0-coreml/blob/73b0c0da665d29ea0332e9f0d32da8e26a4cdb38/README.md): CC-BY-4.0, macOS 15 minimum (app supports 14), 399/410 MB encoders. Upstream reports ~140–155× RTFx on M5 Pro; Clean changes fillers/repetitions. Deferred behind compatible priority-1 models; no local speed/accuracy claim. |
| Canary 1B v2 | [Pinned conversion card](https://huggingface.co/FluidInference/canary-1b-v2-coreml/blob/224631d3724c8f34c2d4d99b92e7e26cd6d2f93b/README.md): CC-BY-4.0, default int4 requires macOS 15, ~573 MB and ~7× RTFx upstream. The macOS-14 fp16 option is ~1.8 GB. Deferred for OS/size and likely latency tradeoffs; local latency is unknown. |
| Qwen3-ASR 1.7B | Live English leaderboard rank 10, 6.14% average WER. No Qwen3-ASR manager in the pinned FluidAudio source; a separate runtime/integration is needed. Not tested. |
| Granite Speech 4.1 2B | Live English leaderboard rank 28, 6.92%, not at the very top of the current table. No Granite manager in the pinned FluidAudio source. Not tested. |
| Cohere Transcribe | Live English leaderboard rank 17, 6.72%. Contrary to the plan's initial uncertainty, there **is** a Core ML path in v0.18.2. Conversion is [Apache-2.0](https://huggingface.co/FluidInference/cohere-transcribe-03-2026-coreml/blob/7ca8c223e84c817a7e125c567c2582559dcd8e32/README.md), with ~1.8 GB encoder plus ~304 MB decoder weights. Pinned [runtime measurements](https://github.com/FluidInference/FluidAudio/blob/e396cf780c4abf8e395087467c0585d3b0d17f68/Documentation/ASR/Cohere.md) show second-scale warm processing and substantial peak RSS on another Mac. Deferred; these are not measurements on this machine. |

Leaderboard source: [Open ASR Leaderboard](https://huggingface.co/spaces/hf-audio/open_asr_leaderboard),
English default selection, version 02-11-2026, retrieved November 4. The visible
Gradio table is retained in [leaderboard.json](../tools/asr-bench/results/2026-11-04/leaderboard.json).
Its dataset mixture and hardware differ from this experiment; leaderboard RTFx
must not be presented as Mac dictation latency. Parakeet Ultra is not part of this
plan or experiment.

## Natural dictation and frontend handoff

The supplied 14 recordings complete the previously missing natural-dictation
comparison. [DICTATION_0.4.md](../tools/asr-bench/DICTATION_0.4.md) documents the
local preparation/annotation process; all original/private derivative files remain
outside Git. The agreed latency budget and measured promotion decision are above.
No private audio, reference file, per-clip result file or audio hash was published.

No second application engine or menu option is requested for Workspace 5 because
no challenger passes this release's quality/latency gates. Any future promotion
must still implement retained dual caches, offline switching back to Parakeet,
load-failure fallback with a reason log, and a backend selector before Workspace 5
designs its menu. No frontend or default-setting change is part of this PR.

## Reproduce

Use the pinned Python dependencies from `tools/asr-bench/requirements.txt` in a
Python 3.12 environment. Set `PYTHON`, `BENCH` (outside Git), `CORPUS` and `RESULTS`
to local paths. All audio remains local. Source/weight downloads are explicit setup
steps, separate from recognition.

```sh
"$PYTHON" tools/asr-bench/prepare_review_corpus.py "$CORPUS"
"$PYTHON" tools/asr-bench/download_models.py "$BENCH/models"
swift build --package-path tools/asr-bench/swift -c release
bash desktop/scripts/build-parakeet.sh

# Run sequentially, retaining stderr (/usr/bin/time -l).
/usr/bin/time -l tools/asr-bench/swift/.build/release/ASRBench \
  "$BENCH/models/v2" "$CORPUS/manifest.json" 2 v2 > "$RESULTS/v2-new.jsonl" 2> "$RESULTS/v2-new.log"
/usr/bin/time -l tools/asr-bench/swift/.build/release/ASRBench \
  "$BENCH/models/v3" "$CORPUS/manifest.json" 2 v3 > "$RESULTS/v3.jsonl" 2> "$RESULTS/v3.log"
/usr/bin/time -l tools/asr-bench/swift/.build/release/ASRBench \
  "$BENCH/models/unified/coreml" "$CORPUS/manifest.json" 2 unified \
  > "$RESULTS/unified.jsonl" 2> "$RESULTS/unified.log"
/usr/bin/time -l tools/asr-bench/swift/.build/release/ASRBench \
  "$BENCH/models/lite" "$CORPUS/manifest.json" 2 lite > "$RESULTS/lite.jsonl" 2> "$RESULTS/lite.log"
/usr/bin/time -l tools/asr-bench/swift/.build/release/ASRBench \
  "$BENCH/models/eou/1280ms" "$CORPUS/manifest.json" 2 eou > "$RESULTS/eou.jsonl" 2> "$RESULTS/eou.log"
/usr/bin/time -l tools/asr-bench/swift/.build/release/ASRBench \
  "$BENCH/models/v2" "$CORPUS/manifest.json" 2 v2-boost \
  "$BENCH/models/ctc" tools/asr-bench/technical-glossary.json \
  > "$RESULTS/v2-boost.jsonl" 2> "$RESULTS/v2-boost.log"
```

For the old baseline, extract `tools/asr-bench/swift` and `desktop/parakeet` from
base commit `32b9e75ace28ecf1ba890c07cb8dab8559b31e1c` into a separate scratch
source directory. Build those sources with `FLUID_AUDIO_PATH` set to an unmodified
checkout of `b68f484789d81fda21efbf81e2ca9fcfd9dc22aa`. The old Swift harness takes
`MODEL_DIRECTORY MANIFEST REPEATS` without a model selector. Its loader expects the
v2 folder named `parakeet-tdt-0.6b-v2-coreml`; use the same pinned v2 files. Do not
point the old package override at the new library or compare a historical result
from another day as though it were a paired baseline.

Save the old baseline output as `$RESULTS/v2-old.jsonl`. Once all seven runs are
complete, score every configuration explicitly (including Unified, Lite and EOU):

```sh
"$PYTHON" tools/asr-bench/model_summary.py "$CORPUS/manifest.json" \
  "$RESULTS/v2-old.jsonl" "$RESULTS/v2-new.jsonl" "$RESULTS/v3.jsonl" \
  "$RESULTS/unified.jsonl" "$RESULTS/lite.jsonl" "$RESULTS/eou.jsonl" \
  "$RESULTS/v2-boost.jsonl"
```

For production-path verification:

```sh
cargo build --locked --release --manifest-path desktop/src-tauri/Cargo.toml --example review_bench
GRETCHEN_PARAKEET_HELPER="$HELPER" /usr/bin/time -l \
  desktop/src-tauri/target/release/examples/review_bench "$MODEL" "$CORPUS/manifest.json" \
  > "$RESULTS/production.jsonl" 2> "$RESULTS/production.log"
"$PYTHON" tools/asr-bench/review_summary.py "$CORPUS/manifest.json" "$RESULTS/production.jsonl"
```

Run once with each helper, keeping the Rust executable, model and corpus unchanged.
The new-helper run in this report uses a wrapper executing
`/usr/bin/sandbox-exec -p '(version 1)(allow default)(deny network*)'` before the
helper and forwards its arguments. The benchmark does not invoke an installer,
change app settings or insert into another app.

## Validation

- `cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --check`: pass.
- `cargo clippy --locked --all-targets --manifest-path desktop/src-tauri/Cargo.toml -- -D warnings`: pass.
- `cargo test --locked --all-targets --manifest-path desktop/src-tauri/Cargo.toml`: 63 application, 36 ASR example and 47 review example tests pass.
- Helper and benchmark Swift release builds pass; 18 Python scoring tests pass (including six sentence-punctuation tests).
- Seven direct-engine configurations each complete 720 warm calls plus a separate first call; all artifacts/corpus hashes verified. The old library's 709 source files match the immutable upstream archive.
- Both current production-path runs complete 1,440 warm result rows plus a first call, with identical old/new transcripts. The new worker runs with network denied; a missing model fails locally without a ready response or downloaded files.
- The 14 supplied natural recordings produce 42 condition variants. Seven direct-engine configurations complete 84 warm calls plus one first call each with networking denied. All 85 old/new v2 transcripts match; private evidence stays local.
- Published private-corpus output is restricted to aggregate metrics and redacted excerpts; the staged text is checked against the private references, sensitive identifiers and raw-result files.
- `git diff --check`: pass. No frontend, icons, bundle ID, signing material, user config or installed app was changed.

The upgrade, public comparison and natural-dictation promotion decision are
complete for this PR. macOS 14 execution, pristine first-install preparation,
live microphones, real-app insertion and power/memory under contention remain
field validation; no such behavior is inferred from the offline runs.

## PR #3 review follow-up

The reproduction bug is fixed: explicit commands cover all candidates and the
scorer lists all seven inputs, including Unified, Lite and EOU. The exact command
reproduces all retained public metrics.

The blocking natural-dictation gap is now resolved with Matt's 14 recordings and
his explicit 100 ms median / 150 ms p95 budget. All seven configurations were run
on the private corpus; aggregate name, number, punctuation, WER and latency results
are published above. The v2 decision is now measured rather than provisional. No
waiver was used. Private audio, references and raw per-clip transcripts stay local;
only aggregates and redacted excerpts are included in this public PR.
