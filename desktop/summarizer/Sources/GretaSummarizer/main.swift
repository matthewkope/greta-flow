// greta-summarizer: local MLX meeting summarizer.
//
// Protocol (stdin/stdout, repeated): 4-byte little-endian u32 length + UTF-8 JSON.
// Request:  {"model_dir": string, "segments": [{"index","start_ms","text"}], "max_tokens": int}
// Response: {"ok": true, "output": {...}} or {"ok": false, "error": string}
//
// The model is loaded only from the local model_dir; nothing is downloaded.
// The helper does not validate grounding (segment_index/quote); Rust does that.
// Logs go to stderr only.

import Foundation
import MLXLLM
import MLXLMCommon
import Tokenizers

func log(_ message: String) {
    FileHandle.standardError.write(Data("greta-summarizer: \(message)\n".utf8))
}

// MARK: - Framing

func readExactly(_ count: Int) -> Data? {
    var data = Data()
    while data.count < count {
        let chunk = FileHandle.standardInput.readData(ofLength: count - data.count)
        if chunk.isEmpty { return nil }
        data.append(chunk)
    }
    return data
}

func readFrame() -> Data? {
    guard let header = readExactly(4) else { return nil }
    let length = header.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self)) }
    guard length <= 64 * 1024 * 1024 else {
        log("frame too large: \(length)")
        return nil
    }
    return length == 0 ? Data() : readExactly(Int(length))
}

func writeFrame(_ payload: Data) {
    var length = UInt32(payload.count).littleEndian
    var frame = Data(bytes: &length, count: 4)
    frame.append(payload)
    FileHandle.standardOutput.write(frame)
}

