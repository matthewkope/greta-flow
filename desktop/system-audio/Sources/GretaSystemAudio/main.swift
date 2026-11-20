// greta-system-audio: captures system output audio via a Core Audio process tap.
//
// Stdout protocol: every frame is a 4-byte little-endian u32 byte length + payload.
// The first frame is a JSON header: {"sample_rate": N} or {"error": "..."} (then exit nonzero).
// Every following frame is raw little-endian f32 mono samples.
// Exits cleanly on stdin EOF or SIGTERM, destroying the aggregate device and the tap.
// Logs go to stderr only.

import AudioToolbox
import CoreAudio
import Foundation

func log(_ message: String) {
    FileHandle.standardError.write(Data("greta-system-audio: \(message)\n".utf8))
}

let stdoutLock = NSLock()

func writeFrame(_ payload: Data) {
    var length = UInt32(payload.count).littleEndian
    var frame = Data(bytes: &length, count: 4)
    frame.append(payload)
    stdoutLock.lock()
    defer { stdoutLock.unlock() }
    FileHandle.standardOutput.write(frame)
}

func writeJSON(_ object: [String: Any]) {
    if let data = try? JSONSerialization.data(withJSONObject: object) { writeFrame(data) }
}

func fail(_ message: String, code: Int32 = 1) -> Never {
    log(message)
    writeJSON(["error": message])
    exit(code)
}

// SIGPIPE would kill us silently if the parent closes stdout; treat it as a write error instead.
signal(SIGPIPE, SIG_IGN)


@available(macOS 14.2, *)
final class SystemAudioCapture {
    private var tapID = AudioObjectID(kAudioObjectUnknown)
    private var aggregateID = AudioObjectID(kAudioObjectUnknown)
    private var procID: AudioDeviceIOProcID?
    private let queue = DispatchQueue(label: "greta.system-audio.io")
    private(set) var sampleRate: Double = 0

    func start() throws {
        let description = CATapDescription(stereoGlobalTapButExcludeProcesses: [])
        description.uuid = UUID()
        description.name = "Greta Flow system audio"
        description.isPrivate = true
        description.muteBehavior = .unmuted

        var tap = AudioObjectID(kAudioObjectUnknown)
        try check(AudioHardwareCreateProcessTap(description, &tap), "AudioHardwareCreateProcessTap")
        tapID = tap

        var format = AudioStreamBasicDescription()
        var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioTapPropertyFormat, mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain)
        try check(AudioObjectGetPropertyData(tapID, &address, 0, nil, &size, &format), "read tap format")
        guard format.mFormatID == kAudioFormatLinearPCM,
            format.mFormatFlags & kAudioFormatFlagIsFloat != 0,
            format.mBitsPerChannel == 32
        else { throw CaptureError("unsupported tap format") }
        sampleRate = format.mSampleRate

        let aggregate: [String: Any] = [
            kAudioAggregateDeviceNameKey: "Greta Flow System Audio",
            kAudioAggregateDeviceUIDKey: "com.matthewkope.gretchenflow.system-audio.\(UUID().uuidString)",
            kAudioAggregateDeviceIsPrivateKey: true,
            kAudioAggregateDeviceIsStackedKey: false,
            kAudioAggregateDeviceTapAutoStartKey: true,
            kAudioAggregateDeviceTapListKey: [
                [
                    kAudioSubTapUIDKey: description.uuid.uuidString,
                    kAudioSubTapDriftCompensationKey: true,
                ]
            ],
        ]
        var aggregateDevice = AudioObjectID(kAudioObjectUnknown)
        try check(
            AudioHardwareCreateAggregateDevice(aggregate as CFDictionary, &aggregateDevice),
            "AudioHardwareCreateAggregateDevice")
        aggregateID = aggregateDevice

