#!/bin/sh
set -eu

helper_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
wrapper="$helper_dir/build-if-supported.sh"
test_dir=$(mktemp -d "${TMPDIR:-/tmp}/herdr-foundation-build-test.XXXXXX")
trap 'rm -rf "$test_dir"' EXIT HUP INT TERM

cat >"$test_dir/swiftc" <<'SH'
#!/bin/sh
if [ "${FAKE_PROBE_RESULT:-failure}" = "success" ]; then
    exit 0
fi
exit 65
SH

cat >"$test_dir/swift" <<'SH'
#!/bin/sh
printf '%s\n' "$*" >"$FAKE_SWIFT_LOG"
exit "${FAKE_BUILD_STATUS:-0}"
SH
chmod +x "$test_dir/swiftc" "$test_dir/swift"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

swift_log="$test_dir/swift.log"

FAKE_PROBE_RESULT=failure \
FAKE_SWIFT_LOG="$swift_log" \
SWIFTC_BIN="$test_dir/swiftc" \
SWIFT_BIN="$test_dir/swift" \
    sh "$wrapper"
[ ! -e "$swift_log" ] || fail "helper build ran after a failed capability probe"

FAKE_PROBE_RESULT=success \
FAKE_SWIFT_LOG="$swift_log" \
SWIFTC_BIN="$test_dir/swiftc" \
SWIFT_BIN="$test_dir/swift" \
    sh "$wrapper"
[ "$(cat "$swift_log")" = "build -c release --package-path naming-helper" ] || \
    fail "helper build received unexpected arguments"

set +e
FAKE_PROBE_RESULT=success \
FAKE_BUILD_STATUS=42 \
FAKE_SWIFT_LOG="$swift_log" \
SWIFTC_BIN="$test_dir/swiftc" \
SWIFT_BIN="$test_dir/swift" \
    sh "$wrapper"
result=$?
set -e
[ "$result" -eq 42 ] || fail "helper build failure was not propagated (status $result)"

echo "PASS: Foundation helper capability routing"
