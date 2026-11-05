import FluidAudio
import Foundation

// One persistent, serial decoder. PCM stays in pipes and is never written to disk.
func emit(_ row: [String: Any]) throws {
    let data = try JSONSerialization.data(withJSONObject: row)
    try FileHandle.standardOutput.write(contentsOf: data + Data([10]))
}
func readExactly(_ count: Int) throws -> Data? {
    var data = Data()
    while data.count < count {
        guard let chunk = try FileHandle.standardInput.read(upToCount: count - data.count), !chunk.isEmpty else {
            if data.isEmpty { return nil }
            throw NSError(domain: "Truncated PCM frame", code: 1)
        }
        data.append(chunk)
    }
    return data
}
guard CommandLine.arguments.count == 2 else {
    throw NSError(domain: "Usage: gretchen-parakeet MODEL_DIRECTORY", code: 1)
}
// The app owns pinned downloads and verification; the worker must never fetch weights.
let models = try AsrModels.loadLocal(from: URL(fileURLWithPath: CommandLine.arguments[1]), version: .v2)
let manager = AsrManager()
try await manager.loadModels(models)
try emit(["ready": true, "protocol": 1])
while let header = try readExactly(4) {
    let count = header.withUnsafeBytes { Int(UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self))) }
    guard count > 0, count <= 16000 * 600, let data = try readExactly(count * 4) else {
        throw NSError(domain: "Invalid PCM frame (limit 10 minutes)", code: 1)
    }
    let samples: [Float] = data.withUnsafeBytes { bytes in
        stride(from: 0, to: bytes.count, by: 4).map {
            Float(bitPattern: UInt32(littleEndian: bytes.loadUnaligned(fromByteOffset: $0, as: UInt32.self)))
        }
    }
    do {
        var state = try TdtDecoderState()
        let result = try await manager.transcribe(samples, decoderState: &state)
        try emit(["text": result.text])
    } catch {
        try emit(["error": String(describing: error)])
    }
}
