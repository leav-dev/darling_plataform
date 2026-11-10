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
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use dp_app_installer::{InstallRequest, InstalledApplication};
use dp_app_library::{ApplicationRecord, Library};
use dp_bundle_inspector::{Architecture, BundleMetadata};
use dp_compatibility_engine::{CompatibilityReport, RequirementKind, RequirementStatus, Verdict};
use dp_hardware_intelligence::{
    DiagnosticLevel, DisplayStatus, Distribution, HardwareReport, HostSource, Libc, LibcFamily,
    Provider, ScanStatus,
};
use dp_shared_prefix::{
    CheckStatus, CommandOutcome, Darling, Inspection, Phase, PrefixEnv, PrefixLocation, PrefixLock,
    PrefixManager, SystemDarling, Validation,
};

/// Text shown by `dp --help` and by any call that needs the command list.
pub const USAGE: &str = "\
usage: dp <command> [options]

Commands:
  dp prefix status    Report the shared DPREFIX: location, phase and warnings
  dp prefix validate  Validate the shared DPREFIX and print one line per check
  dp prefix start     Ensure the environment is up, creating the prefix on first use
  dp prefix stop      Shut the environment down; refuses while managed instances run
  dp hardware scan    Describe the host: system, CPU, memory, GPUs and displays
  dp app inspect      Inspect a .app bundle and print the metadata it declares
  dp app install      Install a .app bundle or .zip archive into the DPREFIX
  dp app list         List the applications the catalogue holds
  dp app compat       Analyse what a .app bundle requires against Darling's capabilities

Options:
      --json                      Print the command outcome as JSON (status, validate, scan, app)
      --active-instances <count>  Managed instances still running (stop only, default 0)
      --replace                   Replace an installed application (app install only)
  -h, --help                      Print this help text
  -V, --version                   Print version information
";

/// Text shown when `dp prefix` is called without a usable subcommand.
const PREFIX_USAGE: &str = "\
usage: dp prefix <subcommand> [options]

Subcommands:
  status    Report the shared DPREFIX: location, phase and warnings
  validate  Validate the shared DPREFIX and print one line per check
  start     Ensure the environment is up, creating the prefix on first use
  stop      Shut the environment down; refuses while managed instances run

Options:
      --json                      Print the outcome as JSON (status, validate)
      --active-instances <count>  Managed instances still running (stop only)
";

/// Text shown when `dp hardware` is called without a usable subcommand.
const HARDWARE_USAGE: &str = "\
usage: dp hardware <subcommand> [options]

Subcommands:
  scan    Describe the host: system, CPU, memory, GPUs and displays

Options:
      --json    Print the report as JSON
";

/// Text shown when `dp app` is called without a usable subcommand.
const APP_USAGE: &str = "\
usage: dp app <subcommand> [options]

Subcommands:
  inspect <path>  Inspect a .app bundle and print the metadata it declares
  install <path>  Install a .app bundle or .zip archive into the DPREFIX
  list            List the applications the catalogue holds
  compat <path>   Analyse what a .app bundle requires against Darling's capabilities

Options:
      --json     Print the outcome as JSON (inspect, install, list, compat)
      --replace  Replace an installed application (install only)

Exit codes of `dp app compat`:
  0  the verdict is compatible or partial
  1  the verdict is not_compatible or unknown, or the analysis failed
  2  a usage error
";

/// Name of the SQLite catalogue inside the platform data directory.
///
/// The catalogue lives with the platform's own files, outside the DPREFIX, so
/// Darling's overlay can never shadow it.
const CATALOGUE_FILE: &str = "library.sqlite3";

/// Directory inside the DPREFIX that installed bundles live in.
const APPLICATIONS_DIR: &str = "Applications";

/// Installation identifier used when the source names no usable bundle name.
const UNNAMED_APPLICATION: &str = "application";

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
    /// Host the hardware scan reads.
    ///
    /// Injected for the same reason as the other halves: the scan must be
    /// reproducible from a captured host tree instead of describing whatever
    /// machine happens to run the tests.
    pub host: Box<dyn HostSource>,
}

