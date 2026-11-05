#!/usr/bin/env bash
# Build the darling-platform Debian package from this checkout, reproducibly.
#
# Usage:
#   scripts/packaging/build-deb.sh                            # version from Cargo.toml + "-1"
#   scripts/packaging/build-deb.sh --version 0.1.0-1          # explicit version, checked against Cargo.toml
#   scripts/packaging/build-deb.sh --out /tmp/deb-out         # write everything under another directory
#
# Everything the build produces stays under --out (default: <repo>/dist):
#   <out>/build/                     cargo target dir, never the workspace ./target
#   <out>/deb-root-<version>/        staged package tree
#   <out>/darling-platform_<version>_<arch>.deb
#
# The build needs cargo, dpkg-deb (>= 1.19 for --root-owner-group) and gzip. It
# never calls sudo, never installs anything and never writes outside --out.

set -euo pipefail

readonly repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
readonly package_name="darling-platform"

# 0755/0644 for everything dpkg-deb copies into the archive.
umask 0022

usage() {
    sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
}

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

note() {
    printf '    %s\n' "$*"
}

step() {
    printf '\n==> %s\n' "$*"
}

# Reads `version` from the [workspace.package] table of the workspace manifest.
workspace_version() {
    awk '
        /^\[workspace\.package\]/ { in_table = 1; next }
        /^\[/                     { in_table = 0 }
        in_table && /^[[:space:]]*version[[:space:]]*=/ {
            line = $0
            sub(/^[^=]*=[[:space:]]*"/, "", line)
            sub(/".*/, "", line)
            print line
            exit
        }
    ' "$repo_root/Cargo.toml"
}

# Removes a path, but only when it is strictly inside the output directory.
remove_tree() {
    local path="$1"
    case "$path" in
    "" | "/") fail "refusing to remove '$path'" ;;
    "${out_dir}/"?*) ;;
    *) fail "refusing to remove '$path': it is outside '$out_dir'" ;;
    esac
    rm -rf -- "$path"
}

version=""
out_dir="${repo_root}/dist"

while [ "$#" -gt 0 ]; do
    case "$1" in
    --version)
        [ "$#" -ge 2 ] || fail "--version needs a value, for example 0.1.0-1"
        version="$2"
        shift 2
        ;;
    --version=*)
        version="${1#*=}"
        shift
        ;;
    --out)
        [ "$#" -ge 2 ] || fail "--out needs a directory"
        out_dir="$2"
        shift 2
        ;;
    --out=*)
        out_dir="${1#*=}"
        shift
        ;;
    -h | --help)
        usage
        exit 0
        ;;
    *)
        fail "unknown argument '$1' (see --help)"
        ;;
    esac
done

for tool in cargo dpkg dpkg-deb gzip; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is required to build the package"
done

dpkg_deb_version="$(LC_ALL=C dpkg-deb --version | head -n1 | grep -o '[0-9][0-9.]*' | head -n1)"
note "dpkg-deb ${dpkg_deb_version-unknown}"
if ! dpkg-deb --help 2>/dev/null | grep -q -- '--root-owner-group'; then
    fail "dpkg-deb is too old: --root-owner-group (dpkg 1.19 or newer) is required so the package build needs no fakeroot"
fi

readonly cargo_version="$(workspace_version)"
[ -n "$cargo_version" ] || fail "cannot read [workspace.package] version from $repo_root/Cargo.toml"

if [ -z "$version" ]; then
    version="${cargo_version}-1"
fi

case "$version" in
*[!A-Za-z0-9.+~:-]*) fail "version '$version' contains characters that are not valid in a Debian version" ;;
esac

upstream_version="${version%-*}"
if [ "$upstream_version" = "$version" ]; then
    fail "version '$version' has no Debian revision: the package version is '<Cargo.toml version>-<revision>', for example '${cargo_version}-1'"
fi
if [ "$upstream_version" != "$cargo_version" ]; then
    fail "version '$version' diverges from the workspace version '$cargo_version' in Cargo.toml: the upstream part of the package version must match it, for example '${cargo_version}-1'"
