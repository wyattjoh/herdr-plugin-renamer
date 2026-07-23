# herdr-plugin-renamer

A [herdr](https://herdr.dev) plugin that names panes from a coding agent's first
prompt. In auto-generated linked worktrees, it also renames the git branch and
workspace.

It supports Claude Code, Codex, and Pi. Slugs come from Apple FoundationModels on
supported Macs, then Pi's configured model, then Codex. Automatic naming has a
local fallback; manual `/rename` requires a working naming model.

## Install

```sh
herdr plugin install wyattjoh/herdr-plugin-renamer
```

Plugins are session-scoped. Repeat the install for every named Herdr session:

```sh
herdr --session agents plugin install wyattjoh/herdr-plugin-renamer
```

Install the herdr integration for each agent you use:

```sh
herdr integration install claude
herdr integration install codex
herdr integration install pi
```

To add the Pi `/rename` command:

```sh
pi install git:github.com/wyattjoh/herdr-plugin-renamer
```

## Manual rename

Run `/rename` in Pi to generate a new name from the first user prompt plus up
to three latest user prompts. It updates the Pi session and Herdr pane, and also
updates the workspace and branch
when the branch was created or previously renamed by this plugin.

The same Herdr action can be invoked directly:

```sh
herdr plugin action invoke rename-current
```

## Requirements

- herdr 0.7.4+ on macOS or Linux
- For on-device naming: macOS 26+ on Apple Silicon with Apple Intelligence
  enabled
- For Pi naming: the `pi` CLI on `PATH` and authenticated
- Optional final fallback: the `codex` CLI on `PATH` and logged in

Without a naming model, automatic naming derives a rough local slug from the
prompt. Manual `/rename` reports the authentication error instead of pretending
that fallback was model-generated.

## What it renames

A prompt about reviewing a cache might rename the pane to `cache-review`. In an
auto-generated linked worktree, the plugin can also rename:

- branch: `<prefix>/cache-review`, or `cache-review` without a prefix
- workspace: `cache-review`

If the generated branch already exists, all names use the first free numeric
suffix, such as `cache-review-2`.

Automatic branch and workspace renaming only happens when the current branch
starts with `worktree/`. Manual `/rename` can rename that branch again after the
plugin has recorded it as managed. Branch renames are local and never push to
the remote.

The generated name is also published as a `task` metadata token on the pane and
workspace. This makes `$task` available to custom Agent and Space sidebar rows.
For example:

```toml
[ui.sidebar.agents]
rows = [["state_icon", "agent"], ["$task"]]

[ui.sidebar.spaces]
rows = [["workspace"], ["$task"]]
```

## Configuration

All settings are optional.

| Setting                       | Default        | Purpose                                      |
| ----------------------------- | -------------- | -------------------------------------------- |
| `HERDR_NAMING_ENGINE`         | `auto`         | Automatic chain, or only `pi` / `codex`      |
| `HERDR_NAMING_BRANCH_PREFIX`  | none           | Prefix renamed branches, such as `wyattjoh`  |
| `HERDR_NAMING_FOUNDATION_BIN` | bundled helper | Override the FoundationModels helper path    |
| `HERDR_NAMING_PI_BIN`         | `pi`           | Override the Pi executable path               |
| `HERDR_NAMING_CODEX_BIN`      | `codex`        | Override the Codex executable path           |

To configure a persistent branch prefix:

```sh
echo wyattjoh > "$(herdr plugin config-dir herdr-plugin-renamer)/branch-prefix"
```

`HERDR_NAMING_BRANCH_PREFIX` takes precedence over that file. Environment
variables must be available wherever herdr is launched.

## Local development

```sh
just build
just link
```

`herdr plugin link` does not run build steps, so use `just link` or build first.
See [CONTRIBUTING.md](CONTRIBUTING.md) for the complete development and test
workflow.

## License

[MIT](LICENSE)
