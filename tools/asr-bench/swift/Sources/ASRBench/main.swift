import AVFoundation
import FluidAudio
import Foundation

struct Clip: Decodable {
    let id: String
    let path: String
    let seconds: Double
}

func emit(_ row: [String: Any]) throws {
    let data = try JSONSerialization.data(withJSONObject: row, options: [.sortedKeys])
    try FileHandle.standardOutput.write(contentsOf: data + Data([10]))
}

// Benchmark-only adapters. All loaders use explicit local artifacts; downloads are a separate step.
// EOU is fed a complete clip and flushed, so its timings are offline processing, not release latency.
@MainActor
func recognizer(model: String, directory: URL, ctcDirectory: URL? = nil, terms: [String] = []) async throws -> ([Float]) async throws -> String {
    switch model {
    case "v2", "v3", "v2-boost":
        let models = try AsrModels.loadLocal(from: directory, version: model == "v3" ? .v3 : .v2)
        let manager = AsrManager()
        try await manager.loadModels(models)
        if model == "v2-boost" {
            guard let ctcDirectory, !terms.isEmpty else {
                throw NSError(domain: "Boosting needs a CTC directory and nonempty glossary", code: 1)
            }
            let tokenizer = try await CtcTokenizer.load(from: ctcDirectory)
            let vocabulary = CustomVocabularyContext(terms: terms.map {
                CustomVocabularyTerm(text: $0, ctcTokenIds: tokenizer.encode($0))
            })
            let ctc = try await CtcModels.loadDirect(from: ctcDirectory)
            let spotter = CtcKeywordSpotter(models: ctc, blankId: ctc.vocabulary.count)
            let rescorer = try await VocabularyRescorer.create(
                spotter: spotter, vocabulary: vocabulary, ctcModelDirectory: ctcDirectory)
            let thresholds = ContextBiasingConstants.rescorerConfig(forVocabSize: terms.count)
            return { samples in
                var state = try TdtDecoderState()
                let result = try await manager.transcribe(samples, decoderState: &state)
                guard let timings = result.tokenTimings, !timings.isEmpty else {
                    throw NSError(domain: "Boosting benchmark requires token timings", code: 1)
                }
                // Same acoustic rescore as VocabularyBoostingSession, using explicit local tokenizer paths.
                // Fail the benchmark on CTC errors; do not silently score unboosted output as boosted.
                let evidence = try await spotter.spotKeywordsWithLogProbs(
                    audioSamples: samples, customVocabulary: vocabulary, minScore: nil)
                guard !evidence.logProbs.isEmpty else {
                    throw NSError(domain: "No CTC evidence", code: 1)
                }
                let rescored = rescorer.ctcTokenRescore(
                    transcript: result.text, tokenTimings: timings, logProbs: evidence.logProbs,
                    frameDuration: evidence.frameDuration, cbw: thresholds.cbw,
                    marginSeconds: 0.5, minSimilarity: max(thresholds.minSimilarity, vocabulary.minSimilarity))
                return rescored.wasModified ? rescored.text : result.text
            }
        }
        return { samples in
            var state = try TdtDecoderState()
            return try await manager.transcribe(samples, decoderState: &state).text
        }
    case "unified":
        let manager = UnifiedAsrManager(config: UnifiedConfig(family: .nemotronMultilingual(.en)), encoderPrecision: .fp16)
        try await manager.loadModels(from: directory)
        return { samples in try await manager.transcribe(samples) }
    case "lite":
        let manager = NemotronTranscribeLiteManager(models: try NemotronTranscribeLiteModels.load(from: directory))
        return { samples in try await manager.transcribe(audio: samples).text }
    case "eou":
        let manager = StreamingEouAsrManager(chunkSize: .ms1280)
        try await manager.loadModels(from: directory)
        return { samples in
            await manager.reset()
            guard let format = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 16000, channels: 1, interleaved: false),
                  let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(samples.count)),
                  let channel = buffer.floatChannelData?[0] else {
                throw NSError(domain: "Cannot allocate audio buffer", code: 1)
            }
            buffer.frameLength = AVAudioFrameCount(samples.count)
            samples.withUnsafeBufferPointer { channel.update(from: $0.baseAddress!, count: samples.count) }
            try await manager.appendAudio(buffer)
            try await manager.processBufferedAudio()
            return try await manager.finish()
        }
    default:
        throw NSError(domain: "Unknown model: \(model)", code: 1)
    }
}

let args = CommandLine.arguments
guard [4, 5, 7].contains(args.count), let repeats = Int(args[3]), repeats > 0 else {
    throw NSError(domain: "Usage: ASRBench MODEL_DIRECTORY MANIFEST REPEATS [v2|v3|unified|lite|eou|v2-boost CTC_DIRECTORY GLOSSARY_JSON]", code: 1)
}
let model = args.count >= 5 ? args[4] : "v2"
guard (model == "v2-boost") == (args.count == 7) else {
    throw NSError(domain: "Only v2-boost accepts and requires CTC_DIRECTORY GLOSSARY_JSON", code: 1)
}
let terms = args.count == 7 ? try JSONDecoder().decode([String].self, from: Data(contentsOf: URL(fileURLWithPath: args[6]))) : []
let clips = try JSONDecoder().decode([Clip].self, from: Data(contentsOf: URL(fileURLWithPath: args[2])))
guard !clips.isEmpty, Set(clips.map(\.id)).count == clips.count else {
    throw NSError(domain: "Empty manifest or duplicate clip IDs", code: 1)
}
let audio: [[Float]] = try clips.map { clip in
    let data = try Data(contentsOf: URL(fileURLWithPath: clip.path))
    guard !data.isEmpty, data.count % 4 == 0, clip.seconds.isFinite, clip.seconds > 0,
          abs(Double(data.count / 4) / 16000 - clip.seconds) < 0.001 else {
        throw NSError(domain: "Invalid 16 kHz float PCM: \(clip.id)", code: 1)
    }
    let samples = data.withUnsafeBytes { bytes in
        stride(from: 0, to: bytes.count, by: 4).map { offset in
            Float(bitPattern: UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: offset, as: UInt32.self)))
        }
    }
    guard samples.allSatisfy(\.isFinite) else { throw NSError(domain: "Non-finite PCM", code: 1) }
    return samples
}
let loadStart = DispatchTime.now().uptimeNanoseconds
let transcribe = try await recognizer(model: model, directory: URL(fileURLWithPath: args[1]),
                                       ctcDirectory: args.count == 7 ? URL(fileURLWithPath: args[5]) : nil, terms: terms)
try emit(["event": "load", "model": model, "seconds": Double(DispatchTime.now().uptimeNanoseconds - loadStart) / 1e9])
for pass in 0...repeats {
    // Pass zero records a first call and warms the model; excluded from warm aggregates.
    let indices = pass == 0 ? [0] : Array(clips.indices)
    for i in indices {
        let start = DispatchTime.now().uptimeNanoseconds
        let text = try await transcribe(audio[i])
        let elapsed = Double(DispatchTime.now().uptimeNanoseconds - start) / 1e9
        try emit(["event": "transcribe", "model": model, "id": clips[i].id,
                  "pass": pass, "seconds": elapsed, "audio_seconds": clips[i].seconds, "text": text])
    }
}
