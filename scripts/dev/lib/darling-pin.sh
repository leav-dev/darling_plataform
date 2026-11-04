#!/usr/bin/env bash
# Helpers shared by scripts/dev/install-darling-pin.sh and its tests.
#
# The release archive does not put the packages at its root: it extracts into a
# dated directory (for example `debs_20260609/`). Discovery must therefore walk the
# extracted tree and installation must pass the discovered paths, never a glob that
# only matches the top level.

# Prints every Debian package below one directory, one absolute path per line.
discover_packages() {
    local root="$1"
    find "$root" -type f -name '*.deb' | sort
}

# Prints a copy-pasteable install command for the given packages.
#
# `apt-get` is used instead of `dpkg -i` because Darling ships as many packages in
# one archive and apt resolves their order and dependencies.
render_install_command() {
    if [ "$#" -eq 0 ]; then
        printf 'error: render_install_command needs at least one package\n' >&2
        return 1
    fi

    printf '  sudo apt-get install -y \\\n'
    printf '    %q \\\n' "$@"
}
