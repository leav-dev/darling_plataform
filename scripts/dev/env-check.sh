#!/usr/bin/env bash
# Fail fast when the toolchain described in DEVELOPMENT.md is incomplete.
#
# The script only reads versions; it never installs or upgrades anything.

set -euo pipefail

readonly MIN_RUST="1.85"
readonly MIN_CMAKE="3.20"

failures=0

have() {
    command -v "$1" >/dev/null 2>&1
}

version_of() {
    case "$1" in
    cmake | git) "$1" --version | head -n1 | awk '{print $3}' ;;
    *) "$1" --version | head -n1 | awk '{print $2}' ;;
    esac
}

version_ge() {
    [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | head -n1)" = "$2" ]
}

check_tool() {
    local name="$1" min="${2:-}" actual
    if ! have "$name"; then
        printf 'MISSING  %-6s not found in PATH\n' "$name"
        failures=$((failures + 1))
        return
    fi

    actual="$(version_of "$name")"
    if [ -n "$min" ] && ! version_ge "$actual" "$min"; then
        printf 'TOO OLD  %-6s %s (need >= %s)\n' "$name" "$actual" "$min"
        failures=$((failures + 1))
        return
    fi

    printf 'OK       %-6s %s\n' "$name" "$actual"
}

printf 'Rust\n'
check_tool cargo "$MIN_RUST"
check_tool rustc "$MIN_RUST"
# Rust ships these as toolchain components, so the binary name is what we probe:
# the component is present exactly when its driver is on PATH.
check_component() {
    local component="$1" binary="$2"
    if have "$binary"; then
        printf 'OK       %-6s installed\n' "$component"
    else
        printf 'MISSING  %-6s run: rustup component add %s\n' "$component" "$component"
        failures=$((failures + 1))
    fi
}

check_component rustfmt rustfmt
check_component clippy cargo-clippy

printf '\nBuild\n'
check_tool cmake "$MIN_CMAKE"
check_tool git

if have ninja; then
    printf 'OK       %-6s %s\n' ninja "$(ninja --version)"
else
    printf 'OPTIONAL %-6s absent; CMake falls back to Makefiles\n' ninja
fi

printf '\n'
if [ "$failures" -gt 0 ]; then
    printf '%d problem(s) found. See DEVELOPMENT.md before building.\n' "$failures" >&2
    exit 1
fi

printf 'Toolchain ready. Next: scripts/ci/check.sh\n'
