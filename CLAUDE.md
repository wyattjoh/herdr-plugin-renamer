# CLAUDE.md

herdr plugin (Rust) that renames an auto-generated herdr worktree branch and
workspace from the coding agent's first prompt. The naming engine is swappable:
on-device Apple FoundationModels by default (a small Swift helper), with a
headless Codex call as the automatic fallback.

## Architecture

Single binary, two phases (`src/main.rs`):

- **Hot phase** (default, every `pane.agent_status_changed` event): pure env-var
  reads, no I/O. `context::evaluate` bails unless the new status is `working` and
  the pane is a linked worktree whose `workspace_label` still starts with
  `worktree-`. On a pass, writes a per-workspace claim marker and forks the cold
  phase detached (`setsid`).
- **Cold phase** (`HERDR_NAMING_PHASE=cold`): `herdr::poll_agent_session` →
  `transcript::read_first_prompt` → `main::generate_slug` (walks the
  `engine::engine_chain`; fallback `slug::fallback_from_prompt`) →
  `git::rename_current_branch` to `wyattjoh/<slug>` → `herdr::workspace_rename`
  to `<slug>`.

Naming outputs: branch `wyattjoh/<slug>`, workspace `<slug>` (bare kebab).

## Naming engines

`generate_slug` (in `main.rs`) walks an ordered chain from `engine::engine_chain`,
selected by `HERDR_NAMING_ENGINE`, and uses the first engine that returns a slug:

- unset / `foundation` / unknown → `[Foundation, Codex]` (on-device first)
- `codex` → `[Codex]` only

Each engine returns `Option<String>` and yields `None` on any failure, so the
chain degrades cleanly: Foundation → Codex → deterministic local slug. Engine
binaries are overridable via `HERDR_NAMING_FOUNDATION_BIN` and
`HERDR_NAMING_CODEX_BIN`.

**OS gate:** the `Foundation` engine is `#[cfg(target_os = "macos")]`. Off macOS
(e.g. Linux) the enum variant, the `foundation` module, and the matching
`[[build]]` swift step are all compiled/skipped, so the default chain collapses
to `[Codex]` and a `foundation` request is silently downgraded. The plugin's
`platforms` are `["macos", "linux"]` (Unix only; the cold phase detaches via
`setsid`). Verify the Linux build with
`cargo check --target x86_64-unknown-linux-gnu`.

## Module map

- `context.rs` — parse the two env JSON blobs, eligibility gate
- `slug.rs` — `sanitize` + `fallback_from_prompt`
- `engine.rs` — pure `engine_chain(HERDR_NAMING_ENGINE)` → ordered fallback list
  (OS-aware: Foundation only on macOS)
- `transcript.rs` — resolve transcript path (glob) + first-prompt extraction for
  `claude` and `codex` (different on-disk formats)
- `foundation.rs` — macOS-only (`#[cfg(target_os = "macos")]`) on-device engine;
  shells to the `herdr-namer` Swift helper (15s timeout), sanitizes its stdout
- `codex.rs` — `codex exec --ignore-user-config --ephemeral -s read-only` with a 30s timeout
- `herdr.rs` — `herdr pane get` (polled) + `herdr workspace rename`
- `git.rs` — current branch + `git branch -m`
- `naming-helper/` — SwiftPM package (`herdr-namer`): a `LanguageModelSession`
  prompt → bare slug on stdout (exit 0), or a reason on stderr (non-zero) when
  Apple Intelligence is unavailable. Same stdout-or-fail contract as `codex`.

## Conventions

- Fail open: every path exits 0; never block herdr.
- Self-idempotent: a successful rename changes the label, so the gate bails after.
- The cold phase polls for BOTH the session and the first prompt. Claude reports
  its session at SessionStart (before the prompt) and stays `working` with no new
  event, so a single transcript read can miss the prompt and never retry. Polling
  the prompt (not just the session) is what makes the Claude path reliable.
- Claim marker keyed on `workspace_id` in `HERDR_PLUGIN_STATE_DIR`, with a 120s
  staleness TTL; removed on a transient cold-phase miss so a later event retries.
- Pure logic (context/slug/transcript) is unit-tested; IO edges are
  integration-tested via `herdr plugin link` + `herdr plugin log list`.

## Key facts (verified against herdr 0.7.1, codex-cli 0.142.4, macOS 26.5)

- FoundationModels runs from a plain SwiftPM CLI: no app bundle, Info.plist,
  entitlement, or signing needed to invoke `LanguageModelSession` locally. The
  package floors `platforms` at `.macOS("26.0")` so symbols are reachable
  without per-call `@available`; runtime gating uses
  `SystemLanguageModel.default.availability` (`.deviceNotEligible` /
  `.appleIntelligenceNotEnabled` / `.modelNotReady`), reported as a non-zero
  exit so Rust falls back to Codex.
- The model lives behind a shared OS daemon, so the short-lived helper does not
  reload weights per spawn: warm ~0.3s, cold ~1-2s end-to-end. Both beat the
  Codex bar. Use `samplingMode: .greedy` for deterministic slugs; the file must
  not be named `main.swift` (conflicts with `@main`).

- herdr `[[events]]` has NO filter/once/debounce; the hook fires on every event.
- Branch detection needs no git call: `workspace_label` (`worktree-<adj>-<noun>-<hex4>`)
  maps to branch `worktree/<adj>-<noun>-<hex4>`; eligibility uses the env label.
- `agent_session` agent label is `claude` or `codex`; transcripts:
  Claude `~/.claude/projects/**/<uuid>.jsonl`, Codex
  `~/.codex/sessions/**/rollout-*<uuid>.jsonl`.
- `--ignore-user-config` on the Codex call disables the user's Codex hooks
  (avoids recursion and nondeterminism); auth still resolves from `CODEX_HOME`.
- Naming model is `gpt-5.5` + `model_reasoning_effort=low` (~2.5s). That is the
  fastest config available on ChatGPT-account auth: `minimal` effort is rejected
  because the `image_gen`/`web_search` tools can't be disabled, and the faster
  `spark`/`flash`/`*-mini` models are API-key-only. `--ephemeral` keeps these
  throwaway runs out of `~/.codex/sessions`.

## Commands

```sh
cargo test                 # unit tests (engine/slug/context/transcript)
cargo test foundation -- --ignored   # live on-device helper check (needs the
                                      # Swift build + Apple Intelligence)
cargo build --release      # produces target/release/herdr-plugin-naming
cargo fmt                  # format

# On-device naming helper (built by the second [[build]] step on install):
swift build -c release --package-path naming-helper   # -> naming-helper/.build/release/herdr-namer
```
