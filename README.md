# herdr-plugin-naming

A [herdr](https://herdr.dev) plugin that renames an auto-generated worktree
branch and its workspace from the coding agent's first prompt.

When you spin up a herdr worktree it gets a throwaway name like
`worktree-silver-field-3fd7` (branch `worktree/silver-field-3fd7`). This plugin
watches for the agent's first real prompt, asks a language model to summarize it
as a short kebab-case slug, then renames:

- the git branch to `<prefix>/<slug>` (or just `<slug>` with no prefix), and
- the herdr workspace to `<slug>`.

It is agent-agnostic: it reads the first prompt from either a Claude Code or a
Codex transcript.

## Naming engines

The slug is produced by a swappable engine, selected with `HERDR_NAMING_ENGINE`
and falling back automatically:

| Engine       | What it is                                                            | Availability      |
| ------------ | -------------------------------------------------------------------- | ----------------- |
| `foundation` | On-device Apple [FoundationModels](https://developer.apple.com/documentation/foundationmodels) via a small Swift helper. Fast, offline, no auth. | macOS 26+ with Apple Intelligence |
| `codex`      | A headless `codex exec` call.                                        | Any OS, needs the `codex` CLI |

- **Default** (`HERDR_NAMING_ENGINE` unset or `foundation`): try on-device first,
  then Codex, then a deterministic slug derived locally from the prompt.
- `HERDR_NAMING_ENGINE=codex`: skip the on-device engine entirely.
- **Off macOS** (e.g. Linux), the on-device engine is compiled out of the binary;
  the chain is Codex then the local fallback regardless of the setting.

So naming always succeeds: a real model when one is available, a local slug
otherwise.

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
   the engine chain, then rename the branch and workspace.

A per-workspace claim marker in the plugin state dir prevents concurrent
`working` events from launching more than one naming call.

## Requirements

- herdr 0.7.0+ on macOS or Linux.
- A herdr agent integration installed for whichever agent you use, so that
  `agent_session` is populated:

  ```sh
  herdr integration install claude   # and/or
  herdr integration install codex
  ```

- For the **on-device** engine: macOS 26 (Tahoe) or newer on Apple Silicon, with
  Apple Intelligence enabled. No app bundle, entitlement, or signing required.
- For the **Codex** engine / fallback: the `codex` CLI on `PATH`, logged in
  (`codex login status`).

If neither model is available, naming falls back to a slug derived locally from
the prompt, so the plugin still works (just with rougher names).

## Configuration

All settings are optional. herdr passes its own environment to hook commands, so
environment variables must be set wherever herdr is launched. For persistent
per-install settings that do not depend on the launch environment, use the
plugin's config dir instead (print it with
`herdr plugin config-dir herdr-plugin-naming`).

| Setting                        | Default        | Notes                                                                 |
| ------------------------------ | -------------- | --------------------------------------------------------------------- |
| `HERDR_NAMING_ENGINE`          | `foundation`   | `foundation` (on-device first) or `codex`.                            |
| `HERDR_NAMING_BRANCH_PREFIX`   | _none_         | Branch prefix. Empty = bare `<slug>`. Env overrides the config file.  |
| `HERDR_NAMING_FOUNDATION_BIN`  | bundled helper | Path to the `herdr-namer` Swift helper.                               |
| `HERDR_NAMING_CODEX_BIN`       | `codex`        | Path to the `codex` binary.                                           |

### Branch prefix

By default the branch is renamed to the bare `<slug>`. To prefix it (e.g.
`yourname/<slug>`), either:

- set `HERDR_NAMING_BRANCH_PREFIX=yourname` in the environment herdr is launched
  with, or
- write the prefix into the config dir (recommended for an installed plugin, no
  launch-env dependency):

  ```sh
  echo yourname > "$(herdr plugin config-dir herdr-plugin-naming)/branch-prefix"
  ```

## Install

```sh
herdr plugin install wyattjoh/herdr-plugin-naming
```

On install, herdr runs the build steps: `cargo build --release` always, and
`swift build -c release` for the on-device helper on macOS only.

## Local development

`herdr plugin link` does NOT run the build steps, so build manually first:

```sh
cargo build --release
swift build -c release --package-path naming-helper   # macOS only, for on-device
herdr plugin link .
herdr plugin log list --plugin herdr-plugin-naming    # diagnostics
herdr plugin unlink herdr-plugin-naming
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full test/lint loop.

## Behavior notes

- Only auto-generated worktrees are ever touched (`worktree-` label prefix).
  Workspaces or branches you have already named are left alone.
- The branch rename is local only; it never pushes or touches the remote.
- Every code path exits 0 so the hook can never block or spam herdr.

## License

[MIT](LICENSE)
