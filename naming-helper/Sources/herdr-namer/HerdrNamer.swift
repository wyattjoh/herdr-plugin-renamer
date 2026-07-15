// herdr-namer: the on-device naming engine.
//
// Reads a coding-task prompt (from argv, else stdin), asks Apple's on-device
// FoundationModels for several short kebab-case candidates, asks the model to
// select the strongest candidate, and prints that selected slug to stdout. The
// Rust plugin invokes this exactly like it invokes `codex`:
//   - success: a bare candidate string on stdout, exit 0
//   - any failure (model unavailable, empty prompt, generation error):
//     nothing on stdout, a short reason on stderr, non-zero exit
// so the caller's existing fallback (Codex, then a local slug) just works.
//
// The Rust side still runs stdout through `slug::sanitize`, so this helper can
// fail open and keep the plugin boundary simple.

import Foundation
import FoundationModels

// Keep parity with the Rust Foundation prompt excerpt so neither path sends a
// giant prompt while still preserving the final instruction in long prompts.
let promptHeadLimit = 1200
let promptTailLimit = 1200

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

func promptExcerpt(_ prompt: String) -> String {
    if prompt.count <= promptHeadLimit + promptTailLimit {
        return prompt
    }

    let headEnd = prompt.index(prompt.startIndex, offsetBy: promptHeadLimit)
    let tailStart = prompt.index(prompt.endIndex, offsetBy: -promptTailLimit)
    let head = String(prompt[..<headEnd])
    let tail = String(prompt[tailStart...])
    return "\(head)\n\n[... middle omitted for naming ...]\n\n\(tail)"
}

func sanitize(_ raw: String) -> String {
    var output = ""
    var previousDash = true

    for scalar in raw.unicodeScalars {
        if scalar.value >= 48 && scalar.value <= 57 {
            output.unicodeScalars.append(scalar)
            previousDash = false
        } else if scalar.value >= 65 && scalar.value <= 90 {
            output.unicodeScalars.append(UnicodeScalar(scalar.value + 32)!)
            previousDash = false
        } else if scalar.value >= 97 && scalar.value <= 122 {
            output.unicodeScalars.append(scalar)
            previousDash = false
        } else if !previousDash {
            output.append("-")
            previousDash = true
        }
    }

    while output.last == "-" {
        output.removeLast()
    }

    let words = output.split(separator: "-").prefix(6)
    var capped = words.joined(separator: "-")
    if capped.count > 50 {
        let end = capped.index(capped.startIndex, offsetBy: 50)
        capped = String(capped[..<end])
        while capped.last == "-" {
            capped.removeLast()
        }
    }

    return capped
}

func cleanCandidates(_ rawCandidates: [String]) -> [String] {
    var seen = Set<String>()
    var cleaned: [String] = []

    for raw in rawCandidates {
        let slug = sanitize(raw)
        guard !slug.isEmpty else {
            continue
        }
        let parts = slug.split(separator: "-")
        guard parts.last?.allSatisfy(\.isNumber) == false else {
            continue
        }
        guard !seen.contains(slug) else {
            continue
        }

        seen.insert(slug)
        cleaned.append(slug)
    }

    return cleaned
}

func debugLog(_ message: String) {
    guard ProcessInfo.processInfo.environment["HERDR_NAMER_DEBUG"] == "1" else {
        return
    }
    FileHandle.standardError.write(Data("\(message)\n".utf8))
}

func greedyOptions(maximumResponseTokens: Int) -> GenerationOptions {
#if compiler(>=6.4)
    GenerationOptions(
        samplingMode: .greedy,
        maximumResponseTokens: maximumResponseTokens)
#else
    GenerationOptions(
        sampling: .greedy,
        maximumResponseTokens: maximumResponseTokens)
#endif
}

// Guided-generation schemas. Asking the model to fill typed fields via
// `respond(to:generating:)` puts the decoder in constrained mode, so the model
// emits structured values instead of free-form prose. Without this, a chatty
// reply like "Sure, here are some ideas for..." would sanitize into a bogus
// branch name. The Rust side still sanitizes the final stdout value.
@Generable
struct TaskNameCandidates {
    @Guide(
        description: "The best compact noun-topic kebab-case slug, 1 to 3 "
            + "words, all lowercase, hyphen-separated. Name the user's goal or "
            + "task topic, not the full instruction."
    )
    let primary: String

    @Guide(
        description: "A compact kebab-case slug focused on the main artifact, "
            + "file, UI surface, command, subsystem, or code object in the "
            + "prompt. Use 1 to 3 words."
    )
    let artifact: String

    @Guide(
        description: "A compact kebab-case slug focused on the desired outcome "
            + "or resulting state. Use 1 to 3 words."
    )
    let outcome: String

