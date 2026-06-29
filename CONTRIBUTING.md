# Contributing

Thanks for your interest. This is a small, single-purpose herdr plugin; the bar
is "keep it lean, fail open, and stay fast on the hot path."

## Build and test

```sh
cargo build --release          # the plugin binary
cargo test                     # pure-logic unit tests (fast)
cargo fmt                      # format (run before committing)
cargo clippy --all-targets     # lint
```

The on-device helper (macOS 26+ only):

```sh
swift build -c release --package-path naming-helper
cargo test foundation -- --ignored   # live end-to-end check (needs the helper
                                      # built and Apple Intelligence available)
```

### Linux / cross-platform

The on-device `foundation` engine is `#[cfg(target_os = "macos")]` and is
compiled out elsewhere. Verify the non-macOS build still type-checks:

```sh
rustup target add x86_64-unknown-linux-gnu
cargo check --target x86_64-unknown-linux-gnu --tests
```

CI runs fmt, clippy, tests, and a release build on both Ubuntu and macOS.

## Conventions

- **Fail open.** Every code path must exit 0 so the hook can never block or spam
  herdr. The hot path must stay allocation-light and do no I/O beyond env reads.
- **Test the pure logic.** Slug/engine/branch/context/transcript logic is unit
  tested; keep new pure logic covered. I/O edges are checked manually via
  `herdr plugin link` + `herdr plugin log list`.
- **Conventional Commits** for commit messages
  (`feat:`, `fix:`, `docs:`, `refactor:`, ...).
- No em dashes in code, comments, or commit messages.
- Architecture, module map, and verified herdr/codex/macOS facts live in
  [CLAUDE.md](CLAUDE.md). Update it when you change a referenced pattern.

## Reporting issues

Include your herdr version (`herdr --version`), OS, the agent integration in
use, and the relevant lines from
`herdr plugin log list --plugin herdr-plugin-renamer` (or the debug log in the
plugin state dir).
