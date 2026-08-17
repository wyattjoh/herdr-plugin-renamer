#!/bin/sh
set -eu

swiftc_bin=${SWIFTC_BIN:-swiftc}
swift_bin=${SWIFT_BIN:-swift}
probe='import FoundationModels
@Generable struct FoundationModelsBuildProbe { @Guide(description: "probe") let value: String }'

if ! printf '%s\n' "$probe" | "$swiftc_bin" -typecheck - >/dev/null 2>&1; then
    echo "FoundationModels guided-generation macros are unavailable in this Swift toolchain; skipping optional on-device helper." >&2
    exit 0
fi

exec "$swift_bin" build -c release --package-path naming-helper
