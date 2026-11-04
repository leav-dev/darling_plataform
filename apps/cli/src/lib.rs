//! Command-line entry point of Darling Platform.
//!
//! Responsibility: parse arguments, dispatch to a command and render its
//! outcome. Every command is a thin adapter; the behaviour lives in the module
//! crates that the command calls.
//!
//! Not responsible for: business logic, DPREFIX access or IPC. Commands are
//! added here only once the service they call exists and is verifiable.

use std::ffi::OsStr;

/// Text shown by `dg --help` and by any call that needs the command list.
pub const USAGE: &str = "\
usage: dg <command> [options]

Commands:
  (no commands are implemented yet; `dg hardware scan` arrives with the
  Hardware Intelligence Engine)

Options:
  -h, --help     Print this help text
  -V, --version  Print version information
";

/// Rendered result of one command invocation.
///
/// Keeping the response as data instead of printing from the parser makes the
/// CLI contract testable without capturing process output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// Process exit code the caller must return.
    pub code: u8,
    /// Text written to standard output.
    pub stdout: String,
    /// Text written to standard error.
    pub stderr: String,
}

impl Response {
    fn output(text: impl Into<String>) -> Self {
        Self {
            code: 0,
            stdout: text.into(),
            stderr: String::new(),
        }
    }

    fn usage_error(message: impl AsRef<str>) -> Self {
        Self {
            code: 2,
            stdout: String::new(),
            stderr: format!("error: {}\n\n{USAGE}", message.as_ref()),
        }
    }
}

/// Version line reported by `dg --version`.
pub fn version_line() -> String {
    format!("dg {}", env!("CARGO_PKG_VERSION"))
}

/// Resolve the arguments that follow the program name into a [`Response`].
///
/// Arguments arrive as [`OsStr`] because the process may receive paths that are
/// not valid UTF-8; such arguments are rejected instead of being lossily
/// converted into a different command.
pub fn execute<I, S>(args: I) -> Response
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut positionals = Vec::new();

    for arg in args {
        let Some(arg) = arg.as_ref().to_str() else {
            return Response::usage_error("argument is not valid UTF-8");
        };

        match arg {
            "-h" | "--help" => return Response::output(USAGE),
            "-V" | "--version" => return Response::output(format!("{}\n", version_line())),
            other if other.starts_with('-') => {
                return Response::usage_error(format!("unrecognized option `{other}`"));
            }
            other => positionals.push(other.to_owned()),
        }
    }

    match positionals.first() {
        Some(command) => Response::usage_error(format!("unknown command `{command}`")),
        None => Response {
            code: 2,
            stdout: String::new(),
            stderr: format!("error: a command is required\n\n{USAGE}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_reports_the_crate_version() {
        let response = execute(["--version"]);

        assert_eq!(response.code, 0);
        assert_eq!(
            response.stdout,
            format!("dg {}\n", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(response.stderr, "");
    }

    #[test]
    fn short_version_flag_matches_the_long_one() {
        assert_eq!(execute(["-V"]), execute(["--version"]));
    }

    #[test]
    fn help_lists_the_usage_line() {
        let response = execute(["--help"]);

        assert_eq!(response.code, 0);
        assert!(response.stdout.starts_with("usage: dg <command>"));
        assert_eq!(response.stderr, "");
    }

    #[test]
    fn short_help_flag_matches_the_long_one() {
        assert_eq!(execute(["-h"]), execute(["--help"]));
    }

    #[test]
    fn unknown_command_is_rejected_and_prints_no_stdout() {
        let response = execute(["frobnicate"]);

        assert_eq!(response.code, 2);
        assert_eq!(response.stdout, "");
        assert!(response.stderr.contains("unknown command `frobnicate`"));
        assert!(response.stderr.contains("usage: dg <command>"));
    }

    #[test]
    fn unrecognized_option_is_rejected() {
        let response = execute(["--frobnicate"]);

        let message = response.stderr.as_str();

        assert_eq!(response.code, 2);
        assert_eq!(response.stdout, "");
        assert!(message.contains("unrecognized option `--frobnicate`"));
    }

    #[test]
    fn missing_command_is_rejected() {
        let response = execute(std::iter::empty::<&str>());

        assert_eq!(response.code, 2);
        assert_eq!(response.stdout, "");
        assert!(response.stderr.contains("a command is required"));
    }

    #[test]
    fn non_utf8_argument_is_rejected_instead_of_being_lossily_converted() {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;

            let invalid = OsStr::from_bytes(b"--nope\xff");
            let response = execute([invalid]);

            assert_eq!(response.code, 2);
            assert!(response.stderr.contains("not valid UTF-8"));
        }
    }
}
