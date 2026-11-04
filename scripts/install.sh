#!/usr/bin/env bash
# Install Darling Platform in one command.
#
# Usage:
#   scripts/install.sh --dry-run      # print every command, execute nothing
#   scripts/install.sh                # ask before each elevated step
#   scripts/install.sh --yes          # no confirmations
#   scripts/install.sh --no-darling   # the pinned Darling is already installed
#   scripts/install.sh --from-deb F   # install an already built .deb instead of building
#
# This script runs unprivileged on purpose. Every step that needs root says why it
# needs it and then asks `sudo` for the password; nothing keeps a root shell or a
# root process alive afterwards. Running it as root is refused.
set -euo pipefail

readonly repo_root="$(cd "$(dirname "$0")/.." && pwd)"
readonly installer="${repo_root}/scripts/dev/install-darling-pin.sh"
readonly deb_builder="${repo_root}/scripts/packaging/build-deb.sh"

dry_run=0
assume_yes=0
install_darling=1
from_deb=""

usage() {
    awk 'NR > 1 { if ($0 !~ /^#/) exit; sub(/^# ?/, ""); print }' "$0"
}

for argument in "$@"; do
    case "$argument" in
    --dry-run) dry_run=1 ;;
    --yes) assume_yes=1 ;;
    --no-darling) install_darling=0 ;;
    --from-deb)
        from_deb="__next__"
        ;;
    -h | --help)
        usage
        exit 0
        ;;
    *)
        if [ "${from_deb:-}" = "__next__" ]; then
            from_deb="$argument"
            continue
        fi
        printf 'error: unknown argument `%s` (try --help)\n' "$argument" >&2
        exit 2
        ;;
    esac
done

if [ "$from_deb" = "__next__" ]; then
    printf 'error: `--from-deb` needs a path to a .deb\n' >&2
    exit 2
fi

step() {
    printf '\n==> %s\n' "$*"
}
note() {
    printf '    %s\n' "$*"
}
fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

# Prints a command; runs it unless --dry-run.
run() {
    printf '    $ %s\n' "$*"
    if [ "$dry_run" -eq 0 ]; then
        "$@"
    fi
}

# Prints and runs a command that needs root, after explaining why.
elevated() {
    printf '    $ sudo %s\n' "$*"
    if [ "$dry_run" -eq 0 ]; then
        sudo "$@"
    fi
}

# Asks a yes/no question. Closed when --yes was passed.
confirm() {
    if [ "$assume_yes" -eq 1 ] || [ "$dry_run" -eq 1 ]; then
        return 0
    fi

    printf '    continue? [y/N] '
    local answer
    if ! read -r answer; then
        fail "no answer on stdin; pass --yes for an unattended install"
    fi
    case "$answer" in
    y | Y | yes | YES) return 0 ;;
    *) fail "cancelled by the operator" ;;
    esac
}

# --- 0. never run as root ---------------------------------------------------
if [ "$(id -u)" -eq 0 ]; then
    fail "run this as your normal user, not as root: each step elevates with sudo and
       drops the privilege afterwards, which is impossible if the whole script is root"
fi

printf 'Darling Platform installer\n'
if [ "$dry_run" -eq 1 ]; then
    printf 'mode: dry run (nothing will be installed)\n'
fi

# --- 1. preflight -----------------------------------------------------------
step "1/5 preflight"

missing=()
for tool in dpkg apt-get sudo sha256sum; do
    command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
done

if [ "$install_darling" -eq 1 ]; then
    for tool in curl wget unzip; do
        command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
    done
fi

if [ "$from_deb" = "" ]; then
    for tool in cargo rustc; do
        command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
    done
fi

if [ "${#missing[@]}" -gt 0 ]; then
    fail "missing tools: ${missing[*]}
       install them first (for Cargo: https://rustup.rs) and run this script again"
fi

if [ -r /etc/os-release ]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    note "system: ${PRETTY_NAME:-unknown}"
    case " ${ID:-} ${ID_LIKE:-} " in
    *debian* | *ubuntu*) ;;
    *) note "warning: this packaging path targets Debian-based systems; continuing anyway" ;;
    esac
