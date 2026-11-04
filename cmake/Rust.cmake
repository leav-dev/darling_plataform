# Rust helpers for the Darling Platform CMake tree.
#
# CMake coordinates the native project and the Rust crates, but Cargo still owns
# compilation, dependency resolution and the test runner. Every cargo invocation
# runs with --target-dir inside the CMake binary directory so an out-of-source
# build never writes into the source tree.
#
# Deliberate scope: there is no Darling discovery and no packaging module yet.
# Those files arrive together with the first feature that consumes them, not
# before.

find_program(
    CARGO_EXECUTABLE
    NAMES cargo
    REQUIRED
    DOC "Cargo executable used to build and test the Rust workspace"
)

set(
    DP_CARGO_PROFILE
    "debug"
    CACHE STRING
    "Cargo profile used by the build (debug or release)"
)
set_property(CACHE DP_CARGO_PROFILE PROPERTY STRINGS debug release)

set(
    DP_CARGO_TARGET_DIR
    "${CMAKE_BINARY_DIR}/cargo"
    CACHE PATH
    "Cargo target directory; defaults inside the CMake build tree"
)

if(DP_CARGO_PROFILE STREQUAL "release")
    set(DP_CARGO_PROFILE_FLAG "--release")
    set(DP_CARGO_ARTIFACT_DIR "${DP_CARGO_TARGET_DIR}/release")
elseif(DP_CARGO_PROFILE STREQUAL "debug")
    set(DP_CARGO_PROFILE_FLAG "")
    set(DP_CARGO_ARTIFACT_DIR "${DP_CARGO_TARGET_DIR}/debug")
else()
    message(
        FATAL_ERROR
        "DP_CARGO_PROFILE must be 'debug' or 'release', got '${DP_CARGO_PROFILE}'"
    )
endif()

set(DP_CARGO_ARTIFACT_DIR "${DP_CARGO_ARTIFACT_DIR}")

# dp_cargo_build(<target> [BINARIES <name>...])
#
# Creates an always-run custom target that builds the whole workspace. Cargo
# keeps its own incremental state, so re-running the command is cheap and the
# build stays correct when a source file changes without touching Cargo.toml.
function(dp_cargo_build target)
    cmake_parse_arguments(ARG "" "" "BINARIES" ${ARGN})

    add_custom_target(
        ${target} ALL
        COMMAND ${CMAKE_COMMAND} -E env CARGO_TERM_COLOR=always
                ${CARGO_EXECUTABLE} build --workspace --locked
                --target-dir ${DP_CARGO_TARGET_DIR} ${DP_CARGO_PROFILE_FLAG}
        WORKING_DIRECTORY ${PROJECT_SOURCE_DIR}
        COMMENT "Building the Rust workspace (cargo, ${DP_CARGO_PROFILE} profile)"
        VERBATIM
    )

    foreach(binary IN LISTS ARG_BINARIES)
        set(path "${DP_CARGO_ARTIFACT_DIR}/${binary}")
        if(NOT EXISTS "${path}")
            message(
                STATUS
                "${binary} will be built into ${DP_CARGO_ARTIFACT_DIR} by target ${target}"
            )
        endif()
    endforeach()
endfunction()

# dp_cargo_test(<target>)
#
# Creates a custom target that runs the workspace test suite. CTest uses the
# same command through add_test, so `ctest` and `cmake --build --target <target>`
# stay equivalent.
function(dp_cargo_test target)
    add_custom_target(
        ${target}
        COMMAND ${CMAKE_COMMAND} -E env CARGO_TERM_COLOR=always
                ${CARGO_EXECUTABLE} test --workspace --locked
                --target-dir ${DP_CARGO_TARGET_DIR} ${DP_CARGO_PROFILE_FLAG}
        WORKING_DIRECTORY ${PROJECT_SOURCE_DIR}
        COMMENT "Running the Rust test suite (cargo test)"
        USES_TERMINAL
        VERBATIM
    )
endfunction()
