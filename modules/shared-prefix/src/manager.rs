//! Lifecycle management of the shared DPREFIX.
//!
//! The manager is the only component that changes the platform's lifecycle phase.
//! It drives exactly the Darling operations that are verified to exist
//! (`ADR-0008`): `start` is `darling shell /usr/bin/true`, which creates the prefix
//! on first use and connects to the container if it already runs (F2 + F3), and
//! `stop` is `darling shutdown`, which terminates every process inside (F4).
//!
//! Three invariants shape this module:
//!
//! - The platform never fabricates Darling's internal layout (S5). No prefix
//!   directory is ever created here; Darling creates it.
//! - Darling cannot be asked whether its container is alive (F5), so the phase is
//!   persisted by the platform, and a phase that stayed mid-transition is reported
//!   as a failure with the interrupted operation named (ADR-0008, decision 4).
//! - `stop` is guarded by the managed instance count, because `darling shutdown`
//!   kills everything inside the container (F4 + S5).
//!
//! Reads (`inspect`) never wait on another process's lock. Consistency-sensitive
//! reads (`validate`) take the shared lock, and state-changing operations (`start`,
//! `stop`) take the exclusive lock and report [`Error::PrefixBusy`] instead of
//! waiting for it.

use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::darling::{CommandOutcome, Darling};
use crate::error::{Error, Result};
use crate::location::{MAX_PREFIX_PATH_BYTES, PrefixLocation};
use crate::lock::PrefixLock;
use crate::state::{Phase, StateRecord, StateStore};

/// Path length above which `validate` warns, in bytes.
const PATH_LENGTH_WARN_BYTES: usize = 200;

/// Operation name persisted while `start` runs.
const OPERATION_START: &str = "start";

/// Operation name persisted while `stop` runs.
const OPERATION_STOP: &str = "stop";

/// The only verified way to make Darling create the prefix and have its container
/// up (F2 + F3).
const START_ARGS: [&str; 2] = ["shell", "/usr/bin/true"];

/// The verified destructive stop (F4).
const STOP_ARGS: [&str; 1] = ["shutdown"];

/// Result of one validation check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// The check holds.
    Pass,
    /// The check holds, but something the platform cannot fix deserves attention.
    Warn,
    /// The prefix violates a requirement and the platform refuses to proceed.
    Fail,
    /// The check does not apply to the current prefix.
    NotApplicable,
}

/// One named validation check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// Stable check name, part of the CLI's output contract.
    pub name: &'static str,
    /// Outcome of the check.
    pub status: CheckStatus,
    /// Human-readable explanation, actionable when the status is not `Pass`.
    pub detail: String,
}

/// The full validation report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Validation {
    /// Every check, in a stable order.
    pub checks: Vec<Check>,
}

impl Validation {
    /// Returns `true` when no check failed.
    ///
    /// Warnings and inapplicable checks do not make a prefix invalid: an absent
    /// prefix is not an error, because `dg prefix start` creates it.
    pub fn passed(&self) -> bool {
        !self
            .checks
            .iter()
            .any(|check| check.status == CheckStatus::Fail)
    }
}

/// A non-blocking snapshot of the prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Inspection {
    /// The dedicated prefix the platform manages.
    pub prefix: PathBuf,
    /// Whether the prefix directory currently exists.
    pub exists: bool,
    /// The phase the platform reports.
    pub phase: Phase,
    /// Operation name when the persisted phase was left mid-transition.
    pub interrupted: Option<String>,
    /// Whether another process currently holds the prefix lock.
    pub busy: bool,
    /// Program that will be invoked for lifecycle operations.
    pub darling_program: PathBuf,
    /// Whether that program could be found on disk.
    pub darling_present: bool,
    /// The Darling release this build is pinned to.
    pub darling_pin: crate::pin::DarlingPin,
    /// Problems the operator should know about, verbatim and non-fatal.
    pub warnings: Vec<String>,
}

/// Drives the lifecycle of the dedicated prefix.
pub struct PrefixManager {
    location: PrefixLocation,
    darling: Box<dyn Darling>,
}

impl PrefixManager {
    /// Builds a manager over `location` that runs lifecycle operations through
    /// `darling`.
    pub fn new(location: PrefixLocation, darling: Box<dyn Darling>) -> Self {
        Self { location, darling }
    }

    /// Reports the prefix without ever waiting on another process's lock.
    ///
    /// The snapshot is honest about what the platform does not know: a phase that
    /// stayed mid-transition is reported as [`Phase::Error`] with the interrupted
    /// operation named (ADR-0008, decision 4), and every suspicion becomes a
    /// warning instead of a failure.
    pub fn inspect(&self) -> Result<Inspection> {
        let persisted = self.state_store().load()?;
        let prefix = self.location.path().to_path_buf();
        let exists = prefix.exists();
        let mut warnings = Vec::new();

        let (phase, interrupted) = match &persisted {
            Some(record) if record.is_transitional() => {
                let operation = record
                    .operation
                    .clone()
                    .unwrap_or_else(|| String::from(phase_name(record.phase)));
                warnings.push(format!(
                    "the previous '{operation}' operation never finished (InterruptedOperation \
                     risk): the persisted phase is still '{}'; run the operation again, or run \
                     \"dg prefix stop\" to reach a known phase",
                    phase_name(record.phase)
                ));
                (Phase::Error, Some(operation))
            }
            Some(record) => (record.phase, None),
            None if exists => {
                warnings.push(format!(
                    "the prefix directory '{}' exists but the platform has no state record for \
                     it; the platform phase is unknown until the next start or stop completes",
                    prefix.display()
                ));
                (Phase::Stopped, None)
            }
            None => (Phase::NotInitialized, None),
        };

        if persisted
            .as_ref()
            .is_some_and(|record| record.phase == Phase::Running)
            && !exists
        {
            warnings.push(format!(
                "the persisted phase is 'running' but the prefix directory '{}' does not exist; \
                 Darling cannot be asked whether its container is alive, so run \"dg prefix \
                 start\" to re-establish it",
                prefix.display()
            ));
        }

        let darling_present = program_on_disk(self.darling.program());

        if !darling_present {
            warnings.push(format!(
                "the Darling program '{}' was not found on disk; lifecycle operations will fail \
                 with DarlingUnavailable until Darling is installed or DG_DARLING points at a \
                 usable binary",
                self.darling.program().display()
            ));
        }

        // The probe neither waits nor creates the lock file, so a read-only
        // command cannot leave a trace on disk.
        let busy = PrefixLock::probe_busy(&self.location.lock_path())?;

        Ok(Inspection {
            prefix,
            exists,
            phase,
            interrupted,
            busy,
            darling_program: self.darling.program().to_path_buf(),
            darling_present,
            darling_pin: crate::pin::pinned(),
            warnings,
        })
    }