fi

case "$(uname -m)" in
x86_64 | amd64) note "architecture: x86-64" ;;
*) fail "the pinned Darling release ships x86-64 packages; $(uname -m) is not supported yet" ;;
esac

if [ "$install_darling" -eq 1 ] && [ ! -x "$installer" ]; then
    fail "$installer is missing; run this from a repository checkout"
fi
if [ "$from_deb" = "" ] && [ ! -x "$deb_builder" ]; then
    fail "$deb_builder is missing; run this from a repository checkout"
fi

# --- 2. explain the root boundary ------------------------------------------
step "2/5 privileges"

cat <<'EOF'
    Two steps need root: installing Darling's packages (its binary is setuid
    root) and installing the darling-platform package. Both run through sudo,
    which will ask for your password once. Nothing keeps root afterwards.
EOF
note "elevated commands that will run:"
note "  sudo dpkg -i <pinned darling packages>"
note "  sudo apt-get install -y <darling-platform .deb>"

if [ "$dry_run" -eq 0 ]; then
    confirm "ask for the sudo password now?"
    step "requesting privileges (sudo -v)"
    sudo -v || fail "sudo was refused; nothing was installed"
fi

# --- 3. Darling pin ---------------------------------------------------------
step "3/5 Darling (pinned)"
if [ "$install_darling" -eq 1 ]; then
    note "release pinned in deps/darling.toml; assets are verified by sha256"
    confirm "download and install the pinned Darling?"
    if [ "$dry_run" -eq 1 ]; then
        printf '    $ %s --yes\n' "$installer"
    else
        "$installer" --yes
    fi
else
    note "skipped (--no-darling)"
fi

# --- 4. darling-platform ----------------------------------------------------
step "4/5 darling-platform"
if [ "$from_deb" != "" ]; then
    package="$from_deb"
    [ -f "$package" ] || fail "$package does not exist"
    note "using the package you provided: $package"
else
    confirm "build the darling-platform package from this checkout?"
    if [ "$dry_run" -eq 1 ]; then
        printf '    $ %s\n' "$deb_builder"
        package="${repo_root}/dist/darling-platform_<version>_amd64.deb"
    else
        "$deb_builder"
        package="$(find "${repo_root}/dist" -maxdepth 1 -name 'darling-platform_*_amd64.deb' -print -quit)"
        [ -n "$package" ] || fail "the package build produced no .deb in ${repo_root}/dist"
    fi
fi

note "installing through apt so its dependency resolution is respected"
confirm "install ${package##*/}?"
elevated apt-get install -y "$package"

# --- 5. verification --------------------------------------------------------
step "5/5 verification"
if [ "$dry_run" -eq 1 ]; then
    note "would verify: dg --version, dg prefix validate, darling shell uname"
else
    command -v dg >/dev/null 2>&1 || fail "dg is not on PATH after installation; check the package payload"
    printf '    dg --version ....... '
    dg --version
    printf '    dg prefix validate . '
    dg prefix validate >/dev/null && printf 'ok\n'

    if [ "$install_darling" -eq 1 ]; then
        printf '    darling shell ...... '
        scratch="$(mktemp -d)"
        if DPREFIX="${scratch}/prefix" timeout 300 darling shell uname 2>/dev/null | grep -q Darwin; then
            printf 'Darwin\n'
        else
            printf 'failed\n'
            note "the platform is installed, but Darling did not answer."
            note "re-run: DPREFIX=${scratch}/prefix darling shell uname"
        fi
        rm -rf "$scratch"
    fi
fi

step "done"
cat <<EOF
    darling-platform is installed. Next:

      dg prefix start        # create the shared prefix and start the environment
      dg prefix status       # phase, lock and warnings

    Uninstall the tool (the prefix and your applications stay untouched):

      sudo apt remove darling-platform
EOF