        var newProcID: AudioDeviceIOProcID?
        try check(
            AudioDeviceCreateIOProcIDWithBlock(&newProcID, aggregateID, queue) {
                _, inputData, _, _, _ in
                SystemAudioCapture.deliver(inputData)
            }, "AudioDeviceCreateIOProcIDWithBlock")
        procID = newProcID
        try check(AudioDeviceStart(aggregateID, procID), "AudioDeviceStart")
    }

    /// Downmix interleaved or non-interleaved f32 buffers to mono and emit one frame.
    private static func deliver(_ inputData: UnsafePointer<AudioBufferList>) {
        let buffers = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: inputData))
        guard let first = buffers.first, first.mDataByteSize > 0 else { return }
        var mono: [Float]
        if buffers.count == 1 {
            let stride = max(1, Int(first.mNumberChannels))
            let total = Int(first.mDataByteSize) / MemoryLayout<Float>.size
            let frames = total / stride
            guard frames > 0, let raw = first.mData else { return }
            let samples = raw.assumingMemoryBound(to: Float.self)
            mono = [Float](repeating: 0, count: frames)
            let scale = 1 / Float(stride)
            for f in 0..<frames {
                var sum: Float = 0
                for c in 0..<stride { sum += samples[f * stride + c] }
                mono[f] = sum * scale
            }
        } else {
            let frames = Int(first.mDataByteSize) / MemoryLayout<Float>.size
            guard frames > 0 else { return }
            mono = [Float](repeating: 0, count: frames)
            var used = 0
            for buffer in buffers {
                guard let raw = buffer.mData,
                    Int(buffer.mDataByteSize) / MemoryLayout<Float>.size >= frames
                else { continue }
                let samples = raw.assumingMemoryBound(to: Float.self)
                for f in 0..<frames { mono[f] += samples[f] }
                used += 1
            }
            guard used > 0 else { return }
            let scale = 1 / Float(used)
            for f in 0..<frames { mono[f] *= scale }
        }
        // Apple Silicon and Intel are little-endian; Float storage is already LE.
        let payload = mono.withUnsafeBufferPointer { Data(buffer: $0) }
        writeFrame(payload)
    }

    func stop() {
        if aggregateID != kAudioObjectUnknown {
            if let procID {
                AudioDeviceStop(aggregateID, procID)
                AudioDeviceDestroyIOProcID(aggregateID, procID)
            }
            procID = nil
            AudioHardwareDestroyAggregateDevice(aggregateID)
            aggregateID = AudioObjectID(kAudioObjectUnknown)
        }
        if tapID != kAudioObjectUnknown {
            AudioHardwareDestroyProcessTap(tapID)
            tapID = AudioObjectID(kAudioObjectUnknown)
        }
    }

    private func check(_ status: OSStatus, _ what: String) throws {
        guard status == noErr else { throw CaptureError("\(what) failed (OSStatus \(status))") }
    }
}

struct CaptureError: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}

@available(macOS 14.2, *)
func run() -> Never {
    let capture = SystemAudioCapture()
    let shutdownLock = NSLock()
    nonisolated(unsafe) var didShutdown = false

    @Sendable func shutdown(_ code: Int32) -> Never {
        shutdownLock.lock()
        if !didShutdown {
            didShutdown = true
            capture.stop()
            log("stopped")
        }
        shutdownLock.unlock()
        exit(code)
    }

    do {
        try capture.start()
    } catch {
        capture.stop()
        fail("\(error)")
    }
    writeJSON(["sample_rate": Int(capture.sampleRate.rounded())])
    log("capturing at \(capture.sampleRate) Hz")

    signal(SIGTERM, SIG_IGN)
    signal(SIGINT, SIG_IGN)
    let termSource = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
    termSource.setEventHandler { shutdown(0) }
    termSource.resume()
    let intSource = DispatchSource.makeSignalSource(signal: SIGINT, queue: .main)
    intSource.setEventHandler { shutdown(0) }
    intSource.resume()

    // Stdin EOF means the parent is gone or asked us to stop.
    Thread.detachNewThread {
        while !FileHandle.standardInput.availableData.isEmpty {}
        DispatchQueue.main.async { shutdown(0) }
    }

    withExtendedLifetime((termSource, intSource)) { dispatchMain() }
}

if #available(macOS 14.2, *) {
    run()
} else {
    fail("System audio capture requires macOS 14.2 or later", code: 2)
}