    /// Creates the directory that will hold the prefix, never the prefix itself.
    ///
    /// Darling creates the prefix with a plain `mkdir`, so a `DPREFIX` whose parent
    /// does not exist fails with "No such file or directory" before it can do
    /// anything. The parent is the platform's path, not one of Darling's internal
    /// directories (ADR-0008, decision 5), so creating it here is ours to do.
    fn ensure_prefix_parent(&self) -> Result<()> {
        let Some(parent) = self
            .location
            .path()
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
        else {
            return Ok(());
        };

        if parent.exists() {
            return Ok(());
        }

        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_path_buf(),
            source,
        })
    }

    /// Validates the prefix under the shared lock and reports every check.
    ///
    /// The shared lock keeps the report consistent with concurrent readers and
    /// excludes the lifecycle operations, which take the exclusive lock. No check
    /// changes the prefix: the platform states only what it can prove without
    /// inventing Darling's layout (ADR-0008, decision 5).
    pub fn validate(&self) -> Result<Validation> {
        let _lock = PrefixLock::acquire_shared(&self.location.lock_path())?;
        let current_uid = current_uid();

        let checks = vec![
            prefix_exists_check(self.location.path()),
            prefix_parent_check(self.location.path()),
            is_directory_check(self.location.path()),
            not_symlink_check(self.location.path()),
            owned_by_current_user_check(self.location.path(), current_uid),
            writable_check(self.location.path()),
            path_length_check(self.location.path()),
            dedicated_location_check(self.location.path()),
            data_dir_outside_prefix_check(&self.location),
            state_record_check(&self.location),
        ];

        Ok(Validation { checks })
    }

    /// Ensures the container is up and the prefix exists, through Darling.
    ///
    /// `darling shell /usr/bin/true` is the only verified way to do both (F2 + F3):
    /// an absent prefix is created by Darling, and a running container is joined
    /// instead of started twice. The platform records the transitional phase before
    /// invoking Darling, so a crash leaves an interrupted operation behind instead
    /// of a silence.
    pub fn start(&self) -> Result<Phase> {
        let _lock = self.lock_exclusive()?;
        self.ensure_prefix_parent()?;
        let store = self.state_store();
        let transitional = if self.location.path().exists() {
            Phase::Starting
        } else {
            Phase::Initializing
        };
        store.save(&record(transitional, Some(OPERATION_START)))?;

        match self.darling.run(self.location.path(), &START_ARGS) {
            Ok(outcome) => finish(
                &store,
                self.darling.program(),
                outcome,
                Phase::Running,
                OPERATION_START,
            ),
            Err(error) => {
                store.save(&record(Phase::Error, Some(OPERATION_START)))?;
                Err(error)
            }
        }
    }

    /// Stops the container through Darling, refusing while instances are alive.
    ///
    /// `darling shutdown` terminates every process inside the container (F4), so an
    /// active managed instance aborts the operation before any state is written and
    /// before Darling is invoked.
    pub fn stop(&self, active_instances: u32) -> Result<Phase> {
        if active_instances > 0 {
            return Err(Error::ActiveProcesses {
                count: active_instances,
            });
        }

        let _lock = self.lock_exclusive()?;
        let store = self.state_store();
        store.save(&record(Phase::Stopping, Some(OPERATION_STOP)))?;

        match self.darling.run(self.location.path(), &STOP_ARGS) {
            Ok(outcome) => finish(
                &store,
                self.darling.program(),
                outcome,
                Phase::Stopped,
                OPERATION_STOP,
            ),
            Err(error) => {
                store.save(&record(Phase::Error, Some(OPERATION_STOP)))?;
                Err(error)
            }
        }
    }

    /// Store backed by this prefix's state file.
    fn state_store(&self) -> StateStore {
        StateStore::new(self.location.state_path())
    }

    /// Takes the exclusive lifecycle lock, or reports the holder instead of waiting.
    fn lock_exclusive(&self) -> Result<PrefixLock> {
        let path = self.location.lock_path();
        match PrefixLock::try_exclusive(&path)? {
            Some(lock) => Ok(lock),
            None => Err(Error::PrefixBusy { path }),
        }
    }
}

/// Builds a record stamped with the current time.
///
/// The timestamp is read here, at the caller of [`StateRecord`], which is why the
/// state layer never touches the clock itself.
fn record(phase: Phase, operation: Option<&str>) -> StateRecord {
    let updated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    StateRecord::new(phase, operation.map(str::to_owned), updated_at)
}