fi

mkdir -p "$out_dir"
out_dir="$(cd "$out_dir" && pwd)"

readonly arch="$(dpkg --print-architecture)"
readonly staging="${out_dir}/deb-root-${version}"
readonly cargo_target_dir="${out_dir}/build"
readonly deb="${out_dir}/${package_name}_${version}_${arch}.deb"
readonly cargo_binary="${cargo_target_dir}/release/dg"

# Timestamps inside the archive come from SOURCE_DATE_EPOCH so two builds of the
# same commit produce byte-identical packages. The last commit time is the
# natural default; an unset value falls back to dpkg-deb's own behaviour.
source_date_epoch="${SOURCE_DATE_EPOCH-}"
if [ -z "$source_date_epoch" ]; then
    source_date_epoch="$(git -C "$repo_root" log -1 --format=%ct 2>/dev/null || true)"
fi
if [ -n "$source_date_epoch" ]; then
    export SOURCE_DATE_EPOCH="$source_date_epoch"
    note "SOURCE_DATE_EPOCH=$SOURCE_DATE_EPOCH"
else
    note "warning: SOURCE_DATE_EPOCH is unset and no git commit time is available; the resulting package will not be byte-reproducible"
fi

step "cargo build --release (target dir: $cargo_target_dir)"
(
    cd "$repo_root"
    cargo build --release --locked --package dp-cli --bin dp --target-dir "$cargo_target_dir"
)
[ -x "$cargo_binary" ] || fail "cargo finished but $cargo_binary is not executable"

step "staging the package tree in $staging"
remove_tree "$staging"
install -d -m 0755 \
    "$staging/DEBIAN" \
    "$staging/usr/bin" \
    "$staging/usr/share/doc/${package_name}"

# Payload. There is no man page and no conffile: the package ships exactly the
# binary, its copyright and its changelog.
install -m 0755 "$cargo_binary" "$staging/usr/bin/dg"
install -m 0644 "$repo_root/packaging/deb/copyright" "$staging/usr/share/doc/${package_name}/copyright"
gzip --best --no-name -c "$repo_root/CHANGELOG.md" >"$staging/usr/share/doc/${package_name}/changelog.gz"
chmod 0644 "$staging/usr/share/doc/${package_name}/changelog.gz"

installed_size="$(du -k -s "$staging" | cut -f1)"

step "writing DEBIAN/control (version $version, architecture $arch, installed size ${installed_size} KiB)"
sed -e "s|@VERSION@|${version}|g" \
    -e "s|@ARCHITECTURE@|${arch}|g" \
    -e "s|@INSTALLED_SIZE@|${installed_size}|g" \
    "$repo_root/packaging/deb/control.in" >"$staging/DEBIAN/control"
chmod 0644 "$staging/DEBIAN/control"

if grep -q '@[A-Z_]*@' "$staging/DEBIAN/control"; then
    fail "DEBIAN/control still contains an unsubstituted placeholder: $(grep -o '@[A-Z_]*@' "$staging/DEBIAN/control" | sort -u | tr '\n' ' ')"
fi

# Digest list of every payload file, in a stable order. Paths are relative to
# the package root and never mention DEBIAN, exactly as dpkg expects them.
(
    cd "$staging"
    find . -path ./DEBIAN -prune -o -type f -print |
        sed 's|^\./||' |
        LC_ALL=C sort |
        xargs -r md5sum >DEBIAN/md5sums
)
chmod 0644 "$staging/DEBIAN/md5sums"

step "dpkg-deb --build --root-owner-group"
remove_tree "$deb"
dpkg-deb --build --root-owner-group "$staging" "$deb"

step "dpkg-deb --info $deb"
dpkg-deb --info "$deb"

size_bytes="$(wc -c <"$deb" | tr -d '[:space:]')"
size_human="$(LC_ALL=C awk -v bytes="$size_bytes" 'BEGIN { printf "%.1f KiB", bytes / 1024 }')"

step "done"
note "package: $deb"
note "size:    ${size_bytes} bytes (${size_human})"

printf '%s\n' "$deb"
