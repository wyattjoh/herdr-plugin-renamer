# herdr-plugin-renamer

A [herdr](https://herdr.dev) plugin that renames numeric tabs from a coding
agent's first prompt. When that prompt happens in an auto-generated linked
worktree, it also renames the git branch and workspace.

When you start an agent in a numbered herdr tab like `1`, this plugin watches
for the agent's first real prompt, asks a language model to summarize it as a
short kebab-case slug, then renames the tab to that slug.

If the pane is also in a herdr linked worktree with a branch like
`worktree/silver-field-3fd7`, the plugin additionally renames:

- the git branch to `<prefix>/<slug>` (or just `<slug>` with no prefix), then
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
   if the new status is `working` and this tab has not already been processed.
   A tab-scoped claim marker prevents duplicate cold phases while the first one
   is still running.
2. **Cold path** (forked, detached): poll `herdr pane get` for the native
   session id (handling the documented status/session timing race), resolve and
   parse the transcript for the first genuine user prompt, generate the slug via
   the engine chain, rename the tab if its current label is numeric, then maybe
   rename the branch and workspace.

A permanent tab-scoped done marker in the plugin state dir enforces the "first
prompt" rule. Transient misses remove the claim marker so a later event can
retry.

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
`herdr plugin config-dir herdr-plugin-renamer`).

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
  echo yourname > "$(herdr plugin config-dir herdr-plugin-renamer)/branch-prefix"
  ```

## Install

```sh
herdr plugin install wyattjoh/herdr-plugin-renamer
```

On install, herdr runs the build steps: `cargo build --release` always, and
`swift build -c release` for the on-device helper on macOS only.

## Local development

`herdr plugin link` does NOT run the build steps, so build manually first:

```sh
cargo build --release
swift build -c release --package-path naming-helper   # macOS only, for on-device
herdr plugin link .
herdr plugin log list --plugin herdr-plugin-renamer    # diagnostics
herdr plugin unlink herdr-plugin-renamer
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full test/lint loop.

## Behavior notes

- Numeric tab labels are renamed from the first prompt in any checkout.
- Branch/workspace renaming only runs in linked worktrees whose current branch
  still starts with `worktree/`.
- Workspace renaming only runs after the branch rename succeeds.
- The branch rename is local only; it never pushes or touches the remote.
- Every code path exits 0 so the hook can never block or spam herdr.

## License

[MIT](LICENSE)
