//! Adapter over the Darling command line.
//!
//! Darling exposes no library interface and no initialisation or status command,
//! so every lifecycle operation is a child process. This module is the only place
//! in the crate that spawns one, which keeps the verified command set (see
//! `ADR-0008`) in a single file to correct when Darling changes.
//!
//! Two guarantees live here:
//!
//! - `DPREFIX` is exported into the child's environment and never passed as an
//!   inline assignment, because the Darling documentation states that inline use
//!   "does not work as expected" (F1).
//! - The exit code is reported, not interpreted (F3: `darling shell` connects to
//!   a running container, so the same command means different things depending on
//!   the state); deciding what a non-zero exit means is the manager's job.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::{Error, Result};

/// Environment variable that points the platform at a specific Darling binary.
const PROGRAM_ENV: &str = "DP_DARLING";

/// Darling program name used when [`PROGRAM_ENV`] is unset, resolved through `PATH`.
const DEFAULT_PROGRAM: &str = "darling";

/// Environment variable Darling reads to select its prefix (F1).
const PREFIX_ENV: &str = "DPREFIX";

/// Outcome of a Darling invocation that did start.
///
/// Anything the child produced is captured, so the caller can attribute a failure
/// to the exact command that produced it. `exit_code` is `None` when the child was
/// terminated by a signal and therefore never reported one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome {
    /// Exit status the child reported, if it reported one at all.
    pub exit_code: Option<i32>,
    /// Everything the child wrote to standard output.
    pub stdout: String,
    /// Everything the child wrote to standard error.
    pub stderr: String,
}

/// The lifecycle operations the platform performs on Darling.
///
/// Implementors only have to run a program with `DPREFIX` exported; they must not
/// decide whether the invocation succeeded.
pub trait Darling {
    /// Program invoked for every operation.
    fn program(&self) -> &Path;

    /// Runs `program <args…>` with `DPREFIX` exported to the child (F1: never an
    /// inline assignment). Returns the outcome for any successful spawn; spawn
    /// failures are errors.
    fn run(&self, prefix: &Path, args: &[&str]) -> Result<CommandOutcome>;
}

/// The real Darling binary, as a child process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemDarling {
    program: PathBuf,
}

impl SystemDarling {
    /// Runs `program` for every operation.
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// Reads `DP_DARLING`, defaulting to `darling` resolved through `PATH`.
    pub fn from_env() -> Self {
        match std::env::var_os(PROGRAM_ENV) {
            Some(program) if !program.is_empty() => Self::new(PathBuf::from(program)),
            _ => Self::new(DEFAULT_PROGRAM),
        }
    }
}

impl Darling for SystemDarling {
    fn program(&self) -> &Path {
        &self.program
    }

    fn run(&self, prefix: &Path, args: &[&str]) -> Result<CommandOutcome> {
        let output = Command::new(&self.program)
            .args(args)
            .env(PREFIX_ENV, prefix)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|source| Error::DarlingUnavailable {
                program: self.program.clone(),
                source,
            })?;

        Ok(CommandOutcome {
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_override_is_named_for_the_platform() {
        assert_eq!(
            PROGRAM_ENV, "DP_DARLING",
            "the override keeps the platform prefix; DG_DARLING was the old brand"
        );
    }

    /// A path that cannot exist, so a spawn against it always fails.
    const MISSING_PROGRAM: &str = "/nonexistent/darling-platform-tests/darling";

    #[test]
    fn new_and_from_env_expose_the_program() {
        let darling = SystemDarling::new("/opt/darling/bin/darling");
        assert_eq!(darling.program(), Path::new("/opt/darling/bin/darling"));

        // `from_env` reads the process environment, so only the default shape is
        // asserted here: whatever it resolves is a usable program path, and never
        // an empty one. Tests never mutate the process environment.
        let from_env = SystemDarling::from_env();
        assert!(
            !from_env.program().as_os_str().is_empty(),
            "from_env must always resolve a program"
        );
    }

    #[test]
    fn a_missing_program_is_reported_as_unavailable() {
        let darling = SystemDarling::new(MISSING_PROGRAM);

        let error = darling
            .run(Path::new("/tmp/prefix"), &["shell", "/usr/bin/true"])
            .expect_err("a missing program cannot be spawned");

        match error {
            Error::DarlingUnavailable { program, source } => {
                assert_eq!(program, PathBuf::from(MISSING_PROGRAM));
                assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
            }
            other => panic!("expected DarlingUnavailable, got {other:?}"),
        }
    }

    #[test]
    fn dprefix_is_exported_to_the_child_and_the_exit_code_is_not_interpreted() {
        // `/bin/sh` stands in for Darling: what matters here is the environment the
        // adapter builds, not the program it runs.
        let darling = SystemDarling::new("/bin/sh");
        let prefix = Path::new("/tmp/a prefix with spaces");

        let outcome = darling
            .run(prefix, &["-c", "printf %s \"$DPREFIX\""])
            .expect("the child must start");

        assert_eq!(outcome.exit_code, Some(0));
        assert_eq!(outcome.stdout, "/tmp/a prefix with spaces");

        let failing = darling
            .run(Path::new("/tmp/prefix"), &["-c", "exit 3"])
            .expect("a non-zero exit is still a successful spawn");
        assert_eq!(failing.exit_code, Some(3));
        assert!(failing.stdout.is_empty());
    }

    #[test]
    fn the_adapter_captures_standard_error() {
        let darling = SystemDarling::new("/bin/sh");

        let outcome = darling
            .run(Path::new("/tmp/prefix"), &["-c", "printf broken >&2"])
            .expect("the child must start");

        assert_eq!(outcome.exit_code, Some(0));
        assert_eq!(outcome.stderr, "broken");
    }
}
