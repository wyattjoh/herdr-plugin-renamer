plugin_id := "herdr-plugin-renamer"

default:
    @just --list

# Build local artifacts without changing herdr's plugin registry.
build:
    cargo build --release
    if [ "$(uname -s)" = "Darwin" ]; then \
        swift build -c release --package-path naming-helper; \
    fi

# Build, then register this checkout with herdr.
link: build
    herdr plugin unlink {{plugin_id}} >/dev/null 2>&1 || true
    herdr plugin link .
