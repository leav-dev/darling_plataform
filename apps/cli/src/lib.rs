//! Command-line entry point of Darling Platform.
//!
//! Responsibility: parse arguments, dispatch to a command and render its
//! outcome. Every command is a thin adapter; the behaviour lives in the module
//! crates that the command calls.
//!
//! Not responsible for: business logic, DPREFIX access or IPC. Commands are
//! added here only once the service they call exists and is verifiable.
//!
//! The environment is injected as [`CliEnv`], so `execute` never reads the
//! process environment: resolution and Darling access stay reproducible in tests.

use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;

use dp_shared_prefix::{
    CheckStatus, CommandOutcome, Darling, Inspection, Phase, PrefixEnv, PrefixLocation,
    PrefixManager, SystemDarling, Validation,
};

/// Text shown by `dg --help` and by any call that needs the command list.
pub const USAGE: &str = "\
usage: dg <command> [options]

Commands:
  dg prefix status    Report the shared DPREFIX: location, phase and warnings
  dg prefix validate  Validate the shared DPREFIX and print one line per check
  dg prefix start     Ensure the environment is up, creating the prefix on first use
  dg prefix stop      Shut the environment down; refuses while managed instances run

Options:
      --json                      Print the command outcome as JSON (status, validate)
      --active-instances <count>  Managed instances still running (stop only, default 0)
  -h, --help                      Print this help text
  -V, --version                   Print version information
";

/// Text shown when `dg prefix` is called without a usable subcommand.
const PREFIX_USAGE: &str = "\
usage: dg prefix <subcommand> [options]

Subcommands:
  status    Report the shared DPREFIX: location, phase and warnings
  validate  Validate the shared DPREFIX and print one line per check
  start     Ensure the environment is up, creating the prefix on first use
  stop      Shut the environment down; refuses while managed instances run

Options:
      --json                      Print the outcome as JSON (status, validate)
      --active-instances <count>  Managed instances still running (stop only)
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

    /// Renders a runtime or library failure: code 1, nothing on standard output.
    fn failure(message: impl std::fmt::Display) -> Self {
        Self {
            code: 1,
            stdout: String::new(),
            stderr: format!("error: {message}\n"),
        }
    }

    fn usage_error(message: impl AsRef<str>) -> Self {
        Self::usage_error_with(message, USAGE)
    }

    /// Renders a usage error against the usage block that documents the command.
    fn usage_error_with(message: impl AsRef<str>, usage: &str) -> Self {
        Self {
            code: 2,
            stdout: String::new(),
            stderr: format!("error: {}\n\n{usage}", message.as_ref()),
        }
    }
}

/// Everything a command needs from outside the process.
///
/// Both halves are injected so that `execute` is a pure function of its
/// arguments plus this value: the real process environment is read in exactly one
/// place, [`CliEnv::from_process`].
pub struct CliEnv {
    /// Environment the shared prefix location is resolved from.
    pub prefix: PrefixEnv,
    /// Adapter that runs the verified Darling commands.
    ///
    /// Shared instead of boxed alone because `execute` borrows this value while
    /// the manager needs an owned adapter, and `Darling` is not `Clone`.
    pub darling: Arc<dyn Darling>,
}

impl CliEnv {
    /// Reads the prefix environment and the Darling program from the process.
    pub fn from_process() -> Self {
        Self {
            prefix: PrefixEnv::from_process(),
            darling: Arc::new(SystemDarling::from_env()),
        }
    }
}

/// Shared view of the caller's [`Darling`], so a manager can be built over the
/// injected adapter while the environment keeps owning it.
struct SharedDarling(Arc<dyn Darling>);

impl Darling for SharedDarling {
    fn program(&self) -> &Path {
        self.0.program()
    }

