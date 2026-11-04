#!/usr/bin/env bash
# Run every gate the CI pipeline runs, locally and in the same order.
#
# Usage:
#   scripts/ci/check.sh                      # debug profile (default)
#   DP_CARGO_PROFILE=release scripts/ci/check.sh
#   DP_BUILD_DIR=/tmp/dp-build scripts/ci/check.sh

set -euo pipefail

cd "$(dirname "$0")/../.."

build_dir="${DP_BUILD_DIR:-build}"
cargo_profile="${DP_CARGO_PROFILE:-debug}"

step() {
    printf '\n==> %s\n' "$*"
}

step "cargo fmt --all --check"
cargo fmt --all --check

step "cargo clippy --workspace --all-targets --locked -- -D warnings"
cargo clippy --workspace --all-targets --locked -- -D warnings

step "cargo test --workspace --locked"
cargo test --workspace --locked

step "cmake configure (${cargo_profile} profile) -> ${build_dir}"
cmake -S . -B "$build_dir" -DDP_CARGO_PROFILE="$cargo_profile"

step "cmake --build ${build_dir}"
cmake --build "$build_dir"

step "ctest --test-dir ${build_dir}"
ctest --test-dir "$build_dir" --output-on-failure

step "tests/packaging/test_packaging.sh"
tests/packaging/test_packaging.sh

printf '\nAll gates passed.\n'
