# herdr-plugin-naming

A [herdr](https://herdr.dev) plugin that renames an auto-generated worktree
branch and its workspace from the coding agent's first prompt.

When you spin up a herdr worktree it gets a throwaway name like
`worktree-silver-field-3fd7` (branch `worktree/silver-field-3fd7`). This plugin
watches for the agent's first real prompt, asks a headless Codex call to
summarize it as a short slug, then renames:

- the git branch to `wyattjoh/<slug>`, and
- the herdr workspace to `<slug>`.

It is agent-agnostic: it reads the first prompt from either a Claude Code or a
Codex transcript.

## How it works

The plugin is a single Rust binary invoked on `pane.agent_status_changed`. That
event fires constantly, so the binary is built around a near-zero-cost bail:

1. **Hot path** (every event, env vars only, no subprocess/socket): proceed only
   if the new status is `working` AND the pane is a linked herdr worktree whose
   label still starts with `worktree-`. Otherwise exit immediately. This gate is
   self-idempotent, since a successful rename changes the label.
2. **Cold path** (forked, detached): poll `herdr pane get` for the native
   session id (handling the documented status/session timing race), resolve and
   parse the transcript for the first genuine user prompt, generate the slug via
   Codex (with a deterministic local fallback), then rename the branch and
   workspace.

A per-workspace claim marker in the plugin state dir prevents concurrent
`working` events from launching more than one Codex call.

## Requirements

- herdr 0.7.0+
- A herdr agent integration installed for whichever agent you use, so that
  `agent_session` is populated:

  ```sh
  herdr integration install claude   # and/or
  herdr integration install codex
  ```

- The `codex` CLI on `PATH`, logged in (`codex login status`). Override the
  binary with `HERDR_NAMING_CODEX_BIN` if it is not on the hook's `PATH`. If
  Codex is unavailable the plugin falls back to a slug derived locally from the
  prompt.
- macOS (uses `setsid` for detachment; trivially portable to Linux).

## Install

```sh
herdr plugin install wyattjoh/herdr-plugin-naming
```

This runs `cargo build --release` as a build step before registering.

## Local development

`herdr plugin link` does NOT run the `[build]` step, so build manually first:

```sh
cargo build --release
herdr plugin link .
herdr plugin log list --plugin herdr-plugin-naming   # diagnostics
herdr plugin unlink herdr-plugin-naming
```

## Behavior notes

- Only auto-generated worktrees are ever touched (`worktree-` label prefix).
  Workspaces or branches you have already named are left alone.
- The branch rename is local only; it never pushes or touches the remote.
- Every code path exits 0 so the hook can never block or spam herdr.