    fn run(&self, prefix: &Path, args: &[&str]) -> dp_shared_prefix::Result<CommandOutcome> {
        self.0.run(prefix, args)
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
pub fn execute<I, S>(args: I, env: &CliEnv) -> Response
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut argv: Vec<String> = Vec::new();
    for arg in args {
        let Some(arg) = arg.as_ref().to_str() else {
            return Response::usage_error("argument is not valid UTF-8");
        };
        argv.push(arg.to_owned());
    }

    let mut positionals: Vec<String> = Vec::new();
    let mut json = false;
    let mut active_instances: Option<u32> = None;
    let mut index = 0;

    while index < argv.len() {
        let arg = argv[index].as_str();

        match arg {
            "-h" | "--help" => return Response::output(USAGE),
            "-V" | "--version" => return Response::output(format!("{}\n", version_line())),
            "--json" => json = true,
            "--active-instances" => {
                let Some(value) = argv.get(index + 1) else {
                    return Response::usage_error("option `--active-instances` requires a count");
                };
                match value.parse::<u32>() {
                    Ok(count) => active_instances = Some(count),
                    Err(_) => {
                        return Response::usage_error(format!(
                            "`--active-instances` needs a non-negative integer, got `{value}`"
                        ));
                    }
                }
                index += 1;
            }
            other if other.starts_with('-') => {
                return Response::usage_error(format!("unrecognized option `{other}`"));
            }
            other => positionals.push(other.to_owned()),
        }

        index += 1;
    }

    let Some(command) = positionals.first().map(String::as_str) else {
        return Response {
            code: 2,
            stdout: String::new(),
            stderr: format!("error: a command is required\n\n{USAGE}"),
        };
    };

    match command {
        "prefix" => execute_prefix(&positionals[1..], json, active_instances, env),
        other => Response::usage_error(format!("unknown command `{other}`")),
    }
}

/// Dispatches the `dg prefix` subcommands.
fn execute_prefix(
    args: &[String],
    json: bool,
    active_instances: Option<u32>,
    env: &CliEnv,
) -> Response {
    let Some(subcommand) = args.first().map(String::as_str) else {
        return Response::usage_error_with("a `dg prefix` subcommand is required", PREFIX_USAGE);
    };

    if let Some(extra) = args.get(1) {
        return Response::usage_error_with(
            format!("unexpected argument `{extra}` for `dg prefix {subcommand}`"),
            PREFIX_USAGE,
        );
    }

    match subcommand {
        "status" | "validate" if active_instances.is_some() => Response::usage_error_with(
            format!("`--active-instances` does not apply to `dg prefix {subcommand}`"),
            PREFIX_USAGE,
        ),
        "start" | "stop" if json => Response::usage_error_with(
            format!("`--json` does not apply to `dg prefix {subcommand}` yet"),
            PREFIX_USAGE,
        ),
        "status" => with_manager(env, |manager| status(manager, json)),
        "validate" => with_manager(env, |manager| validate(manager, json)),
        "start" => with_manager(env, start),
        "stop" => with_manager(env, |manager| stop(manager, active_instances.unwrap_or(0))),
        other => Response::usage_error_with(
            format!("unknown `dg prefix` subcommand `{other}`"),
            PREFIX_USAGE,
        ),
    }
}

/// Ensures the environment is up: Darling creates the prefix on first use.
fn start(manager: &PrefixManager) -> Response {
    match manager.start() {
        Ok(phase) => Response::output(format!("started: {}\n", phase_name(phase))),
        Err(error) => Response::failure(error),
    }
}

/// Shuts the environment down, refusing while managed instances are alive.
fn stop(manager: &PrefixManager, active_instances: u32) -> Response {
    match manager.stop(active_instances) {
        Ok(phase) => Response::output(format!("stopped: {}\n", phase_name(phase))),
        Err(error) => Response::failure(error),
    }
}

/// Runs `command` over a manager built from the injected environment.
///
/// A location that cannot be resolved is a runtime failure of the command, not a
/// usage error: the operator's arguments may be perfect while `DPREFIX` and
/// `HOME` are not.
fn with_manager(env: &CliEnv, command: impl FnOnce(&PrefixManager) -> Response) -> Response {
    match PrefixLocation::resolve(&env.prefix) {
        Ok(location) => {
            let darling = SharedDarling(Arc::clone(&env.darling));
            let manager = PrefixManager::new(location, Box::new(darling));
            command(&manager)
        }
        Err(error) => Response::failure(error),
    }
}

/// Renders `dg prefix status`.
fn status(manager: &PrefixManager, json: bool) -> Response {
    let inspection = match manager.inspect() {
        Ok(inspection) => inspection,
        Err(error) => return Response::failure(error),
    };

    if json {
        return match serde_json::to_string(&inspection) {
            Ok(document) => Response::output(format!("{document}\n")),
            Err(error) => {
                Response::failure(format!("cannot render the prefix status as JSON: {error}"))
            }
        };
    }

    Response::output(render_status(&inspection))
}

/// Renders the inspection as `key: value` lines plus one line per warning.
fn render_status(inspection: &Inspection) -> String {
    let mut lines = vec![
        format!("prefix: {}", inspection.prefix.display()),
        format!("exists: {}", yes_no(inspection.exists)),
        format!("state: {}", phase_name(inspection.phase)),
        format!(
            "interrupted: {}",
            inspection.interrupted.as_deref().unwrap_or("none")
        ),
        format!("busy: {}", yes_no(inspection.busy)),
        format!(
            "pin: {} (commit {})",
            inspection.darling_pin.version,
            &inspection.darling_pin.commit[..12.min(inspection.darling_pin.commit.len())]
        ),
        format!(
            "darling: {}{}",
            inspection.darling_program.display(),
            missing_marker(inspection)
        ),
    ];
    lines.extend(
        inspection
            .warnings
            .iter()
            .map(|warning| format!("warning: {warning}")),
    );

    joined_lines(lines)
}

/// Renders `dg prefix validate`.
///
/// Exit code 1 when a check failed, so the command can guard a script; a warning
/// never fails the validation, because an absent prefix is created on demand.
fn validate(manager: &PrefixManager, json: bool) -> Response {
    let validation = match manager.validate() {
        Ok(validation) => validation,
        Err(error) => return Response::failure(error),
    };
    let code = if validation.passed() { 0 } else { 1 };

    if json {
        return match json_validation(&validation) {
            Ok(document) => Response {
                code,
                stdout: document,
                stderr: String::new(),
            },
            Err(error) => Response::failure(format!(
                "cannot render the prefix validation as JSON: {error}"
            )),
        };
    }

    Response {
        code,
        stdout: render_validation(&validation),
        stderr: String::new(),
    }
}

/// Renders one `status name detail` line per check plus the final result.
fn render_validation(validation: &Validation) -> String {
    let mut lines: Vec<String> = validation
        .checks
        .iter()
        .map(|check| {
            format!(
                "{} {} {}",
                check_status_name(check.status),
                check.name,
                check.detail
            )
        })
        .collect();
    lines.push(String::from(if validation.passed() {
        "result: ok"
    } else {
        "result: failed"
    }));

    joined_lines(lines)
}

/// Builds the validation report as `{"passed": bool, "checks": [...]}`.
fn json_validation(validation: &Validation) -> Result<String, serde_json::Error> {
    let checks = serde_json::to_value(&validation.checks)?;
    let document = serde_json::json!({
        "passed": validation.passed(),
        "checks": checks,
    });

    Ok(format!("{document}\n"))
}

/// Joins `lines` into the single trailing-newline-terminated block the CLI emits.
fn joined_lines(lines: Vec<String>) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

/// Suffix appended to the Darling program when the manager could not find it.
fn missing_marker(inspection: &Inspection) -> &'static str {
    if inspection.darling_present {
        ""
    } else {
        " (not found)"
    }
}