func respond(_ object: [String: Any]) {
    do {
        writeFrame(try JSONSerialization.data(withJSONObject: object))
    } catch {
        writeFrame(Data(#"{"ok":false,"error":"failed to encode response"}"#.utf8))
    }
}

// MARK: - Request

struct Segment: Decodable {
    let index: Int
    let start_ms: Int
    let text: String
}

struct Request: Decodable {
    let model_dir: String
    let segments: [Segment]
    let max_tokens: Int
}

enum HelperError: LocalizedError {
    case message(String)
    var errorDescription: String? {
        switch self { case .message(let m): return m }
    }
}

// MARK: - Tokenizer bridge (swift-transformers -> MLXLMCommon)

struct TokenizerBridge: MLXLMCommon.Tokenizer {
    let upstream: any Tokenizers.Tokenizer

    func encode(text: String, addSpecialTokens: Bool) -> [Int] {
        upstream.encode(text: text, addSpecialTokens: addSpecialTokens)
    }
    func decode(tokenIds: [Int], skipSpecialTokens: Bool) -> String {
        upstream.decode(tokens: tokenIds, skipSpecialTokens: skipSpecialTokens)
    }
    func convertTokenToId(_ token: String) -> Int? { upstream.convertTokenToId(token) }
    func convertIdToToken(_ id: Int) -> String? { upstream.convertIdToToken(id) }
    var bosToken: String? { upstream.bosToken }
    var eosToken: String? { upstream.eosToken }
    var unknownToken: String? { upstream.unknownToken }

    func applyChatTemplate(
        messages: [[String: any Sendable]],
        tools: [[String: any Sendable]]?,
        additionalContext: [String: any Sendable]?
    ) throws -> [Int] {
        do {
            return try upstream.applyChatTemplate(
                messages: messages, tools: tools, additionalContext: additionalContext)
        } catch Tokenizers.TokenizerError.missingChatTemplate {
            throw MLXLMCommon.TokenizerError.missingChatTemplate
        }
    }
}

struct LocalTokenizerLoader: TokenizerLoader {
    func load(from directory: URL) async throws -> any MLXLMCommon.Tokenizer {
        TokenizerBridge(upstream: try await AutoTokenizer.from(modelFolder: directory))
    }
}

// MARK: - Model cache

actor ModelCache {
    private var directory: String?
    private var container: ModelContainer?

    func container(for path: String) async throws -> ModelContainer {
        if let container, directory == path { return container }
        let url = URL(fileURLWithPath: path, isDirectory: true)
        var isDir: ObjCBool = false
        guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDir), isDir.boolValue
        else { throw HelperError.message("model_dir does not exist: \(path)") }
        log("loading model from \(url.path)")
        container = nil
        // Local directory load: no Downloader is involved, so nothing touches the network.
        let loaded = try await LLMModelFactory.shared.loadContainer(
            from: url, using: LocalTokenizerLoader())
        container = loaded
        directory = path
        log("model loaded")
        return loaded
    }
}

// MARK: - Prompt

let systemPrompt = """
    You summarize meeting transcripts. Reply with ONLY one JSON object and nothing else \
    (no prose, no markdown fences). Schema:
    {"bullets":[{"text":string,"segment_index":int,"quote":string}],\
    "action_items":[{"text":string,"owner":string,"segment_index":int,"quote":string}]}
    Rules:
    - Every item must cite exactly one segment by its index in "segment_index".
    - "quote" must be copied VERBATIM, character for character, from that segment's text. \
    Do not paraphrase, fix, or shorten words inside the quote.
    - "owner" is optional; include it only when the transcript names who will do the task.
    - Bullets are the key points, decisions and facts. Action items are concrete tasks.
    - If there is nothing to report, use empty arrays.
    """

func userPrompt(_ segments: [Segment]) -> String {
    var lines = ["Transcript segments:"]
    for s in segments {
        let seconds = s.start_ms / 1000
        let stamp = String(format: "%02d:%02d", seconds / 60, seconds % 60)
        lines.append("[\(s.index)] (\(stamp)) \(s.text)")
    }
    lines.append("")
    lines.append("Return only the JSON object.")
    return lines.joined(separator: "\n")
}

/// Returns the first balanced {...} object in `text`, respecting JSON strings.
func firstJSONObject(in text: String) -> String? {
    let chars = Array(text)
    guard let start = chars.firstIndex(of: "{") else { return nil }
    var depth = 0
    var inString = false
    var escaped = false
    for i in start..<chars.count {
        let c = chars[i]
        if inString {
            if escaped { escaped = false } else if c == "\\" { escaped = true } else if c == "\"" { inString = false }
            continue
        }
        switch c {
        case "\"": inString = true
        case "{": depth += 1
        case "}":
            depth -= 1
            if depth == 0 { return String(chars[start...i]) }
        default: break
        }
    }
    return nil
}

func summarize(_ request: Request, cache: ModelCache) async throws -> Any {
    guard !request.segments.isEmpty else { throw HelperError.message("no segments") }
    let container = try await cache.container(for: request.model_dir)
    let input = UserInput(chat: [.system(systemPrompt), .user(userPrompt(request.segments))])
    let lmInput = try await container.prepare(input: input)
    let parameters = GenerateParameters(maxTokens: max(1, request.max_tokens), temperature: 0)
    var generated = ""
    for await event in try await container.generate(input: lmInput, parameters: parameters) {
        if case .chunk(let text) = event { generated += text }
    }
    guard let json = firstJSONObject(in: generated),
        let object = try? JSONSerialization.jsonObject(with: Data(json.utf8)),
        object is [String: Any]
    else {
        log("model output had no JSON object (\(generated.count) chars)")
        throw HelperError.message("model output did not contain a JSON object")
    }
    return object
}

// MARK: - Main loop

let cache = ModelCache()
while let frame = readFrame() {
    do {
        let request = try JSONDecoder().decode(Request.self, from: frame)
        let output = try await summarize(request, cache: cache)
        respond(["ok": true, "output": output])
    } catch {
        log("request failed: \(error)")
        respond(["ok": false, "error": error.localizedDescription])
    }
}
