// herdr-namer: the on-device naming engine.
//
// Reads a coding-task prompt (from argv, else stdin), asks Apple's on-device
// FoundationModels for a short kebab-case slug, and prints that candidate to
// stdout. The Rust plugin invokes this exactly like it invokes `codex`:
//   - success: a bare candidate string on stdout, exit 0
//   - any failure (model unavailable, empty prompt, generation error):
//     nothing on stdout, a short reason on stderr, non-zero exit
// so the caller's existing fallback (Codex, then a local slug) just works.
//
// The output is only a rough candidate; the Rust side runs it through
// `slug::sanitize`, so this stays deliberately dumb.

import Foundation
import FoundationModels

// Keep parity with the Rust PROMPT_LIMIT so neither engine sees a giant prompt.
let promptLimit = 2000

func fail(_ message: String, code: Int32) -> Never {
    FileHandle.standardError.write(Data("\(message)\n".utf8))
    exit(code)
}

func readPrompt() -> String {
    let args = Array(CommandLine.arguments.dropFirst())
    if !args.isEmpty {
        return args.joined(separator: " ")
    }
    let data = FileHandle.standardInput.readDataToEndOfFile()
    return String(data: data, encoding: .utf8) ?? ""
}

@main
struct HerdrNamer {
    static func main() async {
        let prompt = readPrompt().trimmingCharacters(in: .whitespacesAndNewlines)
        guard !prompt.isEmpty else {
            fail("empty prompt", code: 2)
        }

        let model = SystemLanguageModel.default
        guard case .available = model.availability else {
            var reason = "unknown"
            if case .unavailable(let unavailable) = model.availability {
                reason = "\(unavailable)"
            }
            fail("model unavailable: \(reason)", code: 1)
        }

        let instructions = """
        You name software tasks. Reply with ONLY a short kebab-case slug \
        (2 to 4 words, lowercase, hyphen-separated, no quotes, no surrounding \
        text) summarizing the coding task. No explanation.
        """
        let session = LanguageModelSession(instructions: instructions)
        let capped = String(prompt.prefix(promptLimit))
        // Greedy sampling makes the slug deterministic for a given prompt; the
        // small token cap bounds latency (a slug is only a handful of tokens).
        let options = GenerationOptions(samplingMode: .greedy, maximumResponseTokens: 16)

        do {
            let response = try await session.respond(to: capped, options: options)
            print(response.content)
        } catch {
            fail("generation error: \(error)", code: 3)
        }
    }
}