/// Snake_case phase name, taken from the serialization the state file persists so
/// the text and JSON outputs can never drift from the stored contract.
fn phase_name(phase: Phase) -> String {
    match serde_json::to_value(phase) {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::from("unknown"),
    }
}

/// Lowercase check status, taken from the same serialization `--json` prints.
fn check_status_name(status: CheckStatus) -> String {
    match serde_json::to_value(status) {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::from("unknown"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use dp_shared_prefix::{StateRecord, StateStore};

    use super::*;

    /// A program path that cannot exist, so `status` is deterministic.
    const MISSING_PROGRAM: &str = "/nonexistent/darling-platform-tests/darling";

    /// A Darling double. `status` and `validate` never run a command, so a call
    /// here would mean a read-only command spawned a process.
    struct FakeDarling;

    impl Darling for FakeDarling {
        fn program(&self) -> &Path {
            Path::new(MISSING_PROGRAM)
        }

        fn run(&self, _prefix: &Path, _args: &[&str]) -> dp_shared_prefix::Result<CommandOutcome> {
            panic!("status and validate must never run a Darling command");
        }
    }

    /// An isolated environment: `<root>/prefix` as `DPREFIX` and `<root>/data` as
    /// the XDG data home root, so nothing touches the developer's real prefix.
    fn env_at(root: &Path) -> CliEnv {
        CliEnv {
            prefix: PrefixEnv {
                dprefix: Some(root.join("prefix").into_os_string()),
                xdg_data_home: Some(root.join("data").into_os_string()),
                home: None,
            },
            darling: Arc::new(FakeDarling),
        }
    }

    fn isolated() -> (tempfile::TempDir, CliEnv) {
        let dir = tempfile::tempdir().expect("temp dir");
        let env = env_at(dir.path());
        (dir, env)
    }

    /// An isolated environment whose Darling is a real script at `program`.
    fn env_with_program(root: &Path, program: &Path) -> CliEnv {
        CliEnv {
            prefix: PrefixEnv {
                dprefix: Some(root.join("prefix").into_os_string()),
                xdg_data_home: Some(root.join("data").into_os_string()),
                home: None,
            },
            darling: Arc::new(SystemDarling::new(program)),
        }
    }

    /// Writes an executable shell script that appends its arguments and the
    /// `DPREFIX` it received to `<root>/calls.log`, then exits with `exit_code`.
    fn recording_script(root: &Path, exit_code: i32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let script = root.join(format!("darling-{exit_code}"));
        let log = root.join("calls.log");
        let body = format!(
            "#!/bin/sh\n\
             printf 'argv:%s\\n' \"$*\" >> '{}'\n\
             printf 'prefix:%s\\n' \"$DPREFIX\" >> '{}'\n\
             exit {exit_code}\n",
            log.display(),
            log.display()
        );

        std::fs::write(&script, body).expect("script");
        let mut permissions = std::fs::metadata(&script).expect("metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).expect("permissions");
        script
    }

    /// Everything the recorded Darling calls logged, or an empty string when the
    /// program was never invoked.
    fn recorded_calls(root: &Path) -> String {
        std::fs::read_to_string(root.join("calls.log")).unwrap_or_default()
    }

    /// The persisted phase and operation, when there is a state record.
    fn persisted(env: &CliEnv) -> Option<StateRecord> {
        StateStore::new(location(env).state_path())
            .load()
            .expect("the state file must be readable")
    }

    /// The location the isolated environment resolves to.
    fn location(env: &CliEnv) -> PrefixLocation {
        PrefixLocation::resolve(&env.prefix).expect("the isolated test environment must resolve")
    }

    fn json_document(text: &str) -> serde_json::Value {
        serde_json::from_str(text).expect("the CLI must print exactly one JSON object")
    }

    #[test]
    fn version_reports_the_crate_version() {
        let (_dir, env) = isolated();
        let response = execute(["--version"], &env);

        assert_eq!(response.code, 0);
        assert_eq!(
            response.stdout,
            format!("dg {}\n", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(response.stderr, "");
    }

    #[test]
    fn short_version_flag_matches_the_long_one() {
        let (_dir, env) = isolated();
        assert_eq!(execute(["-V"], &env), execute(["--version"], &env));
    }

    #[test]
    fn help_lists_the_usage_line() {
        let (_dir, env) = isolated();
        let response = execute(["--help"], &env);

        assert_eq!(response.code, 0);
        assert!(response.stdout.starts_with("usage: dg <command>"));
        assert_eq!(response.stderr, "");
    }

    #[test]
    fn short_help_flag_matches_the_long_one() {
        let (_dir, env) = isolated();
        assert_eq!(execute(["-h"], &env), execute(["--help"], &env));
    }

    #[test]
    fn unknown_command_is_rejected_and_prints_no_stdout() {
        let (_dir, env) = isolated();
        let response = execute(["frobnicate"], &env);

        assert_eq!(response.code, 2);
        assert_eq!(response.stdout, "");
        assert!(response.stderr.contains("unknown command `frobnicate`"));
        assert!(response.stderr.contains("usage: dg <command>"));
    }

    #[test]
    fn unrecognized_option_is_rejected() {
        let (_dir, env) = isolated();
        let response = execute(["--frobnicate"], &env);

        let message = response.stderr.as_str();

        assert_eq!(response.code, 2);
        assert_eq!(response.stdout, "");
        assert!(message.contains("unrecognized option `--frobnicate`"));
    }

    #[test]
    fn missing_command_is_rejected() {
        let (_dir, env) = isolated();
        let response = execute(std::iter::empty::<&str>(), &env);

        assert_eq!(response.code, 2);
        assert_eq!(response.stdout, "");
        assert!(response.stderr.contains("a command is required"));
    }

    #[test]
    fn non_utf8_argument_is_rejected_instead_of_being_lossily_converted() {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;

            let (_dir, env) = isolated();
            let invalid = OsStr::from_bytes(b"--nope\xff");
            let response = execute([invalid], &env);

            assert_eq!(response.code, 2);
            assert!(response.stderr.contains("not valid UTF-8"));
        }
    }

    #[test]
    fn help_lists_the_prefix_subcommands_and_their_state() {
        let (_dir, env) = isolated();
        let response = execute(["--help"], &env);

        let text = response.stdout.as_str();
        for subcommand in ["status", "validate", "start", "stop"] {
            assert!(
                text.contains(&format!("dg prefix {subcommand}")),
                "`dg prefix {subcommand}` must be listed: {text}"
            );
        }
        assert!(
            text.contains("--active-instances"),
            "the help text must document the stop option: {text}"
        );
    }

    #[test]
    fn status_reports_an_absent_prefix_without_creating_it() {
        let (_dir, env) = isolated();
        let location = location(&env);

        let response = execute(["prefix", "status"], &env);

        assert_eq!(response.code, 0);
        assert_eq!(response.stderr, "");
        let text = response.stdout.as_str();
        assert!(
            text.contains(&format!("prefix: {}\n", location.path().display())),
            "the prefix path must be reported: {text}"
        );
        assert!(text.contains("exists: no\n"), "unexpected output: {text}");
        assert!(
            text.contains("state: not_initialized\n"),
            "unexpected output: {text}"
        );
        assert!(
            text.contains("interrupted: none\n"),
            "unexpected output: {text}"
        );
        assert!(text.contains("busy: no\n"), "unexpected output: {text}");
        assert!(
            text.contains("pin: v0.1.20260608 (commit e947f0d5a3c6)\n"),
            "the pinned Darling release must be reported: {text}"
        );
        assert!(
            text.contains("(not found)"),
            "the missing Darling program must be marked: {text}"
        );
        assert!(text.ends_with('\n'), "the output must end with a newline");
        assert!(
            !location.path().exists(),
            "status must never create the prefix directory (S5)"
        );
        assert!(
            !location.state_path().exists(),
            "status must never create the state file"
        );
    }

    #[test]
    fn status_json_reports_the_absent_prefix() {
        let (_dir, env) = isolated();
        let location = location(&env);

        let response = execute(["prefix", "status", "--json"], &env);

        assert_eq!(response.code, 0);
        assert_eq!(response.stderr, "");
        assert!(response.stdout.ends_with('\n'));

        let document = json_document(&response.stdout);
        assert_eq!(
            document["prefix"].as_str(),
            Some(location.path().to_string_lossy().as_ref())
        );
        assert_eq!(document["exists"].as_bool(), Some(false));
        assert_eq!(document["phase"].as_str(), Some("not_initialized"));
        assert_eq!(document["interrupted"].as_str(), None);
        assert_eq!(document["busy"].as_bool(), Some(false));
        assert_eq!(
            document["darling_pin"]["version"].as_str(),
            Some("v0.1.20260608")
        );
        assert_eq!(document["darling_program"].as_str(), Some(MISSING_PROGRAM));
        assert!(
            document["warnings"].is_array(),
            "warnings must be an array: {}",
            response.stdout
        );
    }

    #[test]
    fn status_reports_a_running_phase_without_the_prefix_directory() {
        let (_dir, env) = isolated();
        let location = location(&env);
        std::fs::create_dir_all(location.data_dir()).expect("create the platform data dir");
        StateStore::new(location.state_path())
            .save(&StateRecord::new(Phase::Running, None, 1))
            .expect("seed the running state");

        let response = execute(["prefix", "status"], &env);

        assert_eq!(response.code, 0);
        assert_eq!(response.stderr, "");
        let text = response.stdout.as_str();
        assert!(text.contains("exists: no\n"), "unexpected output: {text}");
        assert!(
            text.contains("state: running\n"),
            "unexpected output: {text}"
        );
        assert!(
            text.lines().any(|line| line.starts_with("warning: ")),
            "a running phase without a prefix must warn: {text}"
        );
    }

    #[test]
    fn validate_accepts_an_absent_prefix() {
        let (_dir, env) = isolated();

        let response = execute(["prefix", "validate"], &env);

        assert_eq!(response.code, 0);
        assert_eq!(response.stderr, "");
        let text = response.stdout.as_str();
        assert!(
            text.lines()
                .any(|line| line.starts_with("warn prefix_exists ")),
            "an absent prefix must be warned about, not failed: {text}"
        );
        assert!(text.ends_with("result: ok\n"), "unexpected output: {text}");
    }

    #[test]
    fn validate_json_reports_an_absent_prefix_as_passed() {
        let (_dir, env) = isolated();

        let response = execute(["prefix", "validate", "--json"], &env);

        assert_eq!(response.code, 0);
        assert_eq!(response.stderr, "");
        assert!(response.stdout.ends_with('\n'));

        let document = json_document(&response.stdout);
        assert_eq!(document["passed"].as_bool(), Some(true));
        let checks = document["checks"]
            .as_array()
            .unwrap_or_else(|| panic!("checks must be an array: {}", response.stdout));
        let exists = checks
            .iter()
            .find(|check| check["name"].as_str() == Some("prefix_exists"))
            .unwrap_or_else(|| {
                panic!(
                    "the prefix_exists check must be reported: {}",
                    response.stdout
                )
            });
        assert_eq!(exists["status"].as_str(), Some("warn"));
        assert!(
            exists["detail"]
                .as_str()
                .is_some_and(|detail| !detail.is_empty()),
            "every check must carry a detail: {}",
            response.stdout
        );
    }

    #[test]
    fn validate_fails_when_the_prefix_path_is_a_regular_file() {
        let (_dir, env) = isolated();
        let location = location(&env);
        std::fs::write(location.path(), b"not a directory").expect("create the prefix file");

        let response = execute(["prefix", "validate"], &env);

        assert_eq!(response.code, 1);
        assert_eq!(response.stderr, "");
        let text = response.stdout.as_str();
        assert!(
            text.lines()
                .any(|line| line.starts_with("fail is_directory ")),
            "a non-directory prefix must fail the is_directory check: {text}"
        );
        assert!(
            text.ends_with("result: failed\n"),
            "unexpected output: {text}"
        );

        let json_response = execute(["prefix", "validate", "--json"], &env);
        assert_eq!(json_response.code, 1);
        let document = json_document(&json_response.stdout);
        assert_eq!(document["passed"].as_bool(), Some(false));
    }

    #[test]
    fn start_runs_darling_shell_true_and_records_running() {
        let dir = tempfile::tempdir().expect("temp dir");
        let script = recording_script(dir.path(), 0);
        let env = env_with_program(dir.path(), &script);
        let prefix = location(&env).path().to_path_buf();

        let response = execute(["prefix", "start"], &env);

        assert_eq!(response.code, 0, "stderr: {}", response.stderr);
        assert_eq!(response.stdout, "started: running\n");
        let calls = recorded_calls(dir.path());
        assert!(calls.contains("argv:shell /usr/bin/true"), "calls: {calls}");
        assert!(
            calls.contains(&format!("prefix:{}", prefix.display())),
            "DPREFIX must be exported to the child: {calls}"
        );
        let record = persisted(&env).expect("start must persist a phase");
        assert_eq!(record.phase, Phase::Running);
    }

    #[test]
    fn start_failure_reports_the_exit_code_and_records_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let script = recording_script(dir.path(), 3);
        let env = env_with_program(dir.path(), &script);

        let response = execute(["prefix", "start"], &env);

        assert_eq!(response.code, 1);
        assert_eq!(response.stdout, "");
        assert!(
            response.stderr.contains("exit code 3"),
            "{}",
            response.stderr
        );
        let record = persisted(&env).expect("a failed start must persist a phase");
        assert_eq!(record.phase, Phase::Error);
        assert_eq!(record.operation.as_deref(), Some("start"));
    }

    #[test]
    fn start_without_darling_is_actionable() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join("no-darling-here");
        let env = env_with_program(dir.path(), &missing);

        let response = execute(["prefix", "start"], &env);

        assert_eq!(response.code, 1);
        assert_eq!(response.stdout, "");
        assert!(
            response.stderr.contains("no-darling-here"),
            "the failure must name the program: {}",
            response.stderr
        );
    }

    #[test]
    fn stop_refuses_while_instances_are_active() {
        let dir = tempfile::tempdir().expect("temp dir");
        let script = recording_script(dir.path(), 0);
        let env = env_with_program(dir.path(), &script);

        let response = execute(["prefix", "stop", "--active-instances", "2"], &env);

        assert_eq!(response.code, 1);
        assert_eq!(response.stdout, "");
        assert!(response.stderr.contains('2'), "{}", response.stderr);
        assert_eq!(
            recorded_calls(dir.path()),
            "",
            "darling shutdown must not run while instances are alive"
        );
        assert!(
            persisted(&env).is_none(),
            "a refused stop must not write a state record"
        );
    }

    #[test]
    fn stop_shuts_the_environment_down() {
        let dir = tempfile::tempdir().expect("temp dir");
        let script = recording_script(dir.path(), 0);
        let env = env_with_program(dir.path(), &script);

        let started = execute(["prefix", "start"], &env);
        assert_eq!(started.code, 0, "stderr: {}", started.stderr);

        let response = execute(["prefix", "stop"], &env);

        assert_eq!(response.code, 0, "stderr: {}", response.stderr);
        assert_eq!(response.stdout, "stopped: stopped\n");
        assert!(
            recorded_calls(dir.path()).contains("argv:shutdown"),
            "stop must invoke darling shutdown"
        );
        let record = persisted(&env).expect("stop must persist a phase");
        assert_eq!(record.phase, Phase::Stopped);
    }

    #[test]
    fn lifecycle_options_are_validated_as_usage_errors() {
        let (_dir, env) = isolated();

        let cases: [(&[&str], &str); 5] = [
            (
                &["prefix", "stop", "--active-instances", "abc"],
                "non-negative",
            ),
            (
                &["prefix", "stop", "--active-instances"],
                "requires a count",
            ),
            (&["prefix", "start", "--json"], "does not apply"),
            (
                &["prefix", "status", "--active-instances", "1"],
                "does not apply",
            ),
            (&["prefix", "stop", "--bogus"], "unrecognized option"),
        ];

        for (args, expected) in cases {
            let response = execute(args, &env);

            assert_eq!(response.code, 2, "{args:?} must be a usage error");
            assert_eq!(response.stdout, "", "{args:?} must not print to stdout");
            assert!(
                response.stderr.contains(expected),
                "{args:?} must explain `{expected}`: {}",
                response.stderr
            );
        }
    }

    #[test]
    fn prefix_without_a_subcommand_and_with_an_unknown_option_is_a_usage_error() {
        let (_dir, env) = isolated();

        let missing = execute(["prefix"], &env);
        assert_eq!(missing.code, 2);
        assert_eq!(missing.stdout, "");
        assert!(
            missing.stderr.contains("usage: dg prefix"),
            "unexpected stderr: {}",
            missing.stderr
        );

        let unknown_option = execute(["prefix", "status", "--bogus"], &env);
        assert_eq!(unknown_option.code, 2);
        assert_eq!(unknown_option.stdout, "");
        assert!(
            unknown_option
                .stderr
                .contains("unrecognized option `--bogus`"),
            "unexpected stderr: {}",
            unknown_option.stderr
        );
    }
}
