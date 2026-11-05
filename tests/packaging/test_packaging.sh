#!/usr/bin/env bash
# Offline verification of the darling-platform .deb build and of the local apt
# repository built from it.
#
# Usage:
#   tests/packaging/test_packaging.sh
#
# Needs no root and no network. Everything the checks create lives under a fresh
# mktemp directory that is removed on exit; the only repository-local output is
# what the packaging scripts themselves write under dist/. The package build runs
# `cargo build --locked` without network access, so it needs an already warm
# cargo registry cache (CI warms it by running the test suite first).
#
# The script exits non-zero on the first failure and prints every check it runs.

set -euo pipefail

readonly repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
readonly scratch="$(mktemp -d)"
readonly package_name="darling-platform"

checks=0

cleanup() {
    rm -rf -- "$scratch"
}
trap cleanup EXIT

section() {
    printf '\n==> %s\n' "$*"
}

ok() {
    checks=$((checks + 1))
    printf '  ok   %s\n' "$*"
}

fail() {
    printf '\n  FAIL %s\n' "$*" >&2
    exit 1
}

# Runs a command, keeping its output for the report if it fails.
run_logged() {
    local log="$1"
    shift
    if ! "$@" >"$log" 2>&1; then
        printf '\n--- output of: %s\n' "$*" >&2
        cat "$log" >&2
        fail "command failed: $*"
    fi
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

# Prints the value dpkg-deb reports for one field of a package.
package_field() {
    dpkg-deb --field "$1" "$2"
}

# Asserts that `dpkg-deb --info` reports <name>: <expected>.
assert_info_field() {
    local info="$1" name="$2" expected="$3" actual
    actual="$(printf '%s\n' "$info" | sed -n "s/^[[:space:]]*${name}: *//p")"
    [ "$actual" = "$expected" ] ||
        fail "dpkg-deb --info reports '${name}: ${actual}', expected '${expected}'"
    ok "dpkg-deb --info reports '${name}: ${expected}'"
}

# --- preconditions ----------------------------------------------------------
section "preconditions"

for tool in cargo dpkg dpkg-deb dpkg-scanpackages gzip md5sum sha256sum cmp; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is required by this test"
done
ok "required tools are present (cargo, dpkg, dpkg-deb, dpkg-scanpackages, gzip, md5sum, sha256sum)"

[ "$(id -u)" -ne 0 ] || fail "run this test as a normal user; it never needs root"

host_arch="$(dpkg --print-architecture)"
[ "$host_arch" = "amd64" ] ||
    fail "this packaging is amd64-only: it builds and indexes binary-amd64, but dpkg reports '$host_arch'"
ok "host architecture is amd64 (the packaging builds binary-amd64)"

ws_version="$(workspace_version)"
[ -n "$ws_version" ] || fail "cannot read [workspace.package] version from Cargo.toml"
ok "workspace version from Cargo.toml is ${ws_version}"

expected_version="${ws_version}-1"

# --- 1. the package builds and declares the agreed metadata -----------------
section "1. build-deb.sh produces the package"

run_logged "$scratch/build-deb.log" "$repo_root/scripts/packaging/build-deb.sh"
ok "scripts/packaging/build-deb.sh succeeded"

[ -x "$repo_root/dist/build/release/dp" ] ||
    fail "the release build did not use dist/build as its cargo target dir"
ok "the release binary was built under dist/build (./target untouched)"

dist_dir="$repo_root/dist"
deb_count="$(find "$dist_dir" -maxdepth 1 -type f -name '*.deb' | wc -l | tr -d '[:space:]')"
[ "$deb_count" = "1" ] ||
    fail "expected exactly one .deb in $dist_dir, found ${deb_count}: $(find "$dist_dir" -maxdepth 1 -name '*.deb' -printf '%f ' 2>/dev/null)"
ok "dist/ contains exactly one .deb"

deb="${dist_dir}/${package_name}_${expected_version}_amd64.deb"
[ -f "$deb" ] || fail "expected $deb, which the build did not produce"
ok "the package is named ${deb##*/} (Cargo.toml version ${ws_version} plus revision -1)"

info="$(dpkg-deb --info "$deb")"
assert_info_field "$info" Package "$package_name"
assert_info_field "$info" Version "$expected_version"
assert_info_field "$info" Architecture "amd64"
assert_info_field "$info" Section "utils"

case "$(package_field "$deb" Depends)" in
libc6*)
    ok "dpkg-deb --info reports 'Depends: libc6 …' ($(package_field "$deb" Depends))"
    ;;
*)
    fail "Depends should start with libc6, got '$(package_field "$deb" Depends)'"
    ;;
esac

if "$repo_root/scripts/packaging/build-deb.sh" --version 9.9.9-1 >"$scratch/divergent.log" 2>&1; then
    fail "build-deb.sh accepted --version 9.9.9-1 although Cargo.toml says ${ws_version}"
