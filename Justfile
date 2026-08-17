plugin_id := "herdr-plugin-renamer"

default:
    @just --list

# Build local artifacts without changing herdr's plugin registry.
build:
    cargo build --release
    if [ "$(uname -s)" = "Darwin" ]; then \
        sh naming-helper/build-if-supported.sh; \
    fi
# Build, then register this checkout with herdr.
link: build
    herdr plugin unlink {{plugin_id}} >/dev/null 2>&1 || true
    herdr plugin link .