impl CliEnv {
    /// Reads the prefix environment and the Darling program from the process.
    pub fn from_process() -> Self {
        Self {
            prefix: PrefixEnv::from_process(),
            darling: Arc::new(SystemDarling::from_env()),
            host: Box::new(dp_hardware_intelligence::LinuxHost),
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

/// Version line reported by `dp --version`.
pub fn version_line() -> String {
    format!("dp {}", env!("CARGO_PKG_VERSION"))
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
    let mut replace = false;
    let mut index = 0;

    while index < argv.len() {
        let arg = argv[index].as_str();

        match arg {
            "-h" | "--help" => return Response::output(USAGE),
            "-V" | "--version" => return Response::output(format!("{}\n", version_line())),
            "--json" => json = true,
            "--replace" => replace = true,
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
        "prefix" => execute_prefix(&positionals[1..], json, active_instances, replace, env),
        "hardware" => execute_hardware(&positionals[1..], json, active_instances, replace, env),
        "app" => execute_app(&positionals[1..], json, active_instances, replace, env),
        other => Response::usage_error(format!("unknown command `{other}`")),
    }
}

/// Dispatches the `dp hardware` subcommands.
fn execute_hardware(
    args: &[String],
    json: bool,
    active_instances: Option<u32>,
    replace: bool,
    env: &CliEnv,
) -> Response {
    let Some(subcommand) = args.first().map(String::as_str) else {
        return Response::usage_error_with(
            "a `dp hardware` subcommand is required",
            HARDWARE_USAGE,
        );
    };

    if let Some(extra) = args.get(1) {
        return Response::usage_error_with(
            format!("unexpected argument `{extra}` for `dp hardware {subcommand}`"),
            HARDWARE_USAGE,
        );
    }

    match subcommand {
        "scan" if active_instances.is_some() => Response::usage_error_with(
            "`--active-instances` does not apply to `dp hardware scan`",
            HARDWARE_USAGE,
        ),
        "scan" if replace => Response::usage_error_with(
            "`--replace` does not apply to `dp hardware scan`",
            HARDWARE_USAGE,
        ),
        "scan" => hardware_scan(env, json),
        other => Response::usage_error_with(
            format!("unknown `dp hardware` subcommand `{other}`"),
            HARDWARE_USAGE,
        ),
    }
}

/// Renders `dp hardware scan`.
///
/// Exit code 1 when the host itself could not be described, so a script can tell
/// a degraded but usable report from a useless one. A partial report is a
/// success: the missing subsystem is named in its diagnostics.
fn hardware_scan(env: &CliEnv, json: bool) -> Response {
    let report = dp_hardware_intelligence::scan(env.host.as_ref());
    let code = if report.scan_status == ScanStatus::Failed {
        1
    } else {
        0
    };

    if json {
        return match serde_json::to_string(&report) {
            Ok(document) => Response {
                code,
                stdout: format!("{document}\n"),
                stderr: String::new(),
            },
            Err(error) => Response::failure(format!(
                "cannot render the hardware report as JSON: {error}"
            )),
        };
    }

    Response {
        code,
        stdout: render_hardware(&report),
        stderr: String::new(),
    }
}

/// Dispatches the `dp app` subcommands.
///
/// `--active-instances` counts managed instances, which only `dp prefix stop`
/// acts on, so every `app` subcommand refuses it. The options a subcommand does
/// not use are refused the same way instead of being silently ignored.
fn execute_app(
    args: &[String],
    json: bool,
    active_instances: Option<u32>,
    replace: bool,
    env: &CliEnv,
) -> Response {
    let Some(subcommand) = args.first().map(String::as_str) else {
        return Response::usage_error_with("a `dp app` subcommand is required", APP_USAGE);
    };

    if active_instances.is_some() {
        return Response::usage_error_with(
            format!("`--active-instances` does not apply to `dp app {subcommand}`"),
            APP_USAGE,
        );
    }

    match subcommand {
        "inspect" if replace => {
            Response::usage_error_with("`--replace` does not apply to `dp app inspect`", APP_USAGE)
        }
        "inspect" => match app_path_argument(args, subcommand) {
            Ok(bundle) => app_inspect(Path::new(bundle), json),
            Err(response) => response,
        },
        "install" => match app_path_argument(args, subcommand) {
            Ok(source) => app_install(Path::new(source), replace, json, env),
            Err(response) => response,
        },
        "compat" if replace => {
            Response::usage_error_with("`--replace` does not apply to `dp app compat`", APP_USAGE)
        }
        "compat" => match app_path_argument(args, subcommand) {
            Ok(bundle) => app_compat(Path::new(bundle), json),
            Err(response) => response,
        },
        "list" if replace => {
            Response::usage_error_with("`--replace` does not apply to `dp app list`", APP_USAGE)
        }
        "list" => {
            if let Some(extra) = args.get(1) {
                return Response::usage_error_with(
                    format!("unexpected argument `{extra}` for `dp app list`"),
                    APP_USAGE,
                );
            }
            app_list(json, env)
        }
        other => {
            Response::usage_error_with(format!("unknown `dp app` subcommand `{other}`"), APP_USAGE)
        }
    }
}

/// Reads the single `<path>` argument of `dp app inspect`, `dp app install` and
/// `dp app compat`.
///
/// The argument loop consumes every option before this runs, so a value that
/// still starts with `-` was meant as a path and cannot be one: it is reported
/// as a missing path instead of being handed to the inspector, the installer or
/// the compatibility engine.
fn app_path_argument<'a>(args: &'a [String], subcommand: &str) -> Result<&'a str, Response> {
    let Some(path) = args.get(1) else {
        return Err(Response::usage_error_with(
            format!("`dp app {subcommand}` requires the path of the bundle to read"),
            APP_USAGE,
        ));
    };
    if path.starts_with('-') {
        return Err(Response::usage_error_with(
            format!("`dp app {subcommand}` needs a path, but got the option `{path}`"),
            APP_USAGE,
        ));
    }
    if let Some(extra) = args.get(2) {
        return Err(Response::usage_error_with(
            format!("unexpected argument `{extra}` for `dp app {subcommand}`"),
            APP_USAGE,
        ));
    }
    Ok(path.as_str())
}

/// Renders `dp app inspect`.
///
/// Read-only by construction: the inspector only reads the bundle, so this
/// command never resolves the catalogue and never touches the platform data
/// directory, the lock file or the prefix.
fn app_inspect(bundle: &Path, json: bool) -> Response {
    let metadata = match dp_bundle_inspector::inspect(bundle) {
        Ok(metadata) => metadata,
        Err(error) => return Response::failure(error),
    };

    if json {
        return match serde_json::to_string(&metadata) {
            Ok(document) => Response::output(format!("{document}\n")),
            Err(error) => Response::failure(format!(
                "cannot render the bundle metadata as JSON: {error}"
            )),
        };
    }

    Response::output(render_bundle(&metadata))
}

/// Renders `dp app compat`.
///
/// Read-only like [`app_inspect`], with a stronger consequence: this command
/// never resolves the prefix location, the catalogue or the lock file, and the
/// engine only reads the bundle. Exit code 1 covers both a failed analysis and a
/// verdict that is not established compatibility — `not_compatible` and
/// `unknown` are equally not a green light for a script, while `compatible` and
/// `partial` are.
fn app_compat(bundle: &Path, json: bool) -> Response {
    let request = dp_compatibility_engine::AnalysisRequest {
        bundle,
        host_architecture: std::env::consts::ARCH,
    };
    let report = match dp_compatibility_engine::analyze(&request) {
        Ok(report) => report,
        Err(error) => return Response::failure(error),
    };
    let code = compat_exit_code(report.verdict);

    if json {
        return match serde_json::to_string(&report) {
            Ok(document) => Response {
                code,
                stdout: format!("{document}\n"),
                stderr: String::new(),
            },
            Err(error) => Response::failure(format!(
                "cannot render the compatibility report as JSON: {error}"
            )),
        };
    }

    Response {
        code,
        stdout: render_compat(&report),
        stderr: String::new(),
    }
}

/// Exit code the verdict carries.
///
/// Only an established compatibility is a success: `not_compatible` is a known
/// failure and `unknown` is an absence of evidence, and neither is a reason to
/// let a script continue.
fn compat_exit_code(verdict: Verdict) -> u8 {
    match verdict {
        Verdict::Compatible | Verdict::Partial => 0,
        Verdict::NotCompatible | Verdict::Unknown => 1,
    }
}

/// Renders the compatibility report as `key: value` lines plus one line per
/// requirement, the same summary style `dp hardware scan` prints.
fn render_compat(report: &CompatibilityReport) -> String {
    let mut lines = vec![
        format!("verdict: {}", verdict_name(report.verdict)),
        format!("analyser_version: {}", report.analyser_version),
        format!("profile_version: {}", report.profile_version),
    ];

    if report.requirements.is_empty() {
        lines.push(String::from("requirement: none"));
        return joined_lines(lines);
    }
    lines.extend(report.requirements.iter().map(|requirement| {
        format!(
            "requirement: {} {} {}: {}",
            requirement_status_name(requirement.status),
            requirement_kind_name(requirement.kind),
            requirement.subject,
            requirement.detail
        )
    }));

    joined_lines(lines)
}

/// Renders `dp app install`.
///
/// The prefix location and the catalogue are resolved here, and the write lock
/// the Shared Prefix Manager exposes is taken before anything is written, so an
/// installation cannot run beside another platform operation. Everything else —
/// the staging area, `<prefix>/Applications`, every validation and the two
/// catalogue rows — belongs to the installer, which is why this command creates
/// no directory of its own: a source the installer refuses leaves the prefix
/// untouched.
fn app_install(source: &Path, replace: bool, json: bool, env: &CliEnv) -> Response {
    let location = match PrefixLocation::resolve(&env.prefix) {
        Ok(location) => location,
        Err(error) => return Response::failure(error),
    };
    let _lock = match PrefixLock::try_exclusive(&location.lock_path()) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            return Response::failure(dp_shared_prefix::Error::PrefixBusy {
                path: location.lock_path(),
            });
        }
        Err(error) => return Response::failure(error),
    };
    let library = match Library::open(&catalogue_path(&location)) {
        Ok(library) => library,
        Err(error) => return Response::failure(error),
    };

    let applications_dir = location.path().join(APPLICATIONS_DIR);
    let installation_id = attempt_id(source);
    let now = dp_app_library::now_utc();
    let request = InstallRequest {
        source,
        applications_dir: &applications_dir,
        library: &library,
        installation_id: installation_id.as_str(),
        replace,
        now: now.as_str(),
    };
    let installed = match dp_app_installer::install(&request) {
        Ok(installed) => installed,
        Err(error) => return Response::failure(error),
    };

    if json {
        return match json_install(&installed) {
            Ok(document) => Response::output(document),
            Err(error) => Response::failure(format!(
                "cannot render the installed application as JSON: {error}"
            )),
        };
    }

    Response::output(render_install(&installed))
}