fi
grep -q 'diverges from the workspace version' "$scratch/divergent.log" ||
    fail "build-deb.sh rejected a divergent version without explaining that it diverges from Cargo.toml: $(head -n3 "$scratch/divergent.log")"
ok "build-deb.sh refuses a version that diverges from Cargo.toml"

# --- 2. the payload is minimal and correct ----------------------------------
section "2. the payload is minimal and correct"

listing="$(dpkg-deb -c "$deb")"
printf '%s\n' "$listing" >"$scratch/listing.txt"

payload_paths="$(printf '%s\n' "$listing" | awk 'NF >= 6 { print $6 }')"
[ -n "$payload_paths" ] || fail "dpkg-deb -c listed no payload files"
ok "dpkg-deb -c lists $(printf '%s\n' "$payload_paths" | wc -l | tr -d '[:space:]') payload entries"

printf '%s\n' "$listing" | grep -q -- '^-rwxr-xr-x root/root .* \./usr/bin/dp$' ||
    fail "dpkg-deb -c does not list ./usr/bin/dp as an executable regular file owned by root:root"
ok "dpkg-deb -c lists ./usr/bin/dp as mode 755 root:root regular file"

for required in \
    './usr/share/doc/darling-platform/copyright' \
    './usr/share/doc/darling-platform/changelog.gz'; do
    printf '%s\n' "$listing" | grep -q -- "^-rw-r--r-- root/root .* ${required}\$" ||
        fail "dpkg-deb -c does not list ${required} as a 644 root:root regular file"
    ok "dpkg-deb -c lists ${required}"
done