/// Interprets a lifecycle command's outcome and persists the resulting phase.
///
/// The adapter reports the exit status and never interprets it; deciding that a
/// non-zero status means a failed operation belongs here, together with the phase
/// that records it.
fn finish(
    store: &StateStore,
    program: &Path,
    outcome: CommandOutcome,
    success: Phase,
    operation: &str,
) -> Result<Phase> {
    if outcome.exit_code == Some(0) {
        store.save(&record(success, None))?;
        return Ok(success);
    }

    store.save(&record(Phase::Error, Some(operation)))?;
    Err(Error::DarlingFailed {
        program: program.to_path_buf(),
        exit_code: outcome.exit_code,
        stderr: outcome.stderr,
    })
}

/// Returns `true` when `program` can be found.
///
/// A program with a directory component is checked directly; a bare name is looked
/// up in `PATH`, which is the only way to answer the question for a program that
/// Darling's own launcher would resolve. A missing program is a warning, never a
/// failure: the phase is still worth reporting without Darling installed.
fn program_on_disk(program: &Path) -> bool {
    if program.is_absolute() || program.components().count() > 1 {
        return program.is_file();
    }

    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| directory.join(program).is_file())
    })
}

/// Uid the platform runs as, taken from the platform's own lock file.
///
/// `std` exposes no "current user" call and this crate must not grow a dependency
/// for one, so the answer comes from a file the platform creates for itself: the
/// lock file that `validate` holds a shared lock on while it compares owners.
/// uid the platform runs as.
fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

/// Whether the directory that will hold the prefix is usable.
///
/// Darling creates the prefix with a plain `mkdir`, so a missing parent makes the
/// first `start` fail before Darling can do anything. `start` creates it, and this
/// check says so before the operator hits the error.
fn prefix_parent_check(prefix: &Path) -> Check {
    const NAME: &str = "prefix_parent";

    let Some(parent) = prefix.parent().filter(|dir| !dir.as_os_str().is_empty()) else {
        return Check {
            name: NAME,
            status: CheckStatus::Pass,
            detail: String::from("the prefix sits at the filesystem root"),
        };
    };

    if parent.is_dir() {
        return match rustix::fs::access(parent, rustix::fs::Access::WRITE_OK) {
            Ok(()) => Check {
                name: NAME,
                status: CheckStatus::Pass,
                detail: format!("'{}' exists and is writable", parent.display()),
            },
            Err(_) => Check {
                name: NAME,
                status: CheckStatus::Fail,
                detail: format!(
                    "'{}' exists but the current user cannot write to it, so `dg prefix start` \
                     could not create the prefix",
                    parent.display()
                ),
            },
        };
    }

    let (status, detail) = if parent.exists() {
        (
            CheckStatus::Fail,
            format!(
                "'{}' is not a directory, so the prefix cannot be created below it",
                parent.display()
            ),
        )
    } else {
        (
            CheckStatus::Warn,
            format!(
                "'{}' does not exist yet; `dg prefix start` creates it (Darling only creates the \
                 prefix itself)",
                parent.display()
            ),
        )
    };

    Check {
        name: NAME,
        status,
        detail,
    }
}

fn prefix_exists_check(prefix: &Path) -> Check {
    let (status, detail) = if prefix.exists() {
        (
            CheckStatus::Pass,
            format!("the prefix directory '{}' exists", prefix.display()),
        )
    } else {
        (
            CheckStatus::Warn,
            format!(
                "'{}' is not initialized yet; `dg prefix start` creates it through Darling",
                prefix.display()
            ),
        )
    };

    Check {
        name: "prefix_exists",
        status,
        detail,
    }
}

fn is_directory_check(prefix: &Path) -> Check {
    let (status, detail) = if !prefix.exists() {
        (
            CheckStatus::NotApplicable,
            String::from("the prefix does not exist yet; there is nothing to inspect"),
        )
    } else if prefix.is_dir() {
        (
            CheckStatus::Pass,
            format!("'{}' is a directory", prefix.display()),
        )
    } else {
        (
            CheckStatus::Fail,
            format!(
                "'{}' exists but is not a directory; point DPREFIX at a directory, because \
                 Darling's overlay needs one",
                prefix.display()
            ),
        )
    };

    Check {
        name: "is_directory",
        status,
        detail,
    }
}

fn not_symlink_check(prefix: &Path) -> Check {
    let (status, detail) = match std::fs::symlink_metadata(prefix) {
        Ok(metadata) if metadata.file_type().is_symlink() => (
            CheckStatus::Fail,
            format!(
                "'{}' is a symlink; the shared prefix must be a real directory so Darling's \
                 overlay cannot be redirected elsewhere",
                prefix.display()
            ),
        ),
        Ok(_) => (
            CheckStatus::Pass,
            format!("'{}' is not a symlink", prefix.display()),
        ),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => (
            CheckStatus::NotApplicable,
            String::from("the prefix does not exist yet; there is nothing to inspect"),
        ),
        Err(source) => (
            CheckStatus::Fail,
            format!("cannot inspect '{}': {source}", prefix.display()),
        ),
    };

    Check {
        name: "not_symlink",
        status,
        detail,
    }
}

fn owned_by_current_user_check(prefix: &Path, current_uid: u32) -> Check {
    const NAME: &str = "owned_by_current_user";

    if !prefix.exists() {
        return Check {
            name: NAME,
            status: CheckStatus::NotApplicable,
            detail: String::from(
                "the prefix does not exist yet; Darling creates it with the running user as \
                 owner",
            ),
        };
    }

    let metadata = match std::fs::metadata(prefix) {
        Ok(metadata) => metadata,
        Err(source) => {
            return Check {
                name: NAME,
                status: CheckStatus::Fail,
                detail: format!("cannot inspect '{}': {source}", prefix.display()),
            };
        }
    };
    let owner = metadata.uid();

    let (status, detail) = if current_uid == owner {
        (
            CheckStatus::Pass,
            format!(
                "'{}' is owned by the current user (uid {current_uid})",
                prefix.display()
            ),
        )
    } else {
        (
            CheckStatus::Warn,
            format!(
                "'{}' is owned by uid {owner}, but the platform runs as uid {current_uid}; \
                 Darling can only write its overlay where the running user is allowed to write",
                prefix.display()
            ),
        )
    };

    Check {
        name: NAME,
        status,
        detail,
    }
}

