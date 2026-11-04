#!/usr/bin/env bash
# Verify the pinned-Darling installer's package discovery and install command.
#
# Regression under test: the release archive extracts into a dated subdirectory,
# so a flat `packages/*.deb` glob matched nothing and passed the literal pattern to
# dpkg. Discovery must walk the tree and the command must carry real paths.
#
# Hermetic: no root, no network, no downloads, no system changes.
set -uo pipefail

readonly repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
readonly lib="${repo_root}/scripts/dev/lib/darling-pin.sh"
readonly installer="${repo_root}/scripts/dev/install-darling-pin.sh"

failures=0
checks=0

ok() {
    checks=$((checks + 1))
    printf '  ok   %s\n' "$1"
}
bad() {
    checks=$((checks + 1))
    failures=$((failures + 1))
    printf '  FAIL %s\n' "$1"
}
expect_contains() {
    local label="$1" haystack="$2" needle="$3"
    case "$haystack" in
    *"$needle"*) ok "$label" ;;
    *) bad "$label (missing: $needle)" ;;
    esac
}
expect_absent() {
    local label="$1" haystack="$2" needle="$3"
    case "$haystack" in
    *"$needle"*) bad "$label (unexpected: $needle)" ;;
    *) ok "$label" ;;
    esac
}
expect_eq() {
    local label="$1" actual="$2" expected="$3"
    if [ "$actual" = "$expected" ]; then
        ok "$label"
    else
        bad "$label (expected '$expected', got '$actual')"
    fi
}

if [ "$(id -u)" -eq 0 ]; then
    printf 'error: run this as a normal user; it must not need root\n' >&2
    exit 1
fi

printf '\n==> preconditions\n'
[ -f "$lib" ] && ok "the helper library exists" || bad "the helper library exists"
[ -f "$installer" ] && ok "the installer exists" || bad "the installer exists"
bash -n "$lib" 2>/dev/null && ok "the library parses" || bad "the library parses"
bash -n "$installer" 2>/dev/null && ok "the installer parses" || bad "the installer parses"

# shellcheck source=/dev/null
. "$lib"

printf '\n==> discovery walks a nested archive layout\n'
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

mkdir -p "$scratch/packages/debs_20990101/nested"
touch "$scratch/packages/debs_20990101/darling-cli_1_amd64.deb"
touch "$scratch/packages/debs_20990101/darling_1_amd64.deb"
touch "$scratch/packages/debs_20990101/nested/darling-core_1_amd64.deb"
touch "$scratch/packages/README.txt"

mapfile -t found < <(discover_packages "$scratch/packages")
expect_eq "finds every package below the extract root, and only packages" "${#found[@]}" "3"
expect_contains "finds the top-level packages" "${found[*]}" "debs_20990101/darling_1_amd64.deb"
expect_contains "finds packages one directory deeper" "${found[*]}" "debs_20990101/nested/darling-core_1_amd64.deb"
expect_absent "ignores non-package files" "${found[*]}" "README.txt"

printf '\n==> the rendered command carries real paths\n'
command_text="$(render_install_command "${found[@]}")"
expect_contains "uses apt so dependencies and order are resolved" "$command_text" "apt-get install -y"
expect_absent "never prints a glob dpkg cannot expand" "$command_text" "*.deb"
for package in "${found[@]}"; do
    expect_contains "prints $package" "$command_text" "$package"
done

printf '\n==> an empty extract root is an error, not an empty install\n'
mkdir -p "$scratch/empty"
mapfile -t none < <(discover_packages "$scratch/empty")
expect_eq "finds nothing in an empty tree" "${#none[@]}" "0"
render_install_command "${none[@]}" >/dev/null 2>&1 &&
    bad "rendering zero packages must fail" ||
    ok "rendering zero packages fails instead of printing an empty command"

printf '\n==> the installer still runs its dry run\n'
if "$installer" --dry-run >/dev/null 2>&1; then
    ok "--dry-run exits 0"
else
    bad "--dry-run exits 0"
fi
if "$installer" --nope >/dev/null 2>&1; then
    bad "an unknown argument is rejected"
else
    ok "an unknown argument is rejected"
fi

printf '\n'
if [ "$failures" -ne 0 ]; then
    printf '%d of %d checks failed\n' "$failures" "$checks" >&2
    exit 1
fi
printf 'all %d checks passed\n' "$checks"