unexpected=""
while IFS= read -r path; do
    case "$path" in
    ./) continue ;;
    ./usr/*) continue ;;
    *) unexpected="${unexpected} ${path}" ;;
    esac
done <<<"$payload_paths"
[ -z "$unexpected" ] || fail "the payload has entries outside usr/:${unexpected}"
ok "no payload file lives outside usr/"

control_entries="$(dpkg-deb --ctrl-tarfile "$deb" | tar -t | sed 's|^\./||' | grep -v '^$' | LC_ALL=C sort | tr '\n' ' ')"
[ "$control_entries" = "control md5sums " ] ||
    fail "the control archive holds '${control_entries}', expected exactly 'control md5sums' (no conffiles, no extra control files)"
ok "the control archive carries exactly control and md5sums"

# --- 3. installed size and md5sums are internally consistent ----------------
section "3. installed size and md5sums"

installed_size="$(package_field "$deb" Installed-Size)"
case "$installed_size" in
"" | *[!0-9]*) fail "dpkg-deb --info reports a non-numeric Installed-Size: '${installed_size}'" ;;
esac
[ "$installed_size" -gt 0 ] || fail "dpkg-deb --info reports Installed-Size: 0"
ok "dpkg-deb --info reports Installed-Size: ${installed_size}"

dpkg-deb --ctrl-tarfile "$deb" | tar -xO ./md5sums >"$scratch/md5sums"
[ -s "$scratch/md5sums" ] || fail "the package ships an empty md5sums file"
ok "the package ships a non-empty md5sums file ($(wc -l <"$scratch/md5sums" | tr -d '[:space:]') entries)"

dpkg-deb -x "$deb" "$scratch/extracted"
(
    cd "$scratch/extracted"
    md5sum --quiet --check "$scratch/md5sums"
) >"$scratch/md5check.log" 2>&1 || {
    cat "$scratch/md5check.log" >&2
    fail "md5sum --check failed for the extracted payload"
}
ok "every md5sums digest matches the extracted payload"

staged="${dist_dir}/deb-root-${expected_version}"
[ -d "$staged" ] || fail "the staged tree $staged is missing"
while read -r _ path; do
    [ -n "$path" ] || continue
    [ -e "${staged}/${path}" ] || fail "DEBIAN/md5sums lists ${path}, which is not in the staged tree"
done <"${staged}/DEBIAN/md5sums"
ok "every path in DEBIAN/md5sums exists in the staged tree"

# --- 4. the extracted binary runs ------------------------------------------
section "4. the extracted binary runs"

version_output="$("$scratch/extracted/usr/bin/dp" --version)"
[ "$version_output" = "dp ${ws_version}" ] ||
    fail "\$prefix/usr/bin/dp --version printed '${version_output}', expected 'dp ${ws_version}'"
ok "the extracted binary prints '${version_output}'"

# --- 5. the local apt repository -------------------------------------------
section "5. build-repo.sh produces a consistent repository"

repo="$scratch/repo"
run_logged "$scratch/build-repo.log" "$repo_root/scripts/packaging/build-repo.sh" --out "$repo"
ok "scripts/packaging/build-repo.sh succeeded"

index_dir="${repo}/dists/local/main/binary-amd64"
for artifact in "${index_dir}/Packages" "${index_dir}/Packages.gz" "${repo}/dists/local/Release"; do
    [ -f "$artifact" ] || fail "the repository is missing ${artifact}"
    ok "the repository contains ${artifact#"$repo"/}"
done

[ -f "${repo}/pool/${deb##*/}" ] || fail "the repository pool is missing ${deb##*/}"
ok "the pool holds pool/${deb##*/}"

packages="${index_dir}/Packages"
grep -qx "Filename: pool/${deb##*/}" "$packages" ||
    fail "Packages does not name pool/${deb##*/}"
ok "Packages names pool/${deb##*/}"

deb_sha="$(sha256sum "$deb" | cut -d' ' -f1)"
grep -qx "SHA256: ${deb_sha}" "$packages" ||
    fail "Packages does not record the SHA256 of ${deb##*/} (${deb_sha})"
ok "Packages records the SHA256 of the package (${deb_sha:0:16}…)"

[ "$(sha256sum "${repo}/pool/${deb##*/}" | cut -d' ' -f1)" = "$deb_sha" ] ||
    fail "the pooled copy of ${deb##*/} differs from the built package"
ok "the pooled copy is byte-identical to the built package"

gzip -dc "${index_dir}/Packages.gz" | cmp -s - "$packages" ||
    fail "Packages.gz does not decompress to the same text as Packages"
ok "Packages.gz decompresses to the same text as Packages"

release="${repo}/dists/local/Release"
for field in Origin Label Suite Codename Architectures Components Date MD5Sum SHA256; do
    grep -q "^${field}:" "$release" || fail "Release has no ${field}: field"
done
ok "Release declares Origin, Label, Suite, Codename, Architectures, Components, Date, MD5Sum and SHA256"

if grep -qx "SHA256:${deb_sha}" "$release"; then
    fail "Release lists the .deb checksum directly; it should only cover the index files"
fi
grep -q " main/binary-amd64/Packages$" "$release" || fail "Release does not cover main/binary-amd64/Packages"
ok "Release carries checksums for the generated index files"

run_logged "$scratch/repo-check.log" "$repo_root/scripts/packaging/build-repo.sh" --check --out "$repo"
ok "build-repo.sh --check accepts the repository it just built"

# --- 6. a tampered artefact is rejected ------------------------------------
section "6. a tampered artefact is rejected"

tampered="$scratch/repo-tampered"
cp -a "$repo" "$tampered"
printf 'tampered\n' >>"${tampered}/pool/${deb##*/}"

if "$repo_root/scripts/packaging/build-repo.sh" --check --out "$tampered" >"$scratch/tamper.log" 2>&1; then
    cat "$scratch/tamper.log" >&2
    fail "build-repo.sh --check accepted a repository whose package was modified"
fi
grep -q 'does not match Packages' "$scratch/tamper.log" ||
    fail "build-repo.sh --check failed without reporting the hash mismatch: $(head -n5 "$scratch/tamper.log")"
ok "build-repo.sh --check rejects the tampered package and reports the mismatch"

# --- 7. signing is explicit and unsigned repositories are loud --------------
section "7. signing behaviour"

grep -q 'UNSIGNED' "$scratch/build-repo.log" ||
    fail "building without --sign did not warn that the repository is unsigned"
ok "building without --sign warns that the repository is unsigned"

sign_repo="$scratch/repo-signing"
sign_log="$scratch/sign.log"
if command -v gpg >/dev/null 2>&1; then
    mkdir -p "$scratch/gnupg"
    chmod 700 "$scratch/gnupg"
    if GNUPGHOME="$scratch/gnupg" "$repo_root/scripts/packaging/build-repo.sh" \
        --sign 'no-such-key@darling-platform.invalid' --out "$sign_repo" >"$sign_log" 2>&1; then
        fail "--sign accepted a key id that does not exist"
    fi
    grep -q 'no secret key' "$sign_log" ||
        fail "--sign did not explain the missing key: $(head -n5 "$sign_log")"
    ok "--sign fails with an actionable message when the key is missing"
else
    if "$repo_root/scripts/packaging/build-repo.sh" --sign KEY --out "$sign_repo" >"$sign_log" 2>&1; then
        fail "--sign succeeded without gpg installed"
    fi
    grep -q 'gpg is required' "$sign_log" ||
        fail "--sign did not explain that gpg is missing: $(head -n5 "$sign_log")"
    ok "--sign fails with an actionable message when gpg is missing"
fi

# --- 8. the package build is reproducible ----------------------------------
section "8. the package build is reproducible"

first_sha="$(sha256sum "$deb" | cut -d' ' -f1)"
run_logged "$scratch/build-deb-again.log" "$repo_root/scripts/packaging/build-deb.sh"
second_sha="$(sha256sum "$deb" | cut -d' ' -f1)"
[ "$first_sha" = "$second_sha" ] ||
    fail "a second build produced a different package (${first_sha:0:16}… vs ${second_sha:0:16}…)"
ok "a second build produces a byte-identical package (${first_sha:0:16}…)"

printf '\nall %s checks passed\n' "$checks"