fn writable_check(prefix: &Path) -> Check {
    const NAME: &str = "writable";

    if !prefix.exists() {
        return Check {
            name: NAME,
            status: CheckStatus::NotApplicable,
            detail: String::from(
                "the prefix does not exist yet; Darling creates it when it can write its parent \
                 directory",
            ),
        };
    }

    let (status, detail) = match rustix::fs::access(prefix, rustix::fs::Access::WRITE_OK) {
        Ok(()) => (
            CheckStatus::Pass,
            format!("'{}' is writable by the current user", prefix.display()),
        ),
        Err(errno) => (
            CheckStatus::Fail,
            format!(
                "'{}' is not writable: {}; Darling cannot create its overlay in a read-only \
                 directory",
                prefix.display(),
                std::io::Error::from(errno)
            ),
        ),
    };

    Check {
        name: NAME,
        status,
        detail,
    }
}

/// Reports the length of the prefix path in bytes.
fn path_length_check(prefix: &Path) -> Check {
    let bytes = prefix.as_os_str().as_bytes().len();

    let (status, detail) = if bytes > MAX_PREFIX_PATH_BYTES {
        (
            CheckStatus::Fail,
            format!(
                "the prefix path is {bytes} bytes long, above the {MAX_PREFIX_PATH_BYTES} byte \
                 limit Darling accepts; point DPREFIX at a shorter directory"
            ),
        )
    } else if bytes > PATH_LENGTH_WARN_BYTES {
        (
            CheckStatus::Warn,
            format!(
                "the prefix path is {bytes} bytes long, above the {PATH_LENGTH_WARN_BYTES} byte \
                 comfort limit while still under Darling's {MAX_PREFIX_PATH_BYTES} byte limit; a \
                 long path leaves little room for the entries Darling creates below it"
            ),
        )
    } else {
        (
            CheckStatus::Pass,
            format!(
                "the prefix path is {bytes} bytes long, within the {PATH_LENGTH_WARN_BYTES} byte \
                 comfort limit"
            ),
        )
    };

    Check {
        name: "path_length",
        status,
        detail,
    }
}

fn dedicated_location_check(prefix: &Path) -> Check {
    Check {
        name: "dedicated_location",
        status: CheckStatus::Pass,
        detail: format!(
            "'{}' is a dedicated platform prefix: the platform never shares or modifies the \
             user's conventional ~/.darling prefix, and location resolution rejects it",
            prefix.display()
        ),
    }
}

fn data_dir_outside_prefix_check(location: &PrefixLocation) -> Check {
    let prefix = location.path();
    let data_dir = location.data_dir();

    let (status, detail) = if data_dir.starts_with(prefix) {
        (
            CheckStatus::Warn,
            format!(
                "the platform data directory '{}' is nested inside the prefix '{}'; Darling's \
                 overlay could shadow the lock and state files, so point XDG_DATA_HOME or DPREFIX \
                 elsewhere",
                data_dir.display(),
                prefix.display()
            ),
        )
    } else {
        (
            CheckStatus::Pass,
            format!(
                "the platform data directory '{}' is outside the prefix, so Darling's overlay \
                 cannot shadow the lock and state files",
                data_dir.display()
            ),
        )
    };

    Check {
        name: "data_dir_outside_prefix",
        status,
        detail,
    }
}

fn state_record_check(location: &PrefixLocation) -> Check {
    const NAME: &str = "state_record";

    let (status, detail) = match StateStore::new(location.state_path()).load() {
        Ok(None) => (
            CheckStatus::Pass,
            format!(
                "no state record at '{}' yet; the platform records the phase on the next start or \
                 stop",
                location.state_path().display()
            ),
        ),
        Ok(Some(record)) => (
            CheckStatus::Pass,
            format!(
                "the state record reports phase '{}' (schema version {})",
                phase_name(record.phase),
                record.schema_version
            ),
        ),
        Err(error) => (
            CheckStatus::Fail,
            format!("the state record cannot be trusted: {error}"),
        ),
    };

    Check {
        name: NAME,
        status,
        detail,
    }
}

