# Darling Platform — top-level CMake project.
#
# Responsibilities of this file:
#   * declare the project and its install layout;
#   * delegate Rust compilation to Cargo through cmake/Rust.cmake;
#   * expose the test suite to CTest;
#   * install the binaries the Cargo build produces.
#
# The native Darling build stays a separate, pinned external dependency: it is
# not a Cargo workspace member and it is not built from here. That integration
# arrives with the shared-prefix feature, together with the version/commit pin it
# requires.

cmake_minimum_required(VERSION 3.20)

project(
    darling-platform
    VERSION 0.1.0
    DESCRIPTION "Import, manage and run macOS applications on Linux through Darling"
    LANGUAGES NONE
)

# The project declares no compiled language: Cargo builds everything. GNUInstallDirs
# cannot derive a default libdir without a known target architecture, and the
# skeleton installs only a binary, so the one directory it needs is declared here
# and stays overridable from the cache or the command line.
set(CMAKE_INSTALL_BINDIR "bin" CACHE PATH "Directory for user executables")

list(APPEND CMAKE_MODULE_PATH "${CMAKE_CURRENT_SOURCE_DIR}/cmake")
include(Rust)

enable_testing()

dp_cargo_build(dp-rust BINARIES dp)
dp_cargo_test(dp-rust-test)

add_test(
    NAME rust-unit
    COMMAND ${CARGO_EXECUTABLE} test --workspace --locked
            --target-dir ${DP_CARGO_TARGET_DIR} ${DP_CARGO_PROFILE_FLAG}
)
set_tests_properties(
    rust-unit
    PROPERTIES
        WORKING_DIRECTORY "${PROJECT_SOURCE_DIR}"
        LABELS "rust"
)

install(
    PROGRAMS "${DP_CARGO_ARTIFACT_DIR}/dp"
    DESTINATION "${CMAKE_INSTALL_BINDIR}"
)

message(STATUS "Darling Platform ${PROJECT_VERSION} · cargo profile: ${DP_CARGO_PROFILE}")
message(STATUS "  cargo target dir: ${DP_CARGO_TARGET_DIR}")
message(STATUS "  artifacts:        ${DP_CARGO_ARTIFACT_DIR}")