    @Guide(
        description: "A compact kebab-case slug with enough context to avoid a "
            + "generic label. Use 1 to 3 words."
    )
    let contextual: String

    @Guide(
        description: "The shortest useful compact kebab-case slug that still "
            + "names the task. Use 1 to 3 words."
    )
    let concise: String

    @Guide(
        description: "A distinct alternate compact kebab-case slug. Do not make "
            + "a numbered variant of another candidate. Use 1 to 3 words."
    )
    let alternate: String

    var all: [String] {
        [primary, artifact, outcome, contextual, concise, alternate]
    }
}

@Generable
struct SelectedTaskName {
    @Guide(
        description: "Exactly one kebab-case slug copied from the candidate "
            + "list. Copy the selected candidate exactly. Do not invent a new "
            + "slug, explain the choice, add quotes, or add surrounding text."
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

        let generatorInstructions = """
        You name software tasks for panes, workspaces, and git branches.
        Produce compact noun-topic label candidates, not literal restatements.

        Ground the label in the user's actual prompt. Prefer nouns and domain
        terms from that prompt, or direct synonyms.
        Drop generic request verbs and filler words.
        For change requests that move something "to" a target state, name the
        resulting topic or target state instead of the whole command phrase.
        If the prompt means "change selected file to current", include
        current-file and avoid source-to-target labels like file-to-current.
        Prefer adjective-noun order for other state labels too.
        Include enough noun context to avoid generic labels, such as
        branch-commits instead of commits. If the prompt asks about commits on
        this branch, include branch-commits and avoid commit-log unless the
        prompt specifically asks for a log file.
        Avoid prepositions in the label.
        Do not introduce absent concepts, feature names, protocols, or examples.
        Keep labels short enough to scan as pane labels.
        Favor the user's goal over implementation details. If a prompt mentions
        both a mechanism and a desired user-visible outcome, name the outcome
        unless the mechanism is the actual task topic.
        For prompts about improving title or slug quality with multiple
        generated options, prefer outcome labels like title-quality,
        slug-options, or naming-quality over implementation-heavy labels.
        Generate distinct candidates with different useful angles. Never pad by
        adding numeric suffixes.
        """
        let generatorSession = LanguageModelSession(instructions: generatorInstructions)
        let capped = promptExcerpt(prompt)
        // Greedy sampling makes naming deterministic for a given prompt. The
        // candidate token cap must leave headroom for a JSON object envelope
        // plus several short slugs, else constrained decoding can throw on a
        // truncated object.
        // Use `samplingMode:` with Xcode 27 and `sampling:` with Xcode 26. This
        // keeps strict warning builds clean without dropping the older SDK.
        let generatorOptions = greedyOptions(maximumResponseTokens: 160)
        let judgeOptions = greedyOptions(maximumResponseTokens: 64)

        do {
            let candidatesResponse = try await generatorSession.respond(
                to: capped, generating: TaskNameCandidates.self, options: generatorOptions)
            let candidates = cleanCandidates(candidatesResponse.content.all)
            debugLog("candidates: \(candidates.joined(separator: ", "))")
            guard !candidates.isEmpty else {
                fail("generation error: no usable candidates", code: 3)
            }

            let judgeInstructions = """
            You select the best software task name for a pane, workspace, and git
            branch. Choose exactly one candidate from the provided list.

            Selection criteria, in order:
            1. Grounded in the user's actual prompt.
            2. Specific enough to distinguish the task from nearby work.
            3. Names the topic, artifact, subsystem, or resulting state.
            4. Avoids generic task verbs and filler words.
            5. Avoids overly literal command summaries and implementation-only
               details unless they are the main subject of the prompt.
            6. Avoids numbered variants or padded alternatives.
            7. Short enough to scan as a pane label.

            For "selected file to current" prompts, choose current-file over
            file-to-current because the resulting state is the meaningful topic.
            For prompts about commits on this branch, choose branch-commits over
            commits or commit-log because the branch context is meaningful and
            "log" adds a concept the prompt did not request.

            Copy the selected candidate exactly. Never invent a new slug.
            """
            let judgeSession = LanguageModelSession(instructions: judgeInstructions)
            let candidateLines = candidates.map { "- \($0)" }.joined(separator: "\n")
            let judgePrompt = """
            User prompt:
            \(capped)

            Candidate slugs:
            \(candidateLines)
            """

            let selectedResponse = try await judgeSession.respond(
                to: judgePrompt, generating: SelectedTaskName.self, options: judgeOptions)
            let selected = sanitize(selectedResponse.content.slug)
            debugLog("selected: \(selected)")
            if candidates.contains(selected) {
                print(selected)
            } else {
                debugLog("selected was not in candidates, falling back to first candidate")
                print(candidates[0])
            }
        } catch {
            fail("generation error: \(error)", code: 3)
        }
    }
}
