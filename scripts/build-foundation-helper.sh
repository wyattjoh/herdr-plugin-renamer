#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
swiftc_bin=${SWIFTC_BIN:-swiftc}
swift_bin=${SWIFT_BIN:-swift}
probe_dir=$(mktemp -d "${TMPDIR:-/tmp}/herdr-foundation-probe.XXXXXX")
trap 'rm -rf "$probe_dir"' EXIT HUP INT TERM

cat >"$probe_dir/Probe.swift" <<'SWIFT'
import FoundationModels

@Generable
struct HerdrFoundationProbe {
    @Guide(description: "Probe FoundationModels guided-generation macro support")
    var slug: String
}
SWIFT

if ! "$swiftc_bin" -typecheck "$probe_dir/Probe.swift" >/dev/null 2>&1; then
    echo "FoundationModels guided-generation macros unavailable; skipping optional helper build"
    exit 0
fi

rm -rf "$probe_dir"
trap - EXIT HUP INT TERM
cd "$repo_root"
exec "$swift_bin" build -c release --package-path naming-helper