/// Renders `dp app list`.
///
/// The catalogue is opened because listing it is the command's whole purpose; a
/// missing catalogue is created empty, which is why an empty list is a success.
fn app_list(json: bool, env: &CliEnv) -> Response {
    let location = match PrefixLocation::resolve(&env.prefix) {
        Ok(location) => location,
        Err(error) => return Response::failure(error),
    };
    let library = match Library::open(&catalogue_path(&location)) {
        Ok(library) => library,
        Err(error) => return Response::failure(error),
    };
    let applications = match library.list_applications() {
        Ok(applications) => applications,
        Err(error) => return Response::failure(error),
    };

    if json {
        let rows = match serde_json::to_value(&applications) {
            Ok(rows) => rows,
            Err(error) => {
                return Response::failure(format!(
                    "cannot render the application list as JSON: {error}"
                ));
            }
        };
        let document = serde_json::json!({ "applications": rows });
        return Response::output(format!("{document}\n"));
    }

    Response::output(render_applications(&applications))
}

/// Renders the bundle metadata as `key: value` lines plus one per framework and
/// resource, the same summary style `dp hardware scan` prints.
fn render_bundle(metadata: &BundleMetadata) -> String {
    let mut lines = vec![
        format!("path: {}", metadata.path.display()),
        format!("bundle_id: {}", metadata.bundle_id),
        format!("name: {}", metadata.name),
    ];
    if let Some(display_name) = &metadata.display_name {
        lines.push(format!("display_name: {display_name}"));
    }
    if let Some(version) = &metadata.version {
        lines.push(format!("version: {version}"));
    }
    lines.push(format!("executable: {}", metadata.executable));
    lines.push(format!(
        "executable_path: {}",
        metadata.executable_path.display()
    ));
    if let Some(icon) = &metadata.icon_file {
        lines.push(format!("icon_file: {icon}"));
    }
    if let Some(minimum) = &metadata.minimum_system_version {
        lines.push(format!("minimum_system_version: {minimum}"));
    }
    lines.push(format!(
        "architectures: {}",
        architecture_list(&metadata.architectures)
    ));
    lines.extend(entries(&metadata.frameworks, "framework"));
    lines.extend(entries(&metadata.resources, "resource"));

    joined_lines(lines)
}

/// One `<role>: <path>` line per entry, or a single `<role>: none`.
fn entries(paths: &[String], role: &str) -> Vec<String> {
    if paths.is_empty() {
        return vec![format!("{role}: none")];
    }
    paths.iter().map(|path| format!("{role}: {path}")).collect()
}

/// Renders what one installation produced, starting with where it landed.
fn render_install(installed: &InstalledApplication) -> String {
    let metadata = &installed.metadata;
    let mut lines = vec![
        format!("installed: {}", installed.path.display()),
        format!("installation_id: {}", installed.installation_id),
        format!("format: {}", installed.format),
        format!("bundle_id: {}", metadata.bundle_id),
        format!("name: {}", metadata.name),
    ];
    if let Some(version) = &metadata.version {
        lines.push(format!("version: {version}"));
    }
    lines.push(format!(
        "architectures: {}",
        architecture_list(&metadata.architectures)
    ));

    joined_lines(lines)
}

/// Builds the installation report as one object: the attempt, where it landed
/// and the metadata of the installed copy.
///
/// Both paths go through `to_value` so a path this build cannot represent as
/// JSON text is reported as a failure instead of being silently dropped.
fn json_install(installed: &InstalledApplication) -> Result<String, serde_json::Error> {
    let path = serde_json::to_value(&installed.path)?;
    let metadata = serde_json::to_value(&installed.metadata)?;
    let document = serde_json::json!({
        "installation_id": installed.installation_id.as_str(),
        "path": path,
        "format": installed.format.as_str(),
        "metadata": metadata,
    });

    Ok(format!("{document}\n"))
}

/// Renders the catalogue as one `application:` line per row, in catalogue order.
///
/// An empty catalogue reads `applications: none` instead of printing nothing, so
/// the output keeps its single trailing newline and never looks like a command
/// that produced no output at all.
fn render_applications(applications: &[ApplicationRecord]) -> String {
    if applications.is_empty() {
        return String::from("applications: none\n");
    }
    joined_lines(applications.iter().map(describe_application).collect())
}

/// One application as `application:` followed by space-separated `field=value`
/// pairs, which stays readable when a name or a path contains spaces.
fn describe_application(application: &ApplicationRecord) -> String {
    let mut fields = vec![
        format!("name={}", application.name),
        format!("bundle_id={}", application.bundle_id),
    ];
    if let Some(version) = &application.version {
        fields.push(format!("version={version}"));
    }
    if let Some(architecture) = &application.architecture {
        fields.push(format!("architecture={architecture}"));
    }
    fields.push(format!("status={}", application.installation_status));
    fields.push(format!("path={}", application.bundle_path.display()));

    format!("application: {}", fields.join(" "))
}

/// Joins the reported architectures with the names JSON prints, or `none`.
fn architecture_list(architectures: &[Architecture]) -> String {
    if architectures.is_empty() {
        return String::from("none");
    }
    architectures
        .iter()
        .map(|architecture| json_enum_name(serde_json::to_value(architecture)))
        .collect::<Vec<String>>()
        .join(",")
}

/// The catalogue of this platform: one SQLite file in the platform data
/// directory, derived from the same location the prefix is resolved from so the
/// two can never disagree.
fn catalogue_path(location: &PrefixLocation) -> PathBuf {
    location.data_dir().join(CATALOGUE_FILE)
}

/// Derives the identifier one installation attempt is recorded under.
///
/// A failed attempt is a catalogue row too and `installations.id` is unique, so
/// the value must differ between attempts: the source name keeps the history
/// readable, the process id separates concurrent commands, the clock separates
/// runs of the same command and the counter covers what the other three cannot —
/// two attempts from one process within the same second, which is exactly how a
/// caller installs the same bundle twice.
fn attempt_id(source: &Path) -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let name = source
        .file_stem()
        .and_then(OsStr::to_str)
        .filter(|name| !name.is_empty())
        .unwrap_or(UNNAMED_APPLICATION);
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);

    format!("{name}-{seconds}-{}-{sequence}", std::process::id())
}

