# CLAUDE.md

herdr plugin (Rust) that renames an auto-generated herdr worktree branch and
workspace from the coding agent's first prompt, using a headless Codex call as
the naming engine.

## Architecture

Single binary, two phases (`src/main.rs`):

- **Hot phase** (default, every `pane.agent_status_changed` event): pure env-var
  reads, no I/O. `context::evaluate` bails unless the new status is `working` and
  the pane is a linked worktree whose `workspace_label` still starts with
  `worktree-`. On a pass, writes a per-workspace claim marker and forks the cold
  phase detached (`setsid`).
- **Cold phase** (`HERDR_NAMING_PHASE=cold`): `herdr::poll_agent_session` →
  `transcript::read_first_prompt` → `codex::generate_slug` (fallback
  `slug::fallback_from_prompt`) → `git::rename_current_branch` to
  `wyattjoh/<slug>` → `herdr::workspace_rename` to `<slug>`.

Naming outputs: branch `wyattjoh/<slug>`, workspace `<slug>` (bare kebab).

## Module map

- `context.rs` — parse the two env JSON blobs, eligibility gate
- `slug.rs` — `sanitize` + `fallback_from_prompt`
- `transcript.rs` — resolve transcript path (glob) + first-prompt extraction for
  `claude` and `codex` (different on-disk formats)
- `codex.rs` — `codex exec --ignore-user-config --ephemeral -s read-only` with a 30s timeout
- `herdr.rs` — `herdr pane get` (polled) + `herdr workspace rename`
- `git.rs` — current branch + `git branch -m`

## Conventions

- Fail open: every path exits 0; never block herdr.
- Self-idempotent: a successful rename changes the label, so the gate bails after.
- Claim marker keyed on `workspace_id` in `HERDR_PLUGIN_STATE_DIR`, with a 120s
  staleness TTL; removed on a transient cold-phase miss so a later event retries.
- Pure logic (context/slug/transcript) is unit-tested; IO edges are
  integration-tested via `herdr plugin link` + `herdr plugin log list`.

## Key facts (verified against herdr 0.7.1, codex-cli 0.142.4)

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
cargo test                 # unit tests
cargo build --release      # produces target/release/herdr-plugin-naming
cargo fmt                  # format
```
