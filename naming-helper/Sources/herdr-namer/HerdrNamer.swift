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

// Guided-generation schema. Asking the model to fill a typed `slug` field (via
// `respond(to:generating:)`) puts the decoder in constrained mode, so the model
// emits the slug directly instead of free-form prose. Without this, a chatty
// reply like "Sure, here are some ideas for..." would sanitize into a bogus
// branch name. The Rust side still runs the value through `slug::sanitize`.
@Generable
struct TaskName {
    @Guide(
        description: "A short kebab-case slug, 2 to 4 words, all lowercase, "
            + "hyphen-separated, summarizing the coding task. "
            + "No quotes, no spaces, no surrounding text."
    )
    let slug: String
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
        You name software tasks. Summarize the coding task as a short \
        kebab-case slug.
        """
        let session = LanguageModelSession(instructions: instructions)
        let capped = String(prompt.prefix(promptLimit))
        // Greedy sampling makes the slug deterministic for a given prompt. The
        // token cap bounds latency but must leave headroom for the JSON envelope
        // around the slug (`{"slug":"..."}`): a truncated object fails to parse
        // and throws, so keep this comfortably above a 4-word slug plus braces.
        // Use the `sampling:` label, not the newer `samplingMode:`: it exists on
        // both older macOS 26 SDKs (CI runners) and current ones (deprecated but
        // present), so the helper compiles across the SDK skew.
        let options = GenerationOptions(sampling: .greedy, maximumResponseTokens: 48)

        do {
            // Guided generation: the model fills `TaskName.slug` under
            // constrained decoding, so it cannot return conversational prose.
            let response = try await session.respond(
                to: capped, generating: TaskName.self, options: options)
            print(response.content.slug)
        } catch {
            fail("generation error: \(error)", code: 3)
        }
    }
}