/// Operator-facing snake_case name of a phase, matching the persisted contract.
fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::NotInitialized => "not_initialized",
        Phase::Initializing => "initializing",
        Phase::Stopped => "stopped",
        Phase::Starting => "starting",
        Phase::Running => "running",
        Phase::Busy => "busy",
        Phase::Stopping => "stopping",
        Phase::Error => "error",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::location::PrefixEnv;

    /// A program path that cannot exist, so `inspect` warnings are deterministic.
    const MISSING_PROGRAM: &str = "/nonexistent/darling-platform-tests/darling";

    /// Diagnostic a failing fake Darling prints, including its trailing newline.
    const FAILURE_STDERR: &str = "darling: cannot create the prefix\n";

    fn success() -> CommandOutcome {
        CommandOutcome {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    fn failure() -> CommandOutcome {
        CommandOutcome {
            exit_code: Some(1),
            stdout: String::new(),
            stderr: String::from(FAILURE_STDERR),
        }
    }

    /// How a [`FakeDarling`] answers a run.
    #[derive(Clone)]
    enum FakeReply {
        Outcome(CommandOutcome),
        SpawnFailure,
    }

    /// One recorded invocation.
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Call {
        prefix: PathBuf,
        args: Vec<String>,
    }

    type Observer = Box<dyn Fn(&Path, &[String]) + Send + Sync>;

    struct FakeInner {
        program: PathBuf,
        reply: FakeReply,
        calls: Mutex<Vec<Call>>,
        observer: Option<Observer>,
    }

    /// A Darling double that records calls, returns a fixed outcome and can watch
    /// the persisted state while it is invoked.
    #[derive(Clone)]
    struct FakeDarling {
        inner: Arc<FakeInner>,
    }

    impl FakeDarling {
        fn new(reply: FakeReply) -> Self {
            Self::observing(reply, None)
        }

        fn observing(reply: FakeReply, observer: Option<Observer>) -> Self {
            Self {
                inner: Arc::new(FakeInner {
                    program: PathBuf::from(MISSING_PROGRAM),
                    reply,
                    calls: Mutex::new(Vec::new()),
                    observer,
                }),
            }
        }

        fn succeeding() -> Self {
            Self::new(FakeReply::Outcome(success()))
        }

        fn failing() -> Self {
            Self::new(FakeReply::Outcome(failure()))
        }

        fn calls(&self) -> Vec<Call> {
            self.inner.calls.lock().expect("fake lock").clone()
        }

        fn boxed(&self) -> Box<dyn Darling> {
            Box::new(self.clone())
        }
    }

    impl Darling for FakeDarling {
        fn program(&self) -> &Path {
            &self.inner.program
        }

        fn run(&self, prefix: &Path, args: &[&str]) -> Result<CommandOutcome> {
            let args: Vec<String> = args.iter().map(|arg| String::from(*arg)).collect();
            self.inner.calls.lock().expect("fake lock").push(Call {
                prefix: prefix.to_path_buf(),
                args: args.clone(),
            });
            if let Some(observer) = &self.inner.observer {
                observer(prefix, &args);
            }
            match &self.inner.reply {
                FakeReply::Outcome(outcome) => Ok(outcome.clone()),
                FakeReply::SpawnFailure => Err(Error::DarlingUnavailable {
                    program: self.inner.program.clone(),
                    source: std::io::Error::from(std::io::ErrorKind::NotFound),
                }),
            }
        }
    }

    /// Resolves a location inside `dir`, with the platform data directory
    /// `xdg/darling-platform` next to the prefix, so nothing touches `$HOME`.
    fn location_in(prefix: &Path, xdg: &Path) -> PrefixLocation {
        PrefixLocation::resolve(&PrefixEnv {
            dprefix: Some(prefix.as_os_str().to_os_string()),
            xdg_data_home: Some(xdg.as_os_str().to_os_string()),
            home: None,
        })
        .expect("the isolated test environment must resolve")
    }

    /// A temporary root with `<root>/prefix` as the prefix and `<root>/data` as
    /// the data directory root.
    fn fixture() -> (tempfile::TempDir, PrefixLocation) {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        let location = location_in(&root.join("prefix"), &root.join("data"));
        (dir, location)
    }

    fn manager(location: &PrefixLocation, darling: &FakeDarling) -> PrefixManager {
        PrefixManager::new(location.clone(), darling.boxed())
    }

    fn state_store(location: &PrefixLocation) -> StateStore {
        StateStore::new(location.state_path())
    }

    fn record(location: &PrefixLocation) -> StateRecord {
        state_store(location)
            .load()
            .expect("state must be readable")
            .expect("state must be present")
    }

    fn check<'a>(validation: &'a Validation, name: &str) -> &'a Check {
        validation
            .checks
            .iter()
            .find(|check| check.name == name)
            .unwrap_or_else(|| panic!("missing check '{name}'"))
    }

    #[test]
    fn inspect_leaves_no_trace_on_disk() {
        let (_dir, location) = fixture();
        let fake = FakeDarling::succeeding();

        let inspection = manager(&location, &fake)
            .inspect()
            .expect("inspect must succeed");

        assert!(!inspection.exists);
        assert_eq!(inspection.phase, Phase::NotInitialized);
        assert!(!inspection.busy);
        assert!(
            !location.state_path().exists(),
            "a read-only inspection must not write a state record"
        );
        assert!(
            !location.lock_path().exists(),
            "a read-only inspection must not create the lock file"
        );
        assert!(
            !location.data_dir().exists(),
            "a read-only inspection must not create the platform data directory"
        );
    }

    #[test]
    fn start_creates_the_missing_prefix_parent_but_never_the_prefix() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        let location = location_in(&root.join("does/not/exist/prefix"), &root.join("data"));
        let fake = FakeDarling::succeeding();

        let phase = manager(&location, &fake)
            .start()
            .expect("start must succeed");

        assert_eq!(phase, Phase::Running);
        assert!(
            root.join("does/not/exist").is_dir(),
            "the platform must create the directory that holds the prefix"
        );
        assert!(
            !location.path().exists(),
            "the prefix itself is Darling's to create"
        );
        assert_eq!(
            fake.calls().len(),
            1,
            "Darling must still be the one starting"
        );
    }

    #[test]
    fn validate_warns_about_a_missing_prefix_parent() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        let location = location_in(&root.join("absent/prefix"), &root.join("data"));

        let validation = manager(&location, &FakeDarling::succeeding())
            .validate()
            .expect("validate must succeed");

        assert_eq!(
            check(&validation, "prefix_parent").status,
            CheckStatus::Warn
        );
    }

    #[test]
    fn validate_fails_when_the_prefix_parent_is_not_a_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        let blocker = root.join("blocked");
        std::fs::write(&blocker, b"not a directory").expect("write blocker");
        let location = location_in(&blocker.join("prefix"), &root.join("data"));

        let validation = manager(&location, &FakeDarling::succeeding())
            .validate()
            .expect("validate must succeed");

        assert_eq!(
            check(&validation, "prefix_parent").status,
            CheckStatus::Fail
        );
        assert!(
            !validation.passed(),
            "a blocking parent must fail validation"
        );
    }

    #[test]
    fn start_calls_shell_true_with_the_resolved_prefix_and_creates_nothing() {
        let (_dir, location) = fixture();
        let fake = FakeDarling::succeeding();

        let phase = manager(&location, &fake)
            .start()
            .expect("start must succeed");

        assert_eq!(phase, Phase::Running);
        assert_eq!(
            fake.calls(),
            vec![Call {
                prefix: location.path().to_path_buf(),
                args: vec![String::from("shell"), String::from("/usr/bin/true")],
            }],
            "start must run exactly `darling shell /usr/bin/true` on the resolved prefix"
        );
        assert!(
            !location.path().exists(),
            "start must never create the prefix directory: Darling does (S5, F2)"
        );
        assert_eq!(record(&location).phase, Phase::Running);
    }

    #[test]
    fn the_transitional_phase_is_persisted_before_darling_runs() {
        for (prefix_exists, expected) in [(true, Phase::Starting), (false, Phase::Initializing)] {
            let (_dir, location) = fixture();
            if prefix_exists {
                std::fs::create_dir_all(location.path()).expect("create prefix dir");
            }

            let observed: Arc<Mutex<Option<StateRecord>>> = Arc::new(Mutex::new(None));
            let sink = Arc::clone(&observed);
            let state_path = location.state_path();
            let observer: Observer = Box::new(move |_prefix: &Path, _args: &[String]| {
                let seen = StateStore::new(&state_path)
                    .load()
                    .expect("state must be readable while Darling runs")
                    .expect("the transitional phase must be written before Darling runs");
                *sink.lock().expect("observed lock") = Some(seen);
            });
            let fake = FakeDarling::observing(FakeReply::Outcome(success()), Some(observer));

            manager(&location, &fake)
                .start()
                .expect("start must succeed");

            let seen = observed
                .lock()
                .expect("observed lock")
                .clone()
                .expect("the fake must have observed the state");
            assert_eq!(seen.phase, expected, "prefix_exists={prefix_exists}");
            assert_eq!(seen.operation.as_deref(), Some(OPERATION_START));
        }
    }

    #[test]
    fn a_failing_darling_is_reported_and_leaves_the_error_phase() {
        let (_dir, location) = fixture();
        let fake = FakeDarling::failing();

        let error = manager(&location, &fake)
            .start()
            .expect_err("a failing Darling must fail start");

        match error {
            Error::DarlingFailed {
                program,
                exit_code,
                stderr,
            } => {
                assert_eq!(program, PathBuf::from(MISSING_PROGRAM));
                assert_eq!(exit_code, Some(1));
                assert_eq!(stderr, FAILURE_STDERR, "the child diagnostic is preserved");
            }
            other => panic!("expected DarlingFailed, got {other:?}"),
        }

        let record = record(&location);
        assert_eq!(record.phase, Phase::Error);
        assert_eq!(record.operation.as_deref(), Some(OPERATION_START));
    }

    #[test]
    fn a_darling_that_cannot_be_spawned_is_reported_and_leaves_the_error_phase() {
        let (_dir, location) = fixture();
        let fake = FakeDarling::new(FakeReply::SpawnFailure);

        let error = manager(&location, &fake)
            .start()
            .expect_err("an unavailable Darling must fail start");

        assert!(
            matches!(error, Error::DarlingUnavailable { .. }),
            "the spawn failure must not be rewritten: {error:?}"
        );
        assert_eq!(record(&location).phase, Phase::Error);
    }

    #[test]
    fn stop_refuses_while_managed_instances_are_active() {
        let (_dir, location) = fixture();
        let seeded = StateRecord::new(Phase::Running, Some(String::from(OPERATION_START)), 1);
        state_store(&location).save(&seeded).expect("seed state");
        let bytes_before = std::fs::read(location.state_path()).expect("read state");
        let fake = FakeDarling::succeeding();

        let error = manager(&location, &fake)
            .stop(1)
            .expect_err("stop must refuse while an instance is alive");

        match error {
            Error::ActiveProcesses { count } => assert_eq!(count, 1),
            other => panic!("expected ActiveProcesses, got {other:?}"),
        }
        assert!(
            fake.calls().is_empty(),
            "stop must not call Darling while instances are alive (F4)"
        );
        assert_eq!(
            std::fs::read(location.state_path()).expect("read state"),
            bytes_before,
            "stop must not write the state file when it refuses"
        );
    }

    #[test]
    fn stop_shuts_down_and_persists_the_stopped_phase() {
        let (_dir, location) = fixture();
        let fake = FakeDarling::succeeding();

        let phase = manager(&location, &fake)
            .stop(0)
            .expect("stop must succeed");

        assert_eq!(phase, Phase::Stopped);
        assert_eq!(
            fake.calls(),
            vec![Call {
                prefix: location.path().to_path_buf(),
                args: vec![String::from("shutdown")],
            }]
        );
        assert_eq!(record(&location).phase, Phase::Stopped);
    }

    #[test]
    fn a_failing_shutdown_is_reported_and_leaves_the_error_phase() {
        let (_dir, location) = fixture();
        let fake = FakeDarling::failing();

        let error = manager(&location, &fake)
            .stop(0)
            .expect_err("a failing shutdown must fail stop");

        assert!(
            matches!(
                error,
                Error::DarlingFailed {
                    exit_code: Some(1),
                    ..
                }
            ),
            "expected DarlingFailed with the child exit code, got {error:?}"
        );
        let record = record(&location);
        assert_eq!(record.phase, Phase::Error);
        assert_eq!(record.operation.as_deref(), Some(OPERATION_STOP));
    }

    #[test]
    fn stop_is_refused_while_the_lock_is_held_and_inspect_stays_responsive() {
        let (_dir, location) = fixture();
        let held = PrefixLock::acquire_exclusive(&location.lock_path()).expect("hold the lock");
        let fake = FakeDarling::succeeding();
        let manager = manager(&location, &fake);

        let error = manager
            .stop(0)
            .expect_err("stop must refuse while another holder keeps the lock");

        match error {
            Error::PrefixBusy { path } => assert_eq!(path, location.lock_path()),
            other => panic!("expected PrefixBusy, got {other:?}"),
        }
        assert!(
            fake.calls().is_empty(),
            "a refused stop must not call Darling"
        );
        assert_eq!(
            state_store(&location)
                .load()
                .expect("state must be readable"),
            None,
            "a refused stop must not write the state file"
        );

        let inspection = manager
            .inspect()
            .expect("inspect must not block on another process's lock");
        assert!(inspection.busy, "inspect must report the lock as busy");

        drop(held);
        assert!(
            !manager.inspect().expect("inspect after release").busy,
            "inspect must report the lock as free once it is released"
        );
    }

    #[test]
    fn an_interrupted_start_is_reported_and_recovered_by_starting_again() {
        let (_dir, location) = fixture();
        std::fs::create_dir_all(location.path()).expect("create prefix dir");
        state_store(&location)
            .save(&StateRecord::new(
                Phase::Starting,
                Some(String::from(OPERATION_START)),
                1,
            ))
            .expect("seed interrupted state");
        let fake = FakeDarling::succeeding();
        let manager = manager(&location, &fake);

        let inspection = manager.inspect().expect("inspect must succeed");

        assert_eq!(
            inspection.phase,
            Phase::Error,
            "a phase that stayed mid-transition is reported as a failure (ADR-0008)"
        );
        assert_eq!(inspection.interrupted.as_deref(), Some(OPERATION_START));
        assert!(
            inspection
                .warnings
                .iter()
                .any(|warning| warning.contains("InterruptedOperation")
                    && warning.contains(OPERATION_START)),
            "the interrupted operation must be named in a warning: {:?}",
            inspection.warnings
        );

        let phase = manager
            .start()
            .expect("start must recover the interrupted run");
        assert_eq!(phase, Phase::Running);
        assert_eq!(
            manager.inspect().expect("inspect after recovery").phase,
            Phase::Running
        );
    }

    #[test]
    fn inspect_reports_an_uninitialized_prefix() {
        let (_dir, location) = fixture();

        let inspection = manager(&location, &FakeDarling::succeeding())
            .inspect()
            .expect("inspect must succeed");

        assert_eq!(inspection.phase, Phase::NotInitialized);
        assert!(!inspection.exists);
        assert_eq!(inspection.prefix, location.path());
        assert_eq!(inspection.interrupted, None);
        assert!(!inspection.busy);
        assert_eq!(inspection.darling_program, PathBuf::from(MISSING_PROGRAM));
    }

    #[test]
    fn inspect_warns_when_the_prefix_exists_without_a_state_record() {
        let (_dir, location) = fixture();
        std::fs::create_dir_all(location.path()).expect("create prefix dir");

        let inspection = manager(&location, &FakeDarling::succeeding())
            .inspect()
            .expect("inspect must succeed");

        assert_eq!(inspection.phase, Phase::Stopped);
        assert!(inspection.exists);
        assert!(
            inspection
                .warnings
                .iter()
                .any(|warning| warning.contains("no state record")),
            "an unrecorded prefix must be warned about: {:?}",
            inspection.warnings
        );
    }

    #[test]
    fn inspect_warns_when_the_running_prefix_directory_is_gone() {
        let (_dir, location) = fixture();
        state_store(&location)
            .save(&StateRecord::new(
                Phase::Running,
                Some(String::from(OPERATION_START)),
                1,
            ))
            .expect("seed running state");

        let inspection = manager(&location, &FakeDarling::succeeding())
            .inspect()
            .expect("inspect must succeed");

        assert_eq!(inspection.phase, Phase::Running);
        assert!(!inspection.exists);
        assert!(
            inspection
                .warnings
                .iter()
                .any(|warning| warning.contains("does not exist")),
            "a running phase without a prefix must be warned about: {:?}",
            inspection.warnings
        );
    }

    #[test]
    fn inspect_warns_when_the_darling_program_is_missing() {
        let (_dir, location) = fixture();

        let inspection = manager(&location, &FakeDarling::succeeding())
            .inspect()
            .expect("a missing Darling program must not fail inspect");

        assert!(
            inspection
                .warnings
                .iter()
                .any(|warning| warning.contains(MISSING_PROGRAM)),
            "a missing Darling program must be warned about: {:?}",
            inspection.warnings
        );
    }

    #[test]
    fn validate_accepts_an_absent_prefix() {
        let (_dir, location) = fixture();

        let validation = manager(&location, &FakeDarling::succeeding())
            .validate()
            .expect("validate must succeed");

        let exists = check(&validation, "prefix_exists");
        assert_eq!(exists.status, CheckStatus::Warn);
        assert!(
            exists
                .detail
                .contains("not initialized yet; `dg prefix start` creates it through Darling"),
            "unexpected detail: {}",
            exists.detail
        );
        assert_eq!(
            check(&validation, "owned_by_current_user").status,
            CheckStatus::NotApplicable
        );
        assert_eq!(check(&validation, "state_record").status, CheckStatus::Pass);
        assert!(validation.passed(), "an absent prefix is not a failure");
    }

    #[test]
    fn validate_rejects_a_prefix_that_is_not_a_directory() {
        let (_dir, location) = fixture();
        std::fs::create_dir_all(location.path().parent().expect("parent"))
            .expect("create prefix parent");
        std::fs::write(location.path(), b"not a directory").expect("create prefix file");

        let validation = manager(&location, &FakeDarling::succeeding())
            .validate()
            .expect("validate must succeed");

        assert_eq!(check(&validation, "is_directory").status, CheckStatus::Fail);
        assert!(!validation.passed());
    }

    #[test]
    fn validate_rejects_a_symlinked_prefix() {
        let (_dir, location) = fixture();
        let target = location
            .path()
            .parent()
            .expect("parent")
            .join("prefix-target");
        std::fs::create_dir_all(&target).expect("create target dir");
        std::os::unix::fs::symlink(&target, location.path()).expect("create symlink");

        let validation = manager(&location, &FakeDarling::succeeding())
            .validate()
            .expect("validate must succeed");

        assert_eq!(check(&validation, "not_symlink").status, CheckStatus::Fail);
        assert_eq!(
            check(&validation, "is_directory").status,
            CheckStatus::Pass,
            "the symlink target is a directory; only the symlink check must fail"
        );
        assert!(!validation.passed());
    }

    #[test]
    fn validate_warns_about_a_long_but_accepted_prefix_path() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        let base = root.as_os_str().as_bytes().len();
        let target_len = PATH_LENGTH_WARN_BYTES + 20;
        assert!(
            base + 1 < target_len,
            "the temp root leaves room for the name"
        );
        let name = "a".repeat(target_len - base - 1);
        let prefix = root.join(&name);
        assert_eq!(prefix.as_os_str().as_bytes().len(), target_len);

        let location = location_in(&prefix, &root.join("data"));
        let validation = manager(&location, &FakeDarling::succeeding())
            .validate()
            .expect("validate must succeed");

        let path_length = check(&validation, "path_length");
        assert_eq!(path_length.status, CheckStatus::Warn);
        assert!(
            path_length.detail.contains(&target_len.to_string()),
            "the detail must carry the byte count: {}",
            path_length.detail
        );
        assert!(
            validation.passed(),
            "a long path is a warning, not a failure"
        );
    }

    /// A path of exactly `bytes` bytes, built from a `/p/` root (3 bytes).
    fn path_of_len(bytes: usize) -> PathBuf {
        let path = Path::new("/p").join("a".repeat(bytes - 3));
        assert_eq!(path.as_os_str().as_bytes().len(), bytes);
        path
    }

    #[test]
    fn path_length_fails_above_the_darling_limit() {
        // `PrefixLocation::resolve` already refuses anything longer than the limit,
        // so the boundary itself is asserted here instead of through a location.
        for bytes in [MAX_PREFIX_PATH_BYTES + 1, MAX_PREFIX_PATH_BYTES + 40] {
            let over = path_of_len(bytes);
            let check = path_length_check(&over);
            assert_eq!(
                check.status,
                CheckStatus::Fail,
                "{bytes} bytes: {}",
                check.detail
            );
            assert!(
                check.detail.contains(&bytes.to_string()),
                "the detail must carry the byte count: {}",
                check.detail
            );
        }

        assert_eq!(
            path_length_check(&path_of_len(MAX_PREFIX_PATH_BYTES)).status,
            CheckStatus::Warn,
            "the limit itself is still accepted"
        );
        assert_eq!(
            path_length_check(&path_of_len(PATH_LENGTH_WARN_BYTES + 1)).status,
            CheckStatus::Warn,
            "a path just above the comfort limit only warns"
        );
        assert_eq!(
            path_length_check(&path_of_len(PATH_LENGTH_WARN_BYTES)).status,
            CheckStatus::Pass,
            "the warning threshold itself still passes"
        );
    }

    #[test]
    fn validate_warns_when_the_data_dir_nests_inside_the_prefix() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        let prefix = root.join("shared");
        let location = location_in(&prefix, &prefix.join("data"));
        assert!(
            location.data_dir().starts_with(location.path()),
            "the fixture must nest the data dir inside the prefix"
        );

        let validation = manager(&location, &FakeDarling::succeeding())
            .validate()
            .expect("validate must succeed");

        assert_eq!(
            check(&validation, "data_dir_outside_prefix").status,
            CheckStatus::Warn
        );
        assert!(validation.passed());
    }

    #[test]
    fn validate_fails_when_the_state_record_is_unreadable() {
        let (_dir, location) = fixture();
        std::fs::create_dir_all(location.data_dir()).expect("create data dir");
        std::fs::write(
            location.state_path(),
            b"{\"schema_version\":99,\"phase\":\"running\",\"updated_at\":1}",
        )
        .expect("write newer state");

        let validation = manager(&location, &FakeDarling::succeeding())
            .validate()
            .expect("validate itself must not fail");

        assert_eq!(check(&validation, "state_record").status, CheckStatus::Fail);
        assert!(!validation.passed());
    }

    #[test]
    fn validate_reports_the_dedicated_location_policy() {
        let (_dir, location) = fixture();

        let validation = manager(&location, &FakeDarling::succeeding())
            .validate()
            .expect("validate must succeed");

        let dedicated = check(&validation, "dedicated_location");
        assert_eq!(dedicated.status, CheckStatus::Pass);
        assert!(
            dedicated.detail.contains("~/.darling"),
            "the conventional prefix policy must be named: {}",
            dedicated.detail
        );
    }

    #[test]
    fn phase_names_match_the_on_disk_contract() {
        for phase in [
            Phase::NotInitialized,
            Phase::Initializing,
            Phase::Stopped,
            Phase::Starting,
            Phase::Running,
            Phase::Busy,
            Phase::Stopping,
            Phase::Error,
        ] {
            let json = serde_json::to_value(StateRecord::new(phase, None, 1)).expect("serialize");
            assert_eq!(
                json["phase"].as_str(),
                Some(phase_name(phase)),
                "phase_name must match the persisted name of {phase:?}"
            );
        }
    }
}
