#!/usr/bin/env bash
# Install the pinned Darling release, verifying the release asset first.
#
# Usage:
#   scripts/dev/install-darling-pin.sh --dry-run   # print what would be downloaded
#   scripts/dev/install-darling-pin.sh             # download + verify, install nothing
#   scripts/dev/install-darling-pin.sh --yes       # download, verify and install (needs sudo)
#
# The pin lives in deps/darling.toml. This script never trusts the network: it
# checks the sha256 recorded in the pin before unpacking, and it installs only with
# an explicit `--yes`, because the resulting `darling` binary is setuid root.
set -euo pipefail

readonly script_dir="$(cd "$(dirname "$0")" && pwd)"
readonly repo_root="$(cd "${script_dir}/../.." && pwd)"
readonly pin_file="${repo_root}/deps/darling.toml"

# shellcheck source=lib/darling-pin.sh
. "${script_dir}/lib/darling-pin.sh"

install_yes=0
dry_run=0

for argument in "$@"; do
    case "$argument" in
    --yes) install_yes=1 ;;
    --dry-run) dry_run=1 ;;
    -h | --help)
        sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
        exit 0
        ;;
    *)
        printf 'error: unknown argument `%s` (try --help)\n' "$argument" >&2
        exit 2
        ;;
    esac
done

if [ ! -f "$pin_file" ]; then
    printf 'error: %s is missing\n' "$pin_file" >&2
    exit 1
fi

# Reads `key = "value"` from the pin file. The pin file holds one value per key.
read_pin() {
    grep -E "^[[:space:]]*$1[[:space:]]*=" "$pin_file" | head -n1 |
        sed -E 's/^[^=]*=[[:space:]]*"?([^"]*)"?[[:space:]]*$/\1/'
}

readonly version="$(read_pin version)"
readonly release_commit="$(read_pin commit)"
readonly asset_name="$(read_pin debs_name)"
readonly asset_sha256="$(read_pin debs_sha256)"

if [ -z "$version" ] || [ -z "$asset_name" ] || [ -z "$asset_sha256" ]; then
    printf 'error: %s must define version, debs_name and debs_sha256\n' "$pin_file" >&2
    exit 1
fi

readonly cache_dir="${XDG_CACHE_HOME:-$HOME/.cache}/darling-platform/darling-pin/${version}"
readonly archive="${cache_dir}/${asset_name}"
readonly url="https://github.com/darlinghq/darling/releases/download/${version}/${asset_name}"

printf 'pinned Darling : %s (%s)\n' "$version" "$release_commit"
printf 'asset          : %s\n' "$asset_name"
printf 'sha256         : %s\n' "$asset_sha256"
printf 'source         : %s\n' "$url"
printf 'cache          : %s\n' "$cache_dir"

if [ "$dry_run" -eq 1 ]; then
    printf '\ndry run: nothing downloaded, nothing installed.\n'
    exit 0
fi

if ! command -v sha256sum >/dev/null 2>&1; then
    printf 'error: sha256sum is required to verify the release asset\n' >&2
    exit 1
fi

mkdir -p "$cache_dir"

if [ ! -f "$archive" ]; then
    downloader=""
    for candidate in curl wget; do
        if command -v "$candidate" >/dev/null 2>&1; then
            downloader="$candidate"
            break
        fi
    done

    if [ -z "$downloader" ]; then
        printf 'error: need curl or wget to download the release asset\n' >&2
        exit 1
    fi

    printf '\n==> downloading %s\n' "$asset_name"
    case "$downloader" in
    curl) curl --fail --location --output "${archive}.part" "$url" ;;
    wget) wget --output-document="${archive}.part" "$url" ;;
    esac
    mv "${archive}.part" "$archive"
else
    printf '\n==> reusing %s\n' "$archive"
fi

printf '==> verifying sha256\n'
printf '%s  %s\n' "$asset_sha256" "$archive" | sha256sum --check --status - ||
    {
        printf 'error: %s does not match the pin; refusing to install it\n' "$archive" >&2
        exit 1
    }
printf 'ok\n'

if ! command -v dpkg >/dev/null 2>&1; then
    printf '\nerror: dpkg not found; the pinned Debian packages need a Debian-based system\n' >&2
    printf 'build from the pinned source archive instead: %s\n' "$(read_pin source_name)" >&2
    exit 1
fi

package_dir="${cache_dir}/packages"
mkdir -p "$package_dir"
if command -v unzip >/dev/null 2>&1; then
    unzip -o -q "$archive" -d "$package_dir"
else
    printf 'error: unzip is required to extract %s\n' "$asset_name" >&2
    exit 1
fi

mapfile -t packages < <(discover_packages "$package_dir")
if [ "${#packages[@]}" -eq 0 ]; then
    printf 'error: no .deb packages found inside %s\n' "$asset_name" >&2
    exit 1
fi

printf '\nverified packages (%d):\n' "${#packages[@]}"
printf '  %s\n' "${packages[@]#"$package_dir"/}"

if [ "$install_yes" -ne 1 ]; then
    printf '\nnothing installed. To install the verified packages, run:\n'
    render_install_command "${packages[@]}"
    printf '\nReview first: the resulting darling binary is setuid root.\n'
    exit 0
fi

printf '\n==> installing with apt (sudo)\n'
sudo apt-get install -y "${packages[@]}"
printf '\nDone. Verify with: DPREFIX=/tmp/darling-pin-check/dp darling shell uname\n'