/// Dispatches the `dp prefix` subcommands.
fn execute_prefix(
    args: &[String],
    json: bool,
    active_instances: Option<u32>,
    replace: bool,
    env: &CliEnv,
) -> Response {
    let Some(subcommand) = args.first().map(String::as_str) else {
        return Response::usage_error_with("a `dp prefix` subcommand is required", PREFIX_USAGE);
    };

    if let Some(extra) = args.get(1) {
        return Response::usage_error_with(
            format!("unexpected argument `{extra}` for `dp prefix {subcommand}`"),
            PREFIX_USAGE,
        );
    }

    match subcommand {
        "status" | "validate" if active_instances.is_some() => Response::usage_error_with(
            format!("`--active-instances` does not apply to `dp prefix {subcommand}`"),
            PREFIX_USAGE,
        ),
        "status" | "validate" | "start" | "stop" if replace => Response::usage_error_with(
            format!("`--replace` does not apply to `dp prefix {subcommand}`"),
            PREFIX_USAGE,
        ),
        "start" | "stop" if json => Response::usage_error_with(
            format!("`--json` does not apply to `dp prefix {subcommand}` yet"),
            PREFIX_USAGE,
        ),
        "status" => with_manager(env, |manager| status(manager, json)),
        "validate" => with_manager(env, |manager| validate(manager, json)),
        "start" => with_manager(env, start),
        "stop" => with_manager(env, |manager| stop(manager, active_instances.unwrap_or(0))),
        other => Response::usage_error_with(
            format!("unknown `dp prefix` subcommand `{other}`"),
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

/// Renders `dp prefix status`.
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

/// Renders `dp prefix validate`.
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

/// Renders the hardware report as `key: value` lines plus one per diagnostic.
///
/// The text form is a summary for a human: the CPU feature list is reported as a
/// count and every detail stays available through `--json`.
fn render_hardware(report: &HardwareReport) -> String {
    let mut lines = vec![
        format!("schema_version: {}", report.schema_version),
        format!("scan_status: {}", scan_status_name(report.scan_status)),
        format!("system.architecture: {}", report.system.architecture),
        format!(
            "system.kernel: {}",
            report.system.kernel.as_deref().unwrap_or("unknown")
        ),
        format!(
            "system.distribution: {}",
            render_distribution(report.system.distribution.as_ref())
        ),
        format!("system.libc: {}", render_libc(report.system.libc.as_ref())),
    ];

    match &report.cpu {
        Some(cpu) => {
            lines.push(format!(
                "cpu.vendor: {}",
                cpu.vendor.as_deref().unwrap_or("unknown")
            ));
            lines.push(format!(
                "cpu.model: {}",
                cpu.model.as_deref().unwrap_or("unknown")
            ));
            lines.push(format!(
                "cpu.cores: {} logical, {} physical, {} socket(s)",
                cpu.logical_cores,
                count_or_unknown(cpu.physical_cores),
                count_or_unknown(cpu.sockets)
            ));
            lines.push(format!("cpu.features: {}", cpu.features.len()));
        }
        None => lines.push("cpu: unavailable".to_string()),
    }

    match &report.memory {
        Some(memory) => {
            lines.push(format!("memory.total: {}", gibibytes(memory.total_bytes)));
            if let Some(available) = memory.available_bytes {
                lines.push(format!("memory.available: {}", gibibytes(available)));
            }
            if let Some(swap) = memory.swap_total_bytes {
                lines.push(format!("memory.swap: {}", gibibytes(swap)));
            }
        }
        None => lines.push("memory: unavailable".to_string()),
    }

    if report.gpus.is_empty() {
        lines.push("gpu: none".to_string());
    }
    for gpu in &report.gpus {
        lines.push(format!(
            "gpu: {} {}:{} class {} driver {}",
            gpu.pci_address,
            gpu.vendor_id,
            gpu.device_id,
            gpu.class,
            gpu.driver.as_deref().unwrap_or("none")
        ));
    }

    if report.displays.is_empty() {
        lines.push("display: none".to_string());
    }
    for display in &report.displays {
        let enabled = match display.enabled {
            Some(true) => " enabled",
            Some(false) => " disabled",
            None => "",
        };
        let resolution = display
            .resolution
            .as_deref()
            .map(|mode| format!(" {mode}"))
            .unwrap_or_default();
        lines.push(format!(
            "display: {} {}{enabled}{resolution}",
            display.connector,
            display_status_name(display.status)
        ));
    }

    lines.extend(report.diagnostics.iter().map(|diagnostic| {
        format!(
            "diagnostic: {} {} {}: {}",
            diagnostic_level_name(diagnostic.level),
            provider_name(diagnostic.provider),
            diagnostic.code,
            diagnostic.message
        )
    }));

    joined_lines(lines)
}

/// Renders the distribution as `id [version] [(pretty name)]`.
fn render_distribution(distribution: Option<&Distribution>) -> String {
    let Some(distribution) = distribution else {
        return "unknown".to_string();
    };
    let mut text = distribution.id.clone();
    if let Some(version) = &distribution.version_id {
        text.push(' ');
        text.push_str(version);
    }
    if let Some(pretty) = &distribution.pretty_name {
        text.push_str(&format!(" ({pretty})"));
    }
    text
}

/// Renders the C library as `family (evidence path)`.
fn render_libc(libc: Option<&Libc>) -> String {
    let Some(libc) = libc else {
        return "unknown".to_string();
    };
    format!("{} ({})", libc_family_name(libc.family), libc.evidence)
}

/// Renders a byte count as a human-readable binary unit.
fn gibibytes(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

/// Renders an optional count, keeping the line readable when the kernel omits it.
fn count_or_unknown(count: Option<u32>) -> String {
    count.map_or_else(|| "unknown".to_string(), |count| count.to_string())
}

/// Machine name of one enum, taken from the serialization `--json` prints.
fn json_enum_name(value: Result<serde_json::Value, serde_json::Error>) -> String {
    match value {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::from("unknown"),
    }
}

fn scan_status_name(status: ScanStatus) -> String {
    json_enum_name(serde_json::to_value(status))
}

fn display_status_name(status: DisplayStatus) -> String {
    json_enum_name(serde_json::to_value(status))
}

fn libc_family_name(family: LibcFamily) -> String {
    json_enum_name(serde_json::to_value(family))
}

fn diagnostic_level_name(level: DiagnosticLevel) -> String {
    json_enum_name(serde_json::to_value(level))
}

fn provider_name(provider: Provider) -> String {
    json_enum_name(serde_json::to_value(provider))
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

/// Text name of the report's verdict, taken from the serialization `--json`
/// prints so the two forms can never drift.
fn verdict_name(verdict: Verdict) -> String {
    json_enum_name(serde_json::to_value(verdict))
}

fn requirement_kind_name(kind: RequirementKind) -> String {
    json_enum_name(serde_json::to_value(kind))
}

fn requirement_status_name(status: RequirementStatus) -> String {
    json_enum_name(serde_json::to_value(status))
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
    /// the Xdp data home root, so nothing touches the developer's real prefix.
    fn env_at(root: &Path) -> CliEnv {
        CliEnv {
            prefix: PrefixEnv {
                dprefix: Some(root.join("prefix").into_os_string()),
                xdg_data_home: Some(root.join("data").into_os_string()),
                home: None,
            },
            darling: Arc::new(FakeDarling),
            host: Box::new(dp_hardware_intelligence::LinuxHost),
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
            host: Box::new(dp_hardware_intelligence::LinuxHost),
        }
    }

    /// An isolated environment whose hardware scan reads `host` instead of the
    /// machine running the tests.
    fn env_with_host(root: &Path, host: Box<dyn HostSource>) -> CliEnv {
        CliEnv {
            prefix: PrefixEnv {
                dprefix: Some(root.join("prefix").into_os_string()),
                xdg_data_home: Some(root.join("data").into_os_string()),
                home: None,
            },
            darling: Arc::new(FakeDarling),
            host,
        }
    }

    /// A captured host with one discrete GPU and one connected internal panel.
    fn captured_host() -> dp_hardware_intelligence::MappingSource {
        use dp_hardware_intelligence::MappingSource;

        MappingSource::new()
            .with_file(
                "/etc/os-release",
                "ID=ubuntu\nVERSION_ID=\"24.04\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\n",
            )
            .with_file("/proc/sys/kernel/osrelease", "6.8.0-45-generic\n")
            .with_file(
                "/proc/cpuinfo",
                "processor\t: 0\nvendor_id\t: AuthenticAMD\nmodel name\t: AMD Ryzen 7 7840U\nphysical id\t: 0\ncpu cores\t: 8\nflags\t\t: fpu avx2\n",
            )
            .with_file("/proc/meminfo", "MemTotal: 32608376 kB\n")
            .with_present_path("/lib/x86_64-linux-gnu/libc.so.6")
            .with_dir("/sys/bus/pci/devices", ["0000:03:00.0"])
            .with_file("/sys/bus/pci/devices/0000:03:00.0/class", "0x030000\n")
            .with_file("/sys/bus/pci/devices/0000:03:00.0/vendor", "0x1002\n")
            .with_file("/sys/bus/pci/devices/0000:03:00.0/device", "0x15bf\n")
            .with_link(
                "/sys/bus/pci/devices/0000:03:00.0/driver",
                "../../../bus/pci/drivers/amdgpu",
            )
            .with_dir("/sys/class/drm", ["card0-eDP-1"])
            .with_file("/sys/class/drm/card0-eDP-1/status", "connected\n")
            .with_file("/sys/class/drm/card0-eDP-1/enabled", "enabled\n")
            .with_file("/sys/class/drm/card0-eDP-1/modes", "2880x1800\n")
    }

    #[test]
    fn hardware_scan_renders_the_documented_text_lines() {
        let dir = tempfile::tempdir().expect("temp dir");
        let env = env_with_host(dir.path(), Box::new(captured_host()));

        let response = execute(["hardware", "scan"], &env);

        assert_eq!(response.code, 0, "stderr: {}", response.stderr);
        assert!(response.stderr.is_empty());
        assert!(response.stdout.ends_with('\n'));
        assert!(response.stdout.contains("schema_version: 1.0\n"));
        assert!(response.stdout.contains("scan_status: complete\n"));
        assert!(
            response
                .stdout
                .contains("system.distribution: ubuntu 24.04 (Ubuntu 24.04.1 LTS)\n")
        );
        assert!(
            response
                .stdout
                .contains("system.kernel: 6.8.0-45-generic\n")
        );
        assert!(
            response
                .stdout
                .contains("cpu.cores: 1 logical, 8 physical, 1 socket(s)\n")
        );
        assert!(response.stdout.contains("memory.total: 31.1 GiB\n"));
        assert!(
            response
                .stdout
                .contains("gpu: 0000:03:00.0 0x1002:0x15bf class 0x030000 driver amdgpu\n")
        );
        assert!(
            response
                .stdout
                .contains("display: card0-eDP-1 connected enabled 2880x1800\n")
        );
        assert!(
            response
                .stdout
                .contains("diagnostic: info vulkan not_consulted:")
        );
    }

    #[test]
    fn hardware_scan_json_emits_the_versioned_report() {
        let dir = tempfile::tempdir().expect("temp dir");
        let env = env_with_host(dir.path(), Box::new(captured_host()));

        let response = execute(["hardware", "scan", "--json"], &env);

        assert_eq!(response.code, 0, "stderr: {}", response.stderr);
        let value: serde_json::Value =
            serde_json::from_str(&response.stdout).expect("the report is valid JSON");
        assert_eq!(value["schema_version"], "1.0");
        assert_eq!(value["scan_status"], "complete");
        assert_eq!(value["gpus"][0]["pci_address"], "0000:03:00.0");
        assert_eq!(value["displays"][0]["connector"], "card0-eDP-1");
    }

    #[test]
    fn hardware_scan_of_a_host_with_no_facts_fails_with_code_1() {
        let dir = tempfile::tempdir().expect("temp dir");
        let env = env_with_host(
            dir.path(),
            Box::new(dp_hardware_intelligence::MappingSource::new()),
        );

        let response = execute(["hardware", "scan"], &env);

        assert_eq!(response.code, 1);
        assert!(response.stdout.contains("scan_status: failed\n"));
        assert!(response.stderr.is_empty());
    }

    #[test]
    fn hardware_without_a_subcommand_is_a_usage_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let env = env_with_host(dir.path(), Box::new(captured_host()));

        let response = execute(["hardware"], &env);

        assert_eq!(response.code, 2);
        assert!(response.stdout.is_empty());
        assert!(
            response
                .stderr
                .contains("error: a `dp hardware` subcommand is required")
        );
        assert!(response.stderr.contains("usage: dp hardware <subcommand>"));
    }

    #[test]
    fn hardware_scan_rejects_extra_arguments_and_prefix_options() {
        let dir = tempfile::tempdir().expect("temp dir");
        let env = env_with_host(dir.path(), Box::new(captured_host()));

        let extra = execute(["hardware", "scan", "extra"], &env);
        assert_eq!(extra.code, 2);
        assert!(extra.stderr.contains("unexpected argument `extra`"));

        let foreign = execute(["hardware", "scan", "--active-instances", "1"], &env);
        assert_eq!(foreign.code, 2);
        assert!(
            foreign
                .stderr
                .contains("`--active-instances` does not apply to `dp hardware scan`")
        );
    }

    #[test]
    fn the_usage_block_lists_the_hardware_command() {
        assert!(USAGE.contains("dp hardware scan"));
        let dir = tempfile::tempdir().expect("temp dir");
        let env = env_with_host(dir.path(), Box::new(captured_host()));
        let response = execute(["--help"], &env);
        assert_eq!(response.code, 0);
        assert!(response.stdout.contains("dp hardware scan"));
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
            format!("dp {}\n", env!("CARGO_PKG_VERSION"))
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
        assert!(response.stdout.starts_with("usage: dp <command>"));
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
        assert!(response.stderr.contains("usage: dp <command>"));
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
                text.contains(&format!("dp prefix {subcommand}")),
                "`dp prefix {subcommand}` must be listed: {text}"
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
            missing.stderr.contains("usage: dp prefix"),
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

    /// `CPU_TYPE_X86_64`, as `<mach/machine.h>` defines it.
    const CPU_TYPE_X86_64: u32 = 0x0100_0007;

    /// An `Info.plist` that declares every key the bundle inspector reports.
    const EXAMPLE_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>com.example.example</string>
	<key>CFBundleName</key>
	<string>Example</string>
	<key>CFBundleDisplayName</key>
	<string>Example App</string>
	<key>CFBundleVersion</key>
	<string>1.2.3</string>
	<key>CFBundleExecutable</key>
	<string>Example</string>
	<key>CFBundleIconFile</key>
	<string>Example.icns</string>
	<key>LSMinimumSystemVersion</key>
	<string>10.13</string>
</dict>
</plist>
"#;

    /// A thin 64-bit little-endian Mach-O header for `cputype`, with no load
    /// commands: enough for the inspector to report the architecture.
    fn thin_macho(cputype: u32) -> Vec<u8> {
        let mut bytes = vec![0xcf, 0xfa, 0xed, 0xfe];
        bytes.extend_from_slice(&cputype.to_le_bytes());
        bytes.extend_from_slice(&3u32.to_le_bytes()); // cpusubtype: CPU_SUBTYPE_ANY
        bytes.extend_from_slice(&2u32.to_le_bytes()); // filetype: MH_EXECUTE
        bytes.extend_from_slice(&0u32.to_le_bytes()); // ncmds
        bytes.extend_from_slice(&0u32.to_le_bytes()); // sizeofcmds
        bytes.extend_from_slice(&0u32.to_le_bytes()); // flags
        bytes.extend_from_slice(&0u32.to_le_bytes()); // reserved
        bytes
    }

    /// Writes `bytes` to `path`, creating the directories it needs.
    fn write_fixture(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create the fixture directory");
        }
        std::fs::write(path, bytes).expect("write the fixture file");
    }

    /// A well-formed `Example.app` below `root`: the keys [`EXAMPLE_PLIST`]
    /// declares, a Mach-O executable and the icon it names.
    fn example_bundle(root: &Path) -> PathBuf {
        let bundle = root.join("Example.app");
        write_fixture(
            &bundle.join("Contents").join("Info.plist"),
            EXAMPLE_PLIST.as_bytes(),
        );
        write_fixture(
            &bundle.join("Contents").join("MacOS").join("Example"),
            &thin_macho(CPU_TYPE_X86_64),
        );
        write_fixture(
            &bundle
                .join("Contents")
                .join("Resources")
                .join("Example.icns"),
            b"icns",
        );
        bundle
    }

    /// `CPU_TYPE_ARM64`, `CPU_ARCH_ABI64 | CPU_TYPE_ARM`.
    const CPU_TYPE_ARM64: u32 = 0x0100_000c;

    /// A universal binary holding one thin slice per `cputypes` entry, behind the
    /// big-endian `fat_arch` table `lipo` writes.
    fn fat_macho(cputypes: &[u32]) -> Vec<u8> {
        const FAT_ARCH_LENGTH: usize = 20;

        let slices: Vec<Vec<u8>> = cputypes
            .iter()
            .map(|cputype| thin_macho(*cputype))
            .collect();
        let mut bytes = vec![0xca, 0xfe, 0xba, 0xbe];
        bytes.extend_from_slice(&(slices.len() as u32).to_be_bytes());
        let mut offset = (8 + FAT_ARCH_LENGTH * slices.len()) as u32;
        for (cputype, slice) in cputypes.iter().zip(&slices) {
            bytes.extend_from_slice(&cputype.to_be_bytes());
            bytes.extend_from_slice(&3u32.to_be_bytes()); // cpusubtype: CPU_SUBTYPE_ANY
            bytes.extend_from_slice(&offset.to_be_bytes());
            bytes.extend_from_slice(&(slice.len() as u32).to_be_bytes());
            bytes.extend_from_slice(&0u32.to_be_bytes()); // align: 2^0
            offset += slice.len() as u32;
        }
        for slice in &slices {
            bytes.extend_from_slice(slice);
        }
        bytes
    }

    /// An `Info.plist` that omits `LSMinimumSystemVersion`, so the analysis has
    /// one requirement nobody can decide.
    const UNDECIDED_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>com.example.undecided</string>
	<key>CFBundleName</key>
	<string>Undecided</string>
	<key>CFBundleExecutable</key>
	<string>Undecided</string>
</dict>
</plist>
"#;

    /// An `Undecided.app` below `root`: a bundle whose report is `unknown` because
    /// it declares no minimum macOS version, with an executable carrying a slice
    /// for either architecture CI may use.
    ///
    /// The universal binary keeps the test independent of the machine running
    /// it: an architecture the host cannot run would be `not_compatible`, and the
    /// report under test would then be a failure instead of an open question.
    fn undecided_bundle(root: &Path) -> PathBuf {
        let bundle = root.join("Undecided.app");
        write_fixture(
            &bundle.join("Contents").join("Info.plist"),
            UNDECIDED_PLIST.as_bytes(),
        );
        write_fixture(
            &bundle.join("Contents").join("MacOS").join("Undecided"),
            &fat_macho(&[CPU_TYPE_X86_64, CPU_TYPE_ARM64]),
        );
        bundle
    }

    /// A fixture path as the argument a test hands to `execute`.
    fn path_arg(path: &Path) -> &str {
        path.to_str().expect("a temporary path is valid UTF-8")
    }

    /// An isolated environment without `XDG_DATA_HOME`, so the platform data
    /// directory falls back to `$HOME/.local/share/darling-platform`.
    fn env_without_xdg(root: &Path) -> CliEnv {
        CliEnv {
            prefix: PrefixEnv {
                dprefix: Some(root.join("prefix").into_os_string()),
                xdg_data_home: None,
                home: Some(root.join("home").into_os_string()),
            },
            darling: Arc::new(FakeDarling),
            host: Box::new(dp_hardware_intelligence::LinuxHost),
        }
    }

    /// The catalogue the isolated environment resolves to.
    fn catalogue(env: &CliEnv) -> PathBuf {
        location(env).data_dir().join("library.sqlite3")
    }

    /// The catalogue rows the CLI wrote, read back through the library.
    fn applications(env: &CliEnv) -> Vec<ApplicationRecord> {
        Library::open(&catalogue(env))
            .expect("the catalogue the CLI wrote opens")
            .list_applications()
            .expect("read the applications")
    }

    #[test]
    fn app_inspect_renders_the_documented_text_lines() {
        let (dir, env) = isolated();
        let bundle = example_bundle(dir.path());

        let response = execute(["app", "inspect", path_arg(&bundle)], &env);

        assert_eq!(response.code, 0, "stderr: {}", response.stderr);
        assert_eq!(response.stderr, "");
        let expected = joined_lines(vec![
            format!("path: {}", bundle.display()),
            String::from("bundle_id: com.example.example"),
            String::from("name: Example"),
            String::from("display_name: Example App"),
            String::from("version: 1.2.3"),
            String::from("executable: Example"),
            format!(
                "executable_path: {}",
                bundle
                    .join("Contents")
                    .join("MacOS")
                    .join("Example")
                    .display()
            ),
            String::from("icon_file: Example.icns"),
            String::from("minimum_system_version: 10.13"),
            String::from("architectures: x86_64"),
            String::from("framework: none"),
            String::from("resource: Contents/Resources/Example.icns"),
        ]);
        assert_eq!(response.stdout, expected);
    }

    #[test]
    fn app_inspect_json_emits_the_bundle_metadata() {
        let (dir, env) = isolated();
        let bundle = example_bundle(dir.path());

        let response = execute(["app", "inspect", path_arg(&bundle), "--json"], &env);

        assert_eq!(response.code, 0, "stderr: {}", response.stderr);
        assert_eq!(response.stderr, "");
        assert!(response.stdout.ends_with('\n'));
        let document = json_document(&response.stdout);
        assert_eq!(document["path"].as_str(), Some(path_arg(&bundle)));
        assert_eq!(document["bundle_id"].as_str(), Some("com.example.example"));
        assert_eq!(document["name"].as_str(), Some("Example"));
        assert_eq!(document["display_name"].as_str(), Some("Example App"));
        assert_eq!(document["version"].as_str(), Some("1.2.3"));
        assert_eq!(document["executable"].as_str(), Some("Example"));
        assert_eq!(document["icon_file"].as_str(), Some("Example.icns"));
        assert_eq!(document["minimum_system_version"].as_str(), Some("10.13"));
        assert_eq!(document["architectures"][0].as_str(), Some("x86_64"));
        assert_eq!(
            document["resources"][0].as_str(),
            Some("Contents/Resources/Example.icns")
        );
        assert!(
            document["frameworks"].is_array(),
            "frameworks is always a list: {}",
            response.stdout
        );
    }

    #[test]
    fn app_inspect_creates_neither_the_prefix_nor_the_catalogue() {
        let (dir, env) = isolated();
        let bundle = example_bundle(dir.path());
        let location = location(&env);

        let response = execute(["app", "inspect", path_arg(&bundle)], &env);

        assert_eq!(response.code, 0, "stderr: {}", response.stderr);
        assert!(
            !location.path().exists(),
            "inspect must not create the prefix"
        );
        assert!(
            !location.data_dir().exists(),
            "inspect must not create the platform data directory"
        );
        assert!(
            !location.lock_path().exists(),
            "inspect must not create the lock file"
        );
        assert!(
            !dir.path().join("data").exists(),
            "inspect must leave the data home absent"
        );
    }

    #[test]
    fn app_inspect_of_a_malformed_bundle_fails_with_code_1() {
        let (dir, env) = isolated();
        let bundle = dir.path().join("Broken.app");
        write_fixture(&bundle.join("Contents").join("Info.plist"), b"not a plist");

        let response = execute(["app", "inspect", path_arg(&bundle)], &env);

        assert_eq!(response.code, 1);
        assert_eq!(response.stdout, "");
        assert!(
            response.stderr.starts_with("error: "),
            "unexpected stderr: {}",
            response.stderr
        );
        assert!(
            response.stderr.contains("Info.plist"),
            "the failure must name the file: {}",
            response.stderr
        );
    }

    #[test]
    fn app_compat_renders_an_undecided_report_and_exits_1() {
        let (dir, env) = isolated();
        let bundle = undecided_bundle(dir.path());
        let profile = dp_compatibility_engine::Profile::embedded()
            .expect("the embedded capability profile must be valid");

        let response = execute(["app", "compat", path_arg(&bundle)], &env);

        assert_eq!(
            response.code, 1,
            "an unknown verdict is not established compatibility"
        );
        assert_eq!(response.stderr, "");
        assert!(response.stdout.ends_with('\n'));
        let header: Vec<String> = response
            .stdout
            .lines()
            .take(3)
            .map(str::to_string)
            .collect();
        assert_eq!(
            header,
            vec![
                String::from("verdict: unknown"),
                format!(
                    "analyser_version: {}",
                    dp_compatibility_engine::ANALYSER_VERSION
                ),
                format!("profile_version: {}", profile.profile_version),
            ],
            "unexpected output: {}",
            response.stdout
        );
        let requirements: Vec<&str> = response.stdout.lines().skip(3).collect();
        assert_eq!(
            requirements.len(),
            2,
            "one line per requirement: {}",
            response.stdout
        );
        assert!(
            requirements[0].starts_with("requirement: satisfied architecture "),
            "unexpected output: {}",
            response.stdout
        );
        assert!(
            requirements[1].starts_with("requirement: unknown minimum_os none: "),
            "unexpected output: {}",
            response.stdout
        );
        assert!(
            requirements[1].contains("declare a minimum macOS version"),
            "the undecided requirement must say why: {}",
            response.stdout
        );
    }

    #[test]
    fn app_compat_json_emits_the_versioned_report() {
        let (dir, env) = isolated();
        let bundle = undecided_bundle(dir.path());
        let profile = dp_compatibility_engine::Profile::embedded()
            .expect("the embedded capability profile must be valid");

        let response = execute(["app", "compat", path_arg(&bundle), "--json"], &env);

        assert_eq!(response.code, 1);
        assert_eq!(response.stderr, "");
        assert!(response.stdout.ends_with('\n'));
        let document = json_document(&response.stdout);
        assert_eq!(document["verdict"].as_str(), Some("unknown"));
        assert_eq!(
            document["analyser_version"].as_str(),
            Some(dp_compatibility_engine::ANALYSER_VERSION)
        );
        assert_eq!(
            document["schema_version"].as_str(),
            Some(dp_compatibility_engine::SCHEMA_VERSION)
        );
        assert_eq!(
            document["profile_version"].as_str(),
            Some(profile.profile_version.as_str())
        );
        let requirements = document["requirements"]
            .as_array()
            .unwrap_or_else(|| panic!("requirements is a list: {}", response.stdout));
        assert!(
            requirements
                .iter()
                .any(|requirement| requirement["kind"].as_str() == Some("minimum_os")),
            "the report must carry the evidence it rests on: {}",
            response.stdout
        );
    }

    #[test]
    fn app_compat_creates_neither_the_prefix_nor_the_data_directory() {
        let (dir, env) = isolated();
        let bundle = undecided_bundle(dir.path());
        let location = location(&env);

        let response = execute(["app", "compat", path_arg(&bundle)], &env);

        assert_eq!(response.code, 1, "the fixture's verdict is unknown");
        assert_eq!(response.stderr, "");
        assert!(
            !location.path().exists(),
            "compat must not create the prefix"
        );
        assert!(
            !location.data_dir().exists(),
            "compat must not create the platform data directory"
        );
        assert!(
            !location.lock_path().exists(),
            "compat must not create the lock file"
        );
        assert!(
            !dir.path().join("data").exists(),
            "compat must leave the data home absent"
        );
    }

    #[test]
    fn app_compat_of_a_malformed_bundle_fails_with_code_1() {
        let (dir, env) = isolated();
        let bundle = dir.path().join("Broken.app");
        write_fixture(&bundle.join("Contents").join("Info.plist"), b"not a plist");

        let response = execute(["app", "compat", path_arg(&bundle)], &env);

        assert_eq!(response.code, 1);
        assert_eq!(response.stdout, "");
        assert!(
            response.stderr.starts_with("error: "),
            "unexpected stderr: {}",
            response.stderr
        );
        assert!(
            response.stderr.contains("Info.plist"),
            "the failure must name the file: {}",
            response.stderr
        );
    }

    #[test]
    fn app_compat_without_a_path_is_a_usage_error() {
        let (_dir, env) = isolated();

        let response = execute(["app", "compat"], &env);

        assert_eq!(response.code, 2);
        assert_eq!(response.stdout, "");
        assert!(
            response
                .stderr
                .contains("`dp app compat` requires the path"),
            "unexpected stderr: {}",
            response.stderr
        );
        assert!(
            response.stderr.contains("usage: dp app <subcommand>"),
            "unexpected stderr: {}",
            response.stderr
        );
        assert!(
            response
                .stderr
                .contains("0  the verdict is compatible or partial"),
            "the usage block must document the exit codes: {}",
            response.stderr
        );
        assert!(
            response
                .stderr
                .contains("1  the verdict is not_compatible or unknown"),
            "the usage block must document the exit codes: {}",
            response.stderr
        );
    }

    #[test]
    fn a_report_without_requirements_renders_one_none_line() {
        let report = CompatibilityReport {
            schema_version: String::from(dp_compatibility_engine::SCHEMA_VERSION),
            verdict: Verdict::Unknown,
            analyser_version: String::from(dp_compatibility_engine::ANALYSER_VERSION),
            profile_version: String::from("test"),
            requirements: Vec::new(),
        };

        assert_eq!(
            render_compat(&report),
            "verdict: unknown\nanalyser_version: 1\nprofile_version: test\nrequirement: none\n"
        );
    }

    #[test]
    fn app_without_a_subcommand_and_with_an_unknown_one_is_a_usage_error() {
        let (dir, env) = isolated();
        let bundle = example_bundle(dir.path());

        let missing = execute(["app"], &env);
        assert_eq!(missing.code, 2);
        assert_eq!(missing.stdout, "");
        assert!(
            missing.stderr.contains("a `dp app` subcommand is required"),
            "unexpected stderr: {}",
            missing.stderr
        );
        assert!(missing.stderr.contains("usage: dp app <subcommand>"));

        let unknown = execute(["app", "frobnicate"], &env);
        assert_eq!(unknown.code, 2);
        assert_eq!(unknown.stdout, "");
        assert!(
            unknown
                .stderr
                .contains("unknown `dp app` subcommand `frobnicate`"),
            "unexpected stderr: {}",
            unknown.stderr
        );
        assert!(unknown.stderr.contains("usage: dp app <subcommand>"));

        let missing_path = execute(["app", "install"], &env);
        assert_eq!(missing_path.code, 2);
        assert_eq!(missing_path.stdout, "");
        assert!(
            missing_path.stderr.contains("requires the path"),
            "unexpected stderr: {}",
            missing_path.stderr
        );

        let extra = execute(["app", "list", path_arg(&bundle)], &env);
        assert_eq!(extra.code, 2);
        assert_eq!(extra.stdout, "");
        assert!(
            extra.stderr.contains("unexpected argument"),
            "unexpected stderr: {}",
            extra.stderr
        );
    }

    #[test]
    fn app_rejects_the_options_that_do_not_apply_to_the_subcommand() {
        let (dir, env) = isolated();
        let bundle = example_bundle(dir.path());
        let path = path_arg(&bundle);

        let cases: [(&[&str], &str); 8] = [
            (
                &["app", "inspect", path, "--replace"],
                "`--replace` does not apply to `dp app inspect`",
            ),
            (
                &["app", "compat", path, "--replace"],
                "`--replace` does not apply to `dp app compat`",
            ),
            (
                &["app", "list", "--replace"],
                "`--replace` does not apply to `dp app list`",
            ),
            (
                &["app", "list", "--active-instances", "1"],
                "`--active-instances` does not apply to `dp app list`",
            ),
            (
                &["app", "install", path, "--active-instances", "2"],
                "`--active-instances` does not apply to `dp app install`",
            ),
            (
                &["app", "inspect", path, "--active-instances", "0"],
                "`--active-instances` does not apply to `dp app inspect`",
            ),
            (
                &["prefix", "status", "--replace"],
                "`--replace` does not apply to `dp prefix status`",
            ),
            (
                &["hardware", "scan", "--replace"],
                "`--replace` does not apply to `dp hardware scan`",
            ),
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
        assert!(
            !location(&env).data_dir().exists(),
            "a rejected option must not create the platform data directory"
        );
    }

    #[test]
    fn the_usage_block_lists_the_app_commands() {
        for command in [
            "dp app inspect",
            "dp app install",
            "dp app list",
            "dp app compat",
        ] {
            assert!(USAGE.contains(command), "USAGE must list `{command}`");
        }

        let (_dir, env) = isolated();
        let response = execute(["--help"], &env);

        assert_eq!(response.code, 0);
        for command in [
            "dp app inspect",
            "dp app install",
            "dp app list",
            "dp app compat",
        ] {
            assert!(
                response.stdout.contains(command),
                "`{command}` must be listed: {}",
                response.stdout
            );
        }
        assert!(response.stdout.contains("--replace"));

        let block = execute(["app"], &env);
        assert_eq!(block.code, 2);
        assert!(
            block.stderr.contains("  compat <path>"),
            "the `dp app` usage block must list `compat`: {}",
            block.stderr
        );
    }

    #[test]
    fn app_list_of_an_empty_catalogue_prints_an_empty_list() {
        let (_dir, env) = isolated();

        let response = execute(["app", "list"], &env);

        assert_eq!(response.code, 0, "stderr: {}", response.stderr);
        assert_eq!(response.stderr, "");
        assert_eq!(response.stdout, "applications: none\n");
        assert!(
            catalogue(&env).is_file(),
            "listing opens the catalogue the installer writes"
        );

        let json = execute(["app", "list", "--json"], &env);
        assert_eq!(json.code, 0, "stderr: {}", json.stderr);
        assert_eq!(json.stderr, "");
        assert_eq!(json.stdout, "{\"applications\":[]}\n");
    }

    #[test]
    fn app_install_installs_the_bundle_and_records_it() {
        let (dir, env) = isolated();
        let bundle = example_bundle(dir.path());
        let destination = location(&env)
            .path()
            .join("Applications")
            .join("Example.app");

        let response = execute(["app", "install", path_arg(&bundle)], &env);

        assert_eq!(response.code, 0, "stderr: {}", response.stderr);
        assert_eq!(response.stderr, "");
        assert!(
            response
                .stdout
                .starts_with(&format!("installed: {}\n", destination.display())),
            "the destination must be printed: {}",
            response.stdout
        );
        assert!(response.stdout.ends_with('\n'));
        assert!(
            destination.join("Contents").join("Info.plist").is_file(),
            "the bundle is copied whole into the prefix"
        );
        assert!(
            destination
                .join("Contents")
                .join("MacOS")
                .join("Example")
                .is_file()
        );

        let rows = applications(&env);
        assert_eq!(rows.len(), 1, "unexpected rows: {rows:?}");
        let row = &rows[0];
        assert_eq!(row.id, "com.example.example");
        assert_eq!(row.bundle_id, "com.example.example");
        assert_eq!(row.name, "Example App");
        assert_eq!(row.version.as_deref(), Some("1.2.3"));
        assert_eq!(row.bundle_path, destination);
        assert_eq!(row.architecture.as_deref(), Some("x86_64"));
        assert_eq!(row.installation_type, "bundle");
        assert_eq!(row.installation_status, "installed");

        let library = Library::open(&catalogue(&env)).expect("the CLI wrote the catalogue");
        let attempts = library
            .installations()
            .expect("read the installation history");
        assert_eq!(attempts.len(), 1, "unexpected history: {attempts:?}");
        assert_eq!(attempts[0].origin, path_arg(&bundle));
        assert_eq!(attempts[0].format, "app");
        assert_eq!(attempts[0].result, "installed");

        let listed = execute(["app", "list"], &env);
        assert_eq!(listed.code, 0, "stderr: {}", listed.stderr);
        assert!(
            listed.stdout.contains(&format!(
                "application: name=Example App bundle_id=com.example.example version=1.2.3 \
                 architecture=x86_64 status=installed path={}\n",
                destination.display()
            )),
            "unexpected output: {}",
            listed.stdout
        );
    }

    #[test]
    fn app_install_refuses_an_existing_destination_unless_replace_is_set() {
        let (dir, env) = isolated();
        let bundle = example_bundle(dir.path());
        let destination = location(&env)
            .path()
            .join("Applications")
            .join("Example.app");
        let first = execute(["app", "install", path_arg(&bundle)], &env);
        assert_eq!(first.code, 0, "stderr: {}", first.stderr);

        let refused = execute(["app", "install", path_arg(&bundle)], &env);

        assert_eq!(refused.code, 1);
        assert_eq!(refused.stdout, "");
        assert!(
            refused.stderr.starts_with("error: "),
            "unexpected stderr: {}",
            refused.stderr
        );
        assert!(
            refused.stderr.contains("replace"),
            "the failure must name the policy: {}",
            refused.stderr
        );
        assert_eq!(
            applications(&env).len(),
            1,
            "a refused import records no application"
        );

        let replaced = execute(
            ["app", "install", path_arg(&bundle), "--replace", "--json"],
            &env,
        );

        assert_eq!(replaced.code, 0, "stderr: {}", replaced.stderr);
        assert_eq!(replaced.stderr, "");
        let document = json_document(&replaced.stdout);
        assert_eq!(document["format"].as_str(), Some("app"));
        assert_eq!(document["path"].as_str(), Some(path_arg(&destination)));
        assert_eq!(
            document["metadata"]["bundle_id"].as_str(),
            Some("com.example.example")
        );
        assert!(
            document["installation_id"]
                .as_str()
                .is_some_and(|id| !id.is_empty()),
            "an attempt reports its identifier: {}",
            replaced.stdout
        );
        assert_eq!(
            applications(&env).len(),
            1,
            "a replacement keeps one application row"
        );

        let library = Library::open(&catalogue(&env)).expect("the CLI wrote the catalogue");
        let attempts = library
            .installations()
            .expect("read the installation history");
        assert_eq!(
            attempts.len(),
            3,
            "every attempt stays in the history: {attempts:?}"
        );
        assert_eq!(
            attempts.iter().filter(|a| a.result == "failed").count(),
            1,
            "the refused attempt is recorded as failed: {attempts:?}"
        );
    }

    #[test]
    fn a_source_the_installer_refuses_creates_no_applications_directory() {
        let (dir, env) = isolated();
        let source = dir.path().join("Example.dmg");
        std::fs::write(&source, b"not a disk image").expect("write the source");
        let applications_dir = location(&env).path().join("Applications");

        let response = execute(["app", "install", path_arg(&source)], &env);

        assert_eq!(response.code, 1);
        assert_eq!(response.stdout, "");
        assert!(
            response.stderr.starts_with("error: "),
            "unexpected stderr: {}",
            response.stderr
        );
        assert!(
            response.stderr.contains("Example.dmg"),
            "the failure must name the source: {}",
            response.stderr
        );
        assert!(
            response.stderr.contains(".app") && response.stderr.contains(".zip"),
            "the failure must list the supported formats: {}",
            response.stderr
        );
        assert!(
            !applications_dir.exists(),
            "the applications directory belongs to the installer, which refused to run"
        );
        assert!(
            applications(&env).is_empty(),
            "a refused source records no application"
        );
    }

    #[test]
    fn the_catalogue_falls_back_to_the_home_data_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let env = env_without_xdg(dir.path());
        let bundle = example_bundle(dir.path());

        let response = execute(["app", "install", path_arg(&bundle)], &env);

        assert_eq!(response.code, 0, "stderr: {}", response.stderr);
        let data_dir = dir
            .path()
            .join("home")
            .join(".local")
            .join("share")
            .join("darling-platform");
        assert!(
            data_dir.join("library.sqlite3").is_file(),
            "without XDG_DATA_HOME the catalogue lives in the home data directory"
        );
        assert!(
            !dir.path().join("data").exists(),
            "the XDG data home must stay untouched"
        );
    }
}
