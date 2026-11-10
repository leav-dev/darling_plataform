//! Typed errors of the Shared Prefix Manager.
//!
//! Every failure the crate can report is a variant of [`Error`]: the crate never
//! returns stringly errors, so callers can branch on the cause instead of
//! matching messages. The messages are written for a human operator and are
//! actionable: each one names the offending path and what to do next. They never
//! include environment contents beyond the paths involved, and never credentials.

use std::fmt;
use std::path::PathBuf;

use crate::state::Phase;

/// Convenient result alias for the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Every failure the Shared Prefix Manager can report.
#[derive(Debug)]
pub enum Error {
    /// Neither `DPREFIX` nor `HOME`/`XDG_DATA_HOME` yielded a usable path.
    LocationUnresolved { reason: String },
    /// `DPREFIX` is set but is not an absolute path.
    PrefixNotAbsolute { path: PathBuf },
    /// `DPREFIX` is longer than the limit Darling accepts (255 bytes).
    PrefixPathTooLong { path: PathBuf, max_bytes: usize },
    /// `DPREFIX` points at the user's conventional `~/.darling` prefix.
    ConventionalPrefixConflict { path: PathBuf },
    /// Another operation holds the prefix lock.
    PrefixBusy { path: PathBuf },
    /// An I/O operation on `path` failed.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The state file exists but is not valid JSON for [`crate::state::StateRecord`].
    StateJson {
        path: PathBuf,
        source: serde_json::Error,
    },
    /// The state file was written by a newer schema than this build understands.
    StateSchemaUnsupported {
        path: PathBuf,
        found: u32,
        supported: u32,
    },
    /// The Darling program is missing or cannot be executed.
    DarlingUnavailable {
        program: PathBuf,
        source: std::io::Error,
    },
    /// Darling ran but reported a failure.
    DarlingFailed {
        program: PathBuf,
        exit_code: Option<i32>,
        stderr: String,
    },
    /// A previous lifecycle operation never finished.
    InterruptedOperation { phase: Phase, operation: String },
    /// Refusing to stop the environment while managed instances are still alive.
    ActiveProcesses { count: u32 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LocationUnresolved { reason } => write!(
                f,
                "cannot determine the shared prefix location: {reason}. \
                 Set DPREFIX to an absolute path, or set HOME (or an absolute XDG_DATA_HOME) \
                 so the platform data directory can be located"
            ),
            Self::PrefixNotAbsolute { path } => write!(
                f,
                "DPREFIX '{}' is not an absolute path, and Darling requires an absolute prefix; \
                 set DPREFIX to a path starting at '/', for example \
                 \"$HOME/.local/share/darling-platform/prefix\"",
                path.display()
            ),
            Self::PrefixPathTooLong { path, max_bytes } => write!(
                f,
                "DPREFIX '{}' is longer than the {max_bytes} byte limit Darling accepts for a \
                 prefix; point DPREFIX at a shorter directory",
                path.display()
            ),
            Self::ConventionalPrefixConflict { path } => write!(
                f,
                "DPREFIX '{}' is the user's conventional Darling prefix (~/.darling), which the \
                 platform must never share or modify; unset DPREFIX to use the platform prefix, \
                 or point it at a different directory",
                path.display()
            ),
            Self::PrefixBusy { path } => write!(
                f,
                "another operation currently holds the prefix lock '{}'; wait for it to finish \
                 and retry",
                path.display()
            ),
            Self::Io { path, source } => {
                write!(f, "I/O error on '{}': {source}", path.display())
            }
            Self::StateJson { path, source } => write!(
                f,
                "the prefix state file '{}' is not valid platform state: {source}; remove it to \
                 let the platform rebuild the state from scratch",
                path.display()
            ),
            Self::StateSchemaUnsupported {
                path,
                found,
                supported,
            } => write!(
                f,
                "the prefix state file '{}' uses schema version {found}, but this build supports \
                 at most version {supported}; upgrade the platform, or remove the file to rebuild \
                 the state from scratch",
                path.display()
            ),
            Self::DarlingUnavailable { program, source } => write!(
                f,
                "cannot run Darling '{program}': {source}; install Darling, or point DP_DARLING at \
                 the darling executable, and retry",
                program = program.display()
            ),
            Self::DarlingFailed {
                program,
                exit_code,
                stderr,
            } => {
                let program = program.display();
                let diagnostic = stderr.trim();
                match (exit_code, diagnostic.is_empty()) {
                    (Some(code), false) => write!(
                        f,
                        "Darling '{program}' failed with exit code {code}: {diagnostic}"
                    ),
                    (Some(code), true) => write!(
                        f,
                        "Darling '{program}' failed with exit code {code} and printed no diagnostic"
                    ),
                    (None, false) => write!(
                        f,
                        "Darling '{program}' did not report an exit code (it was killed by a \
                         signal): {diagnostic}"
                    ),
                    (None, true) => write!(
                        f,
                        "Darling '{program}' did not report an exit code (it was killed by a \
                         signal) and printed no diagnostic"
                    ),
                }
            }
            Self::InterruptedOperation { phase, operation } => write!(
                f,
                "the previous '{operation}' operation never finished: the persisted phase is still \
                 '{phase:?}'; run the operation again, or run \"dp prefix stop\" to reach a known \
                 phase before continuing"
            ),
            Self::ActiveProcesses { count } => write!(
                f,
                "{count} managed instance(s) are still running inside the environment; stop them \
                 first, because \"darling shutdown\" terminates every process inside the container"
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::StateJson { source, .. } => Some(source),
            Self::DarlingUnavailable { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_variants() -> Vec<Error> {
        vec![
            Error::LocationUnresolved {
                reason: String::from("HOME and XDG_DATA_HOME are unset"),
            },
            Error::PrefixNotAbsolute {
                path: PathBuf::from("prefix"),
            },
            Error::PrefixPathTooLong {
                path: PathBuf::from("/very/long"),
                max_bytes: 255,
            },
            Error::ConventionalPrefixConflict {
                path: PathBuf::from("/home/dev/.darling"),
            },
            Error::PrefixBusy {
                path: PathBuf::from("/data/prefix.lock"),
            },
            Error::Io {
                path: PathBuf::from("/data/state.json"),
                source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            },
            Error::StateJson {
                path: PathBuf::from("/data/state.json"),
                source: serde_json::from_str::<serde_json::Value>("{").unwrap_err(),
            },
            Error::StateSchemaUnsupported {
                path: PathBuf::from("/data/state.json"),
                found: 2,
                supported: 1,
            },
            Error::DarlingUnavailable {
                program: PathBuf::from("/opt/darling/bin/darling"),
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            },
            Error::DarlingFailed {
                program: PathBuf::from("/opt/darling/bin/darling"),
                exit_code: Some(1),
                stderr: String::from("darling: cannot create the prefix"),
            },
            Error::InterruptedOperation {
                phase: Phase::Starting,
                operation: String::from("start"),
            },
            Error::ActiveProcesses { count: 2 },
        ]
    }

    #[test]
    fn messages_are_actionable_and_name_the_path() {
        for error in all_variants() {
            let message = error.to_string();
            assert!(
                !message.trim().is_empty(),
                "every error must carry a message: {error:?}"
            );
            match &error {
                Error::LocationUnresolved { .. } => {
                    assert!(message.contains("DPREFIX"), "unexpected message: {message}");
                }
                Error::PrefixNotAbsolute { path }
                | Error::PrefixPathTooLong { path, .. }
                | Error::ConventionalPrefixConflict { path }
                | Error::PrefixBusy { path }
                | Error::Io { path, .. }
                | Error::StateJson { path, .. }
                | Error::StateSchemaUnsupported { path, .. } => assert!(
                    message.contains(&path.display().to_string()),
                    "message must name the path: {message}"
                ),
                Error::DarlingUnavailable { program, .. }
                | Error::DarlingFailed { program, .. } => assert!(
                    message.contains(&program.display().to_string()),
                    "message must name the program: {message}"
                ),
                Error::InterruptedOperation { operation, .. } => assert!(
                    message.contains(operation.as_str()),
                    "message must name the interrupted operation: {message}"
                ),
                Error::ActiveProcesses { count } => assert!(
                    message.contains(&count.to_string()),
                    "message must name the instance count: {message}"
                ),
            }
        }
    }

    #[test]
    fn darling_failure_messages_report_the_exit_code_and_the_diagnostic() {
        let failed = Error::DarlingFailed {
            program: PathBuf::from("/opt/darling/bin/darling"),
            exit_code: Some(7),
            stderr: String::from("darling: cannot create the prefix\n"),
        };
        let message = failed.to_string();
        assert!(message.contains("exit code 7"), "message: {message}");
        assert!(
            message.contains("darling: cannot create the prefix"),
            "the child diagnostic must be preserved: {message}"
        );
        assert!(
            !message.contains("\n"),
            "the trailing newline of the diagnostic must be trimmed: {message:?}"
        );

        let signaled = Error::DarlingFailed {
            program: PathBuf::from("/opt/darling/bin/darling"),
            exit_code: None,
            stderr: String::new(),
        };
        let message = signaled.to_string();
        assert!(
            message.contains("did not report an exit code"),
            "a killed child must be reported as such: {message}"
        );
    }

    #[test]
    fn source_is_exposed_only_for_wrapped_failures() {
        for error in all_variants() {
            let has_source = std::error::Error::source(&error).is_some();
            let expected = matches!(
                error,
                Error::Io { .. } | Error::StateJson { .. } | Error::DarlingUnavailable { .. }
            );
            assert_eq!(has_source, expected, "unexpected source for {error:?}");
        }
    }
}
