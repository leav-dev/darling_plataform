//! Darling App Installer (DAI).
//!
//! Responsibility: import `.app` bundles and — once the flow is verified — ZIP
//! archives holding one; extract them into staging, validate paths and structure
//! with the bundle inspector, install the bundle into the shared DPREFIX and
//! record the attempt in the application library.
//!
//! Not responsible for: resolving or locking the prefix, executing anything,
//! declaring compatibility, or overwriting an existing installation without an
//! explicit policy. The CLI resolves the DPREFIX, takes the Shared Prefix
//! Manager's write lock and hands the installer the paths it may write to, so an
//! install cannot start while an instance is active and this crate never shells
//! out.
//!
//! # Formats
//!
//! This cut imports, in the order the plan sets:
//!
//! 1. `.app` — a bundle that is already extracted.
//! 2. `.zip` — an archive holding a bundle. Only the stored and deflate methods
//!    are supported, both in pure Rust, because the package must not gain a
//!    system dependency.
//!
//! `.dmg`, `.pkg` and `.xip` are refused with an error that names the path and
//! lists what is supported: each has its own mounting or installing flow, and
//! PKG runs scripts, so neither arrives here without its own design. The
//! extension is what identifies the format, so a `.zip` is not sniffed as
//! anything else.
//!
//! # The flow, in order
//!
//! 1. Identify the format from the source's extension. An unsupported source is
//!    not an installation attempt and is not recorded: `installations.format`
//!    stores the format vocabulary, and a source with no supported format has no
//!    value to store.
//! 2. Stage into `<applications_dir>/.staging/<installation_id>`, on the same
//!    file system as the destination.
//! 3. Extract the archive or copy the bundle into staging, validating every
//!    path as it is written (see [`archive`] and [`bundle`]).
//! 4. Find exactly one `.app` below the staging area. Zero or several are both
//!    errors, and each names what was staged.
//! 5. Inspect it with [`dp_bundle_inspector`]: a bundle whose metadata cannot be
//!    trusted is never placed in the prefix.
//! 6. Derive the destination `<applications_dir>/<name>.app` from the metadata
//!    name (see [`destination_name`]).
//! 7. Refuse an existing destination unless the request asked to replace it.
//! 8. Move the staged bundle into place with a single `std::fs::rename`.
//! 9. Record the application and the installation in the library, in one
//!    transaction.
//! 10. Remove the staging area.
//!
//! # Replace policy
//!
//! An installation is only overwritten when the request sets `replace`, which is
//! the explicit policy the plan requires. A replacement moves the installed copy
//! to `<staging>/<installation_id>-replaced`, renames the staged bundle into
//! place and deletes the moved copy; if the second rename fails, the original is
//! put back and the error reports the failure. If the catalogue refuses the new
//! row, the swap is undone: the new copy is removed and the replaced one
//! restored, so a failed import never leaves an application the catalogue cannot
//! list.
//!
//! # Failure accounting
//!
//! Every error after the format is accepted writes one [`InstallationRecord`]
//! with `result = "failed"`, no `application_id` and the error message as its
//! `detail`. No destination, no partial bundle and no staging directory survive
//! a failure: the staging guard removes its directory on every path out.
//!
//! The record of a failed attempt is itself a row, so `installation_id` must be
//! unique per attempt. A caller that reuses it gets the original install error,
//! because that is what failed, together with [`Error::FailureNotRecorded`]: the
//! returned error states that the failed attempt could not be recorded and names
//! the collision, so a silently missing history row is never reported as a
//! different kind of failure.

mod archive;
mod bundle;
pub mod error;

#[cfg(test)]
mod test_support;

use std::ffi::OsStr;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use dp_app_library::{ApplicationRecord, InstallationRecord, Library};
use dp_bundle_inspector::{Architecture, BundleMetadata};

pub use error::{Error, Result};

/// Directory inside `applications_dir` that holds the staging areas.
const STAGING_DIRECTORY: &str = ".staging";

/// Suffix of a macOS application bundle.
const BUNDLE_SUFFIX: &str = ".app";

/// A bundle's metadata and resources directory.
const CONTENTS_DIRECTORY: &str = "Contents";

/// A bundle's resources directory, where a declared icon is resolved.
const RESOURCES_DIRECTORY: &str = "Resources";

/// Separator of the architecture list the catalogue stores.
const ARCHITECTURE_SEPARATOR: &str = ",";

/// `applications.installation_status` of a bundle that is in place.
const INSTALLED: &str = "installed";

/// `installations.result` of an attempt that ended with an error.
const FAILED: &str = "failed";

/// The input format this cut can import, in the order the plan sets them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    /// An extracted `.app` bundle directory, copied as it is.
    App,
    /// A `.zip` archive holding a bundle, extracted into staging.
    Zip,
}

impl SourceFormat {
    /// Identifies the format of `source` by its extension.
    ///
    /// The extension is matched case-insensitively, because a macOS filesystem
    /// does not distinguish `Example.ZIP` from `Example.zip`. Everything else —
    /// `.dmg`, `.pkg`, `.xip`, no extension — is refused with the path and the
    /// formats that are supported.
    fn identify(source: &Path) -> Result<Self> {
        let extension = source.extension().and_then(OsStr::to_str);
        match extension.map(str::to_ascii_lowercase).as_deref() {
            Some("app") => Ok(Self::App),
            Some("zip") => Ok(Self::Zip),
            _ => Err(Error::UnsupportedFormat {
                path: source.to_path_buf(),
                found: describe_extension(extension),
            }),
        }
    }

    /// The value `installations.format` stores for this input.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Zip => "zip",
        }
    }

    /// The value `applications.installation_type` stores for the result: a
    /// `.app` import is a copied bundle, a ZIP import is an extracted archive.
    pub fn installation_type(self) -> &'static str {
        match self {
            Self::App => "bundle",
            Self::Zip => "zip",
        }
    }
}

impl fmt::Display for SourceFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Everything one installation attempt needs.
///
/// The installer takes the paths it must write to and the catalogue it must
/// record in. It does not read the environment, resolve the DPREFIX or take the
/// Shared Prefix Manager's write lock: the caller does all three before it builds
/// a request, which keeps an install from racing a running instance and keeps
/// this crate testable without a prefix.
pub struct InstallRequest<'a> {
    /// The `.app` directory or `.zip` archive to import.
    pub source: &'a Path,
    /// `<DPREFIX>/Applications`, the directory installed bundles live in.
    pub applications_dir: &'a Path,
    /// The catalogue the attempt is recorded in, successful or not.
    pub library: &'a Library,
    /// Stable identifier of this attempt. It names the staging directory and is
    /// stored as the `installations.id` of the row this attempt writes, so the
    /// caller must not reuse it for a second attempt: a reused id cannot be
    /// recorded, and the error then states that the failed attempt was not
    /// recorded (see [`Error::FailureNotRecorded`]).
    pub installation_id: &'a str,
    /// Whether an application already installed at the destination may be
    /// replaced. Without it an existing destination is an error.
    pub replace: bool,
    /// RFC 3339 UTC timestamp of the attempt, stored as given.
    pub now: &'a str,
}

/// What an import installed, and what the inspector proved about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledApplication {
    /// The bundle's metadata, with `path` and `executable_path` rewritten to name
    /// the installed copy instead of the staging directory it was read from.
    pub metadata: BundleMetadata,
    /// The installed bundle, `<applications_dir>/<name>.app`.
    pub path: PathBuf,
    /// The installation id the attempt was recorded under.
    pub installation_id: String,
    /// The input format the bundle came from.
    pub format: SourceFormat,
}

/// Imports, validates and installs one application, recording the attempt.
///
/// The bundle is staged first, inspected before anything is placed in the prefix,
/// and moved into place with one rename, so no half-installed application is ever
/// visible. The request is accepted once its format is supported; from that point
/// on every outcome, including every error, is recorded in the application
/// library.
pub fn install(request: &InstallRequest<'_>) -> Result<InstalledApplication> {
    let format = SourceFormat::identify(request.source)?;
    match attempt(request, format) {
        Ok(installed) => Ok(installed),
        Err(error) => Err(record_failure(request, format, error)),
    }
}

/// Runs the documented flow. The staging guard cleans up whatever happens next.
fn attempt(request: &InstallRequest<'_>, format: SourceFormat) -> Result<InstalledApplication> {
    let staging = Staging::create(request.applications_dir, request.installation_id)?;
    match format {
        SourceFormat::App => bundle::copy(request.source, staging.path())?,
        SourceFormat::Zip => archive::extract(request.source, staging.path())?,
    }

    let staged = bundle::single(staging.path())?;
    let inspected = dp_bundle_inspector::inspect(&staged).map_err(|source| Error::Inspection {
        bundle: staged.clone(),
        source: Box::new(source),
    })?;

    let destination = request.applications_dir.join(format!(
        "{}{BUNDLE_SUFFIX}",
        destination_name(&inspected.name)?
    ));
    let executable_path = rebase(&inspected.executable_path, &staged, &destination);
    let icon_path = declared_icon(&inspected, &staged, &destination)?;

    // The swap: the installed copy is moved aside first, so the staged bundle
    // always lands in a free path and the operator never sees a directory that
    // holds half of each version.
    let mut replaced: Option<PathBuf> = None;
    if destination.exists() {
        if !request.replace {
            return Err(Error::DestinationExists { path: destination });
        }
        let backup = staging.replaced_path();
        std::fs::rename(&destination, &backup).map_err(|source| Error::Io {
            path: destination.clone(),
            source,
        })?;
        replaced = Some(backup);
    }
    if let Err(source) = std::fs::rename(&staged, &destination) {
        // The staged bundle could not be placed, so the installation that was
        // moved aside still has the destination to itself.
        if let Some(backup) = replaced.take() {
            restore(&backup, &destination)?;
        }
        return Err(Error::Io {
            path: destination,
            source,
        });
    }

    let application = ApplicationRecord {
        // The bundle identifier identifies the application, so a second install
        // of the same application updates its row instead of adding one.
        id: inspected.bundle_id.clone(),
        bundle_id: inspected.bundle_id.clone(),
        name: inspected
            .display_name
            .clone()
            .unwrap_or_else(|| inspected.name.clone()),
        version: inspected.version.clone(),
        bundle_path: destination.clone(),
        executable_path: executable_path.clone(),
        icon_path: icon_path.clone(),
        architecture: architecture_list(&inspected.architectures),
        installation_type: format.installation_type().to_string(),
        installed_at: request.now.to_string(),
        installation_status: INSTALLED.to_string(),
    };
    let installation = InstallationRecord {
        id: request.installation_id.to_string(),
        application_id: Some(application.id.clone()),
        origin: request.source.display().to_string(),
        format: format.as_str().to_string(),
        result: INSTALLED.to_string(),
        detail: None,
        occurred_at: request.now.to_string(),
    };
    if let Err(error) = store(request, &application, &installation) {
        // A bundle the catalogue does not know about cannot be listed or
        // uninstalled, so the file system goes back to what it was: the new copy
        // is removed and the replaced one put back in its place.
        roll_back(&destination, replaced.as_deref())?;
        return Err(error);
    }
    // The replaced copy is deleted with the staging area, which the guard removes
    // when this function returns.
    if let Some(backup) = replaced {
        std::fs::remove_dir_all(&backup).map_err(|source| Error::Io {
            path: backup,
            source,
        })?;
    }

    Ok(InstalledApplication {
        metadata: BundleMetadata {
            path: destination.clone(),
            executable_path,
            ..inspected
        },
        path: destination,
        installation_id: request.installation_id.to_string(),
        format,
    })
}

/// Records one failed attempt and returns the error that reports the failure.
///
/// An import that left nothing installed still tells the operator what was tried
/// and why it failed, so the attempt is written as a `failed` history row. That
/// row carries `installation_id`, which is unique per attempt: when the history
/// already holds it the attempt cannot be recorded, and the returned error
/// carries both causes — the install failure that stopped the attempt and the
/// refusal that kept it out of the catalogue — instead of replacing the first
/// with a library error the operator cannot act on.
fn record_failure(request: &InstallRequest<'_>, format: SourceFormat, error: Error) -> Error {
    let record = InstallationRecord {
        id: request.installation_id.to_string(),
        // A failed attempt produced no application row to point at.
        application_id: None,
        origin: request.source.display().to_string(),
        format: format.as_str().to_string(),
        result: FAILED.to_string(),
        detail: Some(error.to_string()),
        occurred_at: request.now.to_string(),
    };
    match request.library.record_installation(&record) {
        Ok(()) => error,
        Err(source) => Error::FailureNotRecorded {
            install: Box::new(error),
            record: Box::new(Error::Library {
                operation: "recording a failed installation",
                source: Box::new(source),
            }),
        },
    }
}

/// Derives the destination directory name from the name the bundle declares.
///
/// `CFBundleName` is untrusted input and need not be a directory name, so the
/// value is sanitized instead of used verbatim: a trailing `.app` is stripped
/// (the destination gets its own), `/` and `\` become `-`, and the result is
/// trimmed. A name that is empty, `.` or `..` after that would place the bundle
/// over the applications directory or outside it, so it is rejected with the
/// declared value. Any other value that is not a single ordinary path component —
/// a drive prefix, a second separator, a NUL byte — is rejected the same way.
fn destination_name(declared: &str) -> Result<String> {
    let name = declared.strip_suffix(BUNDLE_SUFFIX).unwrap_or(declared);
    let name: String = name
        .chars()
        .map(|character| match character {
            '/' | '\\' => '-',
            other => other,
        })
        .collect();
    let name = name.trim().to_string();
    if name.is_empty() || name == "." || name == ".." || !is_single_component(&name) {
        return Err(Error::InvalidBundleName {
            declared: declared.to_string(),
        });
    }
    Ok(name)
}

/// Proves `value` is one ordinary path component, so joining it onto a directory
/// cannot leave that directory or climb out of it.
fn is_single_component(value: &str) -> bool {
    if value.contains('\0') {
        return false;
    }
    let mut components = Path::new(value).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(component)), None) => component == OsStr::new(value),
        _ => false,
    }
}

/// Proves an installation id can name one directory inside the staging area.
fn installation_component(installation_id: &str) -> Result<&str> {
    if is_single_component(installation_id) {
        Ok(installation_id)
    } else {
        Err(Error::InvalidInstallationId {
            value: installation_id.to_string(),
        })
    }
}

/// Resolves the icon the bundle declares against the installed copy.
///
/// The catalogue may only point at a file the install owns, so the declared icon
/// is reported only when it exists inside the staged bundle once symlinks are
/// resolved; the reported path names the same file inside the installed copy. A
/// declared icon the bundle does not ship is not an error: the inspector reports
/// it without resolving it, and macOS falls back to the executable's icon.
fn declared_icon(
    metadata: &BundleMetadata,
    staged: &Path,
    destination: &Path,
) -> Result<Option<PathBuf>> {
    let Some(declared) = metadata.icon_file.as_deref() else {
        return Ok(None);
    };
    let path = staged
        .join(CONTENTS_DIRECTORY)
        .join(RESOURCES_DIRECTORY)
        .join(declared);
    let root = staged.canonicalize().map_err(|source| Error::Io {
        path: staged.to_path_buf(),
        source,
    })?;
    match path.canonicalize() {
        Ok(target) if target.starts_with(&root) => Ok(Some(rebase(&path, staged, destination))),
        // A link that resolves outside the bundle is not an icon this install
        // owns; the inspector has already refused the declared value itself.
        Ok(_) => Ok(None),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Io { path, source }),
    }
}

/// Rewrites a path the inspector reported inside the staged bundle so it names
/// the same file inside the installed copy.
fn rebase(path: &Path, staged: &Path, destination: &Path) -> PathBuf {
    match path.strip_prefix(staged) {
        Ok(relative) => destination.join(relative),
        // Unreachable: the inspector builds every reported path from the bundle
        // it was handed. Keeping the path instead of panicking keeps a reporting
        // bug from aborting an install that has already been placed.
        Err(_) => path.to_path_buf(),
    }
}

/// Joins the executable's architectures into the single text the catalogue
/// stores, using the names the bundle report uses.
fn architecture_list(architectures: &[Architecture]) -> Option<String> {
    if architectures.is_empty() {
        return None;
    }
    let names: Vec<String> = architectures
        .iter()
        .map(|item| architecture_name(*item))
        .collect();
    Some(names.join(ARCHITECTURE_SEPARATOR))
}

/// Names one architecture. A CPU type this build does not know keeps its raw
/// value, so a record never claims a name the executable did not have.
fn architecture_name(architecture: Architecture) -> String {
    match architecture {
        Architecture::X86_64 => String::from("x86_64"),
        Architecture::Arm64 => String::from("arm64"),
        Architecture::Arm => String::from("arm"),
        Architecture::I386 => String::from("i386"),
        Architecture::PowerPc => String::from("powerpc"),
        Architecture::Unknown(cputype) => format!("unknown(0x{cputype:08x})"),
    }
}

/// Stores the two rows of a successful attempt as one unit.
///
/// The application row and its installation row are written in one transaction,
/// so the catalogue holds both or neither: a row the library refuses leaves no
/// application behind that names a path the roll back then removes, and no
/// history entry that names an application nothing stored. That is what lets
/// [`roll_back`] put the file system back knowing the catalogue holds nothing of
/// the attempt.
///
/// A replacement overwrites the row of the application it replaces: the row id is
/// the bundle identifier, so reinstalling an application must not add a second
/// row for it. Without the replace policy an already registered row is a conflict
/// the operator has to see, not a silent overwrite.
fn store(
    request: &InstallRequest<'_>,
    application: &ApplicationRecord,
    installation: &InstallationRecord,
) -> Result<()> {
    let written = if request.replace {
        request.library.replace_install(application, installation)
    } else {
        request.library.record_install(application, installation)
    };
    written.map_err(|source| Error::Library {
        operation: "recording the installation",
        source: Box::new(source),
    })
}

/// Removes the freshly installed copy and puts the replaced one back.
///
/// Used when the catalogue refuses a row after the swap: the file system returns
/// to the state the attempt found it in, so a failed import never leaves an
/// application that nothing can list or uninstall.
fn roll_back(destination: &Path, replaced: Option<&Path>) -> Result<()> {
    std::fs::remove_dir_all(destination).map_err(|source| Error::Io {
        path: destination.to_path_buf(),
        source,
    })?;
    match replaced {
        Some(backup) => restore(backup, destination),
        None => Ok(()),
    }
}

/// Puts the copy the swap moved aside back at `destination`.
fn restore(backup: &Path, destination: &Path) -> Result<()> {
    std::fs::rename(backup, destination).map_err(|source| Error::RestoreFailed {
        destination: destination.to_path_buf(),
        backup: backup.to_path_buf(),
        source,
    })
}

/// The staging area of one installation attempt.
///
/// Every attempt stages into `<applications_dir>/.staging/<installation_id>`,
/// which is on the same file system as the destination, so placing the bundle is
/// one `rename`: atomic, and never a copy an operator can catch half-written.
///
/// The directory is removed when the guard is dropped, which is what makes "no
/// leftover staging directory" hold on every path out, including the ones the
/// flow cannot reach. A drop cannot report an error, and a staging directory is
/// never an installed application: a repeated installation id clears it before
/// the next attempt writes there.
struct Staging {
    path: PathBuf,
}

impl Staging {
    /// Creates the staging directory, clearing what a crashed attempt left.
    fn create(applications_dir: &Path, installation_id: &str) -> Result<Self> {
        let path = applications_dir
            .join(STAGING_DIRECTORY)
            .join(installation_component(installation_id)?);
        if path.exists() {
            std::fs::remove_dir_all(&path).map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
        }
        std::fs::create_dir_all(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        Ok(Self { path })
    }

    /// The directory everything is extracted or copied into.
    fn path(&self) -> &Path {
        &self.path
    }

    /// Where the installed copy is kept while a replacement is in flight. It sits
    /// inside the staging area, so cleanup removes it with everything else.
    fn replaced_path(&self) -> PathBuf {
        let mut path = self.path.clone().into_os_string();
        path.push("-replaced");
        PathBuf::from(path)
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
        // The area itself only goes away when it is empty, so another attempt's
        // staging directory is never removed.
        if let Some(area) = self.path.parent() {
            let _ = std::fs::remove_dir(area);
        }
    }
}

/// Names the extension a source carries, for the message that refuses it.
fn describe_extension(extension: Option<&str>) -> &'static str {
    match extension.map(str::to_ascii_lowercase).as_deref() {
        Some("dmg") => "a .dmg disk image",
        Some("pkg") => "a .pkg installer package, which runs scripts and cannot be rolled back",
        Some("xip") => "a .xip archive",
        Some(_) => "an unsupported file type",
        None => "a file without an extension",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        CPU_TYPE_ARM64, CPU_TYPE_X86_64, create_dir, entry, example_bundle, example_plist,
        fat_macho, plist, thin_macho, write_bundle, write_file, write_zip, write_zip_with_symlink,
        zip_bundle,
    };
    use tempfile::TempDir;

    /// The identifier [`example_plist`] declares.
    const EXAMPLE: &str = "com.example.example";
    /// The timestamp every attempt in these tests is recorded with.
    const NOW: &str = "2026-11-09T10:00:00Z";
    /// A later timestamp, for the attempt that replaces the first one.
    const LATER: &str = "2026-11-10T11:00:00Z";

    /// One test's own applications directory and catalogue, in a fresh temporary
    /// directory that is removed when the fixture is dropped.
    struct Fixture {
        directory: TempDir,
        applications: PathBuf,
        library: Library,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().expect("create the temporary directory");
            let applications = directory.path().join("Applications");
            create_dir(&applications);
            let library = Library::open(&directory.path().join("library.sqlite3"))
                .expect("open a fresh catalogue");
            Self {
                directory,
                applications,
                library,
            }
        }

        fn staging(&self) -> PathBuf {
            self.applications.join(STAGING_DIRECTORY)
        }

        fn applications_rows(&self) -> Vec<ApplicationRecord> {
            self.library
                .list_applications()
                .expect("read the applications")
        }

        fn attempts(&self) -> Vec<InstallationRecord> {
            self.library
                .installations()
                .expect("read the installation history")
        }

        /// Proves the attempt left no staging directory, not even the area.
        fn assert_no_staging(&self) {
            assert!(
                !self.staging().exists(),
                "the staging area '{}' must be removed",
                self.staging().display()
            );
        }
    }

    fn request<'a>(
        fixture: &'a Fixture,
        source: &'a Path,
        installation_id: &'a str,
    ) -> InstallRequest<'a> {
        InstallRequest {
            source,
            applications_dir: &fixture.applications,
            library: &fixture.library,
            installation_id,
            replace: false,
            now: NOW,
        }
    }

    /// Proves nothing was installed and nothing was staged.
    fn assert_nothing_installed(fixture: &Fixture) {
        assert!(
            fixture.applications_rows().is_empty(),
            "a failed import records no application"
        );
        let installed: Vec<String> = std::fs::read_dir(&fixture.applications)
            .expect("read the applications directory")
            .map(|entry| {
                entry
                    .expect("read the entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| name.ends_with(BUNDLE_SUFFIX))
            .collect();
        assert!(
            installed.is_empty(),
            "a failed import installs nothing: {installed:?}"
        );
        fixture.assert_no_staging();
    }

    /// Proves one failed attempt was recorded, with the cause in its detail.
    fn assert_failed_attempt(fixture: &Fixture, installation_id: &str, format: &str, cause: &str) {
        let attempts = fixture.attempts();
        assert_eq!(
            attempts.len(),
            1,
            "exactly one failed attempt is recorded: {attempts:?}"
        );
        let attempt = &attempts[0];
        assert_eq!(attempt.id, installation_id);
        assert_eq!(
            attempt.application_id, None,
            "a failed attempt belongs to no application"
        );
        assert_eq!(attempt.format, format);
        assert_eq!(attempt.result, FAILED);
        assert_eq!(attempt.occurred_at, NOW);
        let detail = attempt
            .detail
            .as_deref()
            .expect("a failure explains itself");
        assert!(
            detail.contains(cause),
            "the detail must name the cause '{cause}': {detail}"
        );
    }

    #[test]
    fn an_app_bundle_is_copied_installed_and_recorded() {
        let fixture = Fixture::new();
        let source = example_bundle(fixture.directory.path());
        let destination = fixture.applications.join("Example.app");

        let installed = install(&request(&fixture, &source, "installation-1"))
            .expect("the bundle is installed");

        assert!(destination.join("Contents").join("Info.plist").is_file());
        assert!(
            destination
                .join("Contents")
                .join("MacOS")
                .join("Example")
                .is_file()
        );
        assert!(
            destination
                .join("Contents")
                .join("Resources")
                .join("Example.icns")
                .is_file(),
            "the bundle is copied whole, resources included"
        );
        assert_eq!(installed.path, destination);
        assert_eq!(installed.format, SourceFormat::App);
        assert_eq!(installed.installation_id, "installation-1");
        assert_eq!(
            installed.metadata.path, destination,
            "the report names the installed copy, not the staging directory"
        );
        assert_eq!(
            installed.metadata.executable_path,
            destination.join("Contents").join("MacOS").join("Example")
        );

        let rows = fixture.applications_rows();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.id, EXAMPLE, "the row id is the bundle identifier");
        assert_eq!(row.bundle_id, EXAMPLE);
        assert_eq!(row.name, "Example");
        assert_eq!(row.version.as_deref(), Some("1.2.3"));
        assert_eq!(row.bundle_path, destination);
        assert_eq!(
            row.executable_path,
            destination.join("Contents").join("MacOS").join("Example")
        );
        assert_eq!(
            row.icon_path,
            Some(
                destination
                    .join("Contents")
                    .join("Resources")
                    .join("Example.icns")
            )
        );
        assert_eq!(row.architecture.as_deref(), Some("x86_64"));
        assert_eq!(row.installation_type, "bundle");
        assert_eq!(row.installation_status, INSTALLED);
        assert_eq!(row.installed_at, NOW);

        let history = fixture
            .library
            .histories(EXAMPLE)
            .expect("read the history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, "installation-1");
        assert_eq!(history[0].application_id.as_deref(), Some(EXAMPLE));
        assert_eq!(history[0].origin, source.display().to_string());
        assert_eq!(history[0].format, "app");
        assert_eq!(history[0].result, INSTALLED);
        assert_eq!(history[0].occurred_at, NOW);
        fixture.assert_no_staging();
    }

    #[test]
    fn a_zip_archive_is_extracted_installed_and_recorded() {
        let fixture = Fixture::new();
        let plist = plist(
            Some("com.example.zipped"),
            "Zipped",
            Some("3.4"),
            "Zipped",
            Some("Zipped.icns"),
        );
        let archive = fixture.directory.path().join("Zipped.zip");
        let mut entries = zip_bundle("Zipped.app", "Zipped", &plist);
        entries.push(entry("Zipped.app/Contents/Resources/keep.txt", b"resource"));
        write_zip(&archive, &entries);

        let installed = install(&request(&fixture, &archive, "installation-2"))
            .expect("the archive is installed");

        let destination = fixture.applications.join("Zipped.app");
        assert_eq!(installed.format, SourceFormat::Zip);
        assert_eq!(installed.path, destination);
        assert!(
            destination
                .join("Contents")
                .join("Resources")
                .join("keep.txt")
                .is_file(),
            "the archive is extracted whole"
        );
        let rows = fixture.applications_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].installation_type, "zip");
        assert_eq!(rows[0].version.as_deref(), Some("3.4"));
        assert_eq!(
            rows[0].icon_path, None,
            "a declared icon the archive does not hold is never recorded"
        );
        let history = fixture
            .library
            .histories("com.example.zipped")
            .expect("read the history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].format, "zip");
        assert_eq!(history[0].origin, archive.display().to_string());
        fixture.assert_no_staging();
    }

    #[test]
    fn an_unsupported_source_names_the_path_and_the_supported_formats() {
        let fixture = Fixture::new();

        for name in ["Example.dmg", "Example.pkg", "Example.xip", "Example"] {
            let source = fixture.directory.path().join(name);
            write_file(&source, b"not a bundle");

            let error = install(&request(&fixture, &source, "installation-1"))
                .expect_err("this cut imports .app and .zip only");
            let message = error.to_string();

            assert!(
                matches!(error, Error::UnsupportedFormat { .. }),
                "unexpected error: {error:?}"
            );
            assert!(
                message.contains(name),
                "the message must name the path: {message}"
            );
            assert!(
                message.contains(".app") && message.contains(".zip"),
                "the message must list the supported formats: {message}"
            );
        }

        assert!(
            fixture.attempts().is_empty(),
            "a source with no supported format is not an installation attempt"
        );
        fixture.assert_no_staging();
    }

    #[test]
    fn an_absolute_zip_entry_is_rejected() {
        let fixture = Fixture::new();
        let archive = fixture.directory.path().join("Example.zip");
        let mut entries = zip_bundle("Example.app", "Example", &example_plist());
        entries.push(entry("/escaped.txt", b"escaped"));
        write_zip(&archive, &entries);

        let error = install(&request(&fixture, &archive, "installation-1"))
            .expect_err("an absolute entry would land outside the staging area");

        assert!(
            matches!(error, Error::ArchiveEntryAbsolute { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            error.to_string().contains("/escaped.txt"),
            "the message must name the entry: {error}"
        );
        assert_nothing_installed(&fixture);
        assert_failed_attempt(&fixture, "installation-1", "zip", "/escaped.txt");
    }

    #[test]
    fn a_zip_entry_that_climbs_out_of_the_staging_area_is_rejected() {
        let fixture = Fixture::new();
        let archive = fixture.directory.path().join("Example.zip");
        let mut entries = zip_bundle("Example.app", "Example", &example_plist());
        entries.push(entry("../escaped.txt", b"escaped"));
        write_zip(&archive, &entries);

        let error = install(&request(&fixture, &archive, "installation-1"))
            .expect_err("a '..' entry walks out of the staging area");

        assert!(
            matches!(error, Error::ArchiveEntryTraversal { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            error.to_string().contains("../escaped.txt"),
            "the message must name the entry: {error}"
        );
        assert!(
            !fixture.staging().join("escaped.txt").exists(),
            "the entry is not written beside the staging directory"
        );
        assert_nothing_installed(&fixture);
        assert_failed_attempt(&fixture, "installation-1", "zip", "../escaped.txt");
    }

    #[test]
    fn a_zip_symlink_entry_is_rejected() {
        let fixture = Fixture::new();
        let archive = fixture.directory.path().join("Example.zip");
        write_zip_with_symlink(
            &archive,
            &zip_bundle("Example.app", "Example", &example_plist()),
            ("Example.app/Contents/link", "/etc/passwd"),
        );

        let error = install(&request(&fixture, &archive, "installation-1"))
            .expect_err("an archive installs no links");

        assert!(
            matches!(error, Error::ArchiveEntrySymlink { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            error.to_string().contains("Example.app/Contents/link"),
            "the message must name the entry: {error}"
        );
        assert_nothing_installed(&fixture);
        assert_failed_attempt(
            &fixture,
            "installation-1",
            "zip",
            "Example.app/Contents/link",
        );
    }

    #[test]
    fn a_zip_without_a_bundle_is_rejected() {
        let fixture = Fixture::new();
        let archive = fixture.directory.path().join("Empty.zip");
        write_zip(&archive, &[entry("readme.txt", b"nothing to install")]);

        let error = install(&request(&fixture, &archive, "installation-1"))
            .expect_err("a source without a bundle installs nothing");

        assert!(
            matches!(error, Error::NoBundleInStaging { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            error.to_string().contains(".app"),
            "the message must say what was missing: {error}"
        );
        assert_nothing_installed(&fixture);
        assert_failed_attempt(&fixture, "installation-1", "zip", ".app");
    }

    #[test]
    fn a_zip_with_two_bundles_is_rejected() {
        let fixture = Fixture::new();
        let archive = fixture.directory.path().join("Two.zip");
        let mut entries = zip_bundle("Example.app", "Example", &example_plist());
        entries.extend(zip_bundle(
            "Other.app",
            "Other",
            &plist(
                Some("com.example.other"),
                "Other",
                Some("1.0"),
                "Other",
                None,
            ),
        ));
        write_zip(&archive, &entries);

        let error = install(&request(&fixture, &archive, "installation-1"))
            .expect_err("two bundles are not one application");
        let message = error.to_string();

        assert!(
            matches!(error, Error::SeveralBundlesInStaging { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            message.contains("Example.app") && message.contains("Other.app"),
            "the message must name both candidates: {message}"
        );
        assert_nothing_installed(&fixture);
        assert_failed_attempt(&fixture, "installation-1", "zip", "Other.app");
    }

    #[test]
    fn an_existing_destination_is_refused_without_replace() {
        let fixture = Fixture::new();
        let source = example_bundle(fixture.directory.path());
        let installed = fixture.applications.join("Example.app");
        let marker = installed.join("Contents").join("keep.txt");
        write_file(&marker, b"already installed");

        let error = install(&request(&fixture, &source, "installation-1"))
            .expect_err("an installed application is not overwritten silently");

        assert!(
            matches!(error, Error::DestinationExists { .. }),
            "unexpected error: {error:?}"
        );
        let message = error.to_string();
        assert!(message.contains("Example.app"), "message: {message}");
        assert!(message.contains("replace"), "message: {message}");
        assert!(
            marker.is_file(),
            "the installed copy is left exactly as it was"
        );
        assert!(
            !installed.join("Contents").join("Info.plist").exists(),
            "nothing of the refused bundle is written over the installed one"
        );
        assert!(
            fixture.applications_rows().is_empty(),
            "a refused import records no application"
        );
        fixture.assert_no_staging();
        assert_failed_attempt(&fixture, "installation-1", "app", "Example.app");
    }

    #[test]
    fn a_replacement_swaps_the_bundle_and_appends_to_the_history() {
        let fixture = Fixture::new();
        let first = write_bundle(
            fixture.directory.path(),
            "Example.app",
            "Example",
            &plist(Some(EXAMPLE), "Example", Some("1.2.3"), "Example", None),
        );
        write_file(
            &first.join("Contents").join("Resources").join("old.txt"),
            b"old",
        );
        install(&request(&fixture, &first, "installation-1"))
            .expect("the first version is installed");

        let second_parent = fixture.directory.path().join("second");
        create_dir(&second_parent);
        let second = write_bundle(
            &second_parent,
            "Example.app",
            "Example",
            &plist(Some(EXAMPLE), "Example", Some("2.0.0"), "Example", None),
        );
        write_file(
            &second.join("Contents").join("Resources").join("new.txt"),
            b"new",
        );
        let mut replacement = request(&fixture, &second, "installation-2");
        replacement.replace = true;
        replacement.now = LATER;

        let installed = install(&replacement).expect("the replacement is installed");

        let destination = fixture.applications.join("Example.app");
        assert_eq!(installed.path, destination);
        assert!(
            destination
                .join("Contents")
                .join("Resources")
                .join("new.txt")
                .is_file(),
            "the new copy is in place"
        );
        assert!(
            !destination
                .join("Contents")
                .join("Resources")
                .join("old.txt")
                .exists(),
            "the replaced copy is deleted, not merged into the new one"
        );

        let rows = fixture.applications_rows();
        assert_eq!(
            rows.len(),
            1,
            "a replacement updates the application row instead of adding one"
        );
        assert_eq!(rows[0].id, EXAMPLE);
        assert_eq!(rows[0].version.as_deref(), Some("2.0.0"));
        assert_eq!(rows[0].installed_at, LATER);

        let history = fixture
            .library
            .histories(EXAMPLE)
            .expect("read the history");
        assert_eq!(history.len(), 2, "both attempts stay in the history");
        assert_eq!(history[0].id, "installation-1");
        assert_eq!(history[1].id, "installation-2");
        assert_eq!(history[1].occurred_at, LATER);
        fixture.assert_no_staging();
    }

    #[test]
    fn an_inspection_failure_leaves_nothing_behind_and_records_the_attempt() {
        let fixture = Fixture::new();
        let broken = write_bundle(
            fixture.directory.path(),
            "Broken.app",
            "Broken",
            &plist(None, "Broken", Some("1.0"), "Broken", None),
        );

        let error = install(&request(&fixture, &broken, "installation-1"))
            .expect_err("a bundle without an identifier cannot be trusted");

        assert!(
            matches!(error, Error::Inspection { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            error.to_string().contains("CFBundleIdentifier"),
            "the message must carry the inspector's reason: {error}"
        );
        assert!(
            !fixture.applications.join("Broken.app").exists(),
            "no bundle is left behind"
        );
        assert_nothing_installed(&fixture);
        assert_failed_attempt(&fixture, "installation-1", "app", "CFBundleIdentifier");
    }

    #[cfg(unix)]
    #[test]
    fn an_escaping_symlink_in_a_source_bundle_is_rejected() {
        let fixture = Fixture::new();
        let bundle = write_bundle(
            fixture.directory.path(),
            "Linked.app",
            "Linked",
            &plist(
                Some("com.example.linked"),
                "Linked",
                Some("1.0"),
                "Linked",
                None,
            ),
        );
        let outside = fixture.directory.path().join("outside.txt");
        write_file(&outside, b"outside the bundle");
        std::os::unix::fs::symlink(&outside, bundle.join("Contents").join("escape.txt"))
            .expect("create the escaping symlink");

        let error = install(&request(&fixture, &bundle, "installation-1"))
            .expect_err("a link may not reach outside the bundle");
        let message = error.to_string();

        assert!(
            matches!(error, Error::SymlinkEscapes { .. }),
            "unexpected error: {error:?}"
        );
        assert!(message.contains("escape.txt"), "message: {message}");
        assert!(message.contains("outside.txt"), "message: {message}");
        assert_nothing_installed(&fixture);
        assert_failed_attempt(&fixture, "installation-1", "app", "escape.txt");
    }

    #[test]
    fn the_architectures_of_the_executable_are_joined_with_a_comma() {
        let fixture = Fixture::new();
        let bundle = fixture.directory.path().join("Universal.app");
        write_file(
            &bundle.join("Contents").join("Info.plist"),
            plist(
                Some("com.example.universal"),
                "Universal",
                None,
                "Universal",
                None,
            )
            .as_bytes(),
        );
        write_file(
            &bundle.join("Contents").join("MacOS").join("Universal"),
            &fat_macho(&[CPU_TYPE_X86_64, CPU_TYPE_ARM64]),
        );

        install(&request(&fixture, &bundle, "installation-1"))
            .expect("a universal bundle is installed");

        assert_eq!(
            fixture.applications_rows()[0].architecture.as_deref(),
            Some("x86_64,arm64")
        );
    }

    #[test]
    fn an_unusable_installation_id_is_refused_before_anything_is_staged() {
        let fixture = Fixture::new();
        let source = example_bundle(fixture.directory.path());

        let error = install(&request(&fixture, &source, "../escape"))
            .expect_err("an installation id names one staging directory");

        assert!(
            matches!(error, Error::InvalidInstallationId { .. }),
            "unexpected error: {error:?}"
        );
        assert!(error.to_string().contains("../escape"), "{error}");
        assert!(
            !fixture.directory.path().join("escape").exists(),
            "nothing is created outside the staging area"
        );
        assert_nothing_installed(&fixture);
        assert_failed_attempt(&fixture, "../escape", "app", "installation id");
    }

    #[test]
    fn the_destination_name_is_sanitized_and_never_leaves_the_directory() {
        assert_eq!(
            destination_name("Example").expect("a plain name"),
            "Example"
        );
        assert_eq!(
            destination_name("Example.app").expect("a trailing bundle suffix is stripped"),
            "Example"
        );
        assert_eq!(
            destination_name("  Spaced  ").expect("the name is trimmed"),
            "Spaced"
        );
        assert_eq!(
            destination_name("a/b\\c").expect("separators become dashes"),
            "a-b-c"
        );
        for declared in ["", "   ", ".", "..", ".app", "bad\0name"] {
            let error = destination_name(declared)
                .expect_err("a name that is not one path component is refused");
            assert!(
                matches!(error, Error::InvalidBundleName { .. }),
                "unexpected error for '{declared}': {error:?}"
            );
        }
    }

    #[test]
    fn a_thin_executable_is_copied_byte_for_byte() {
        let fixture = Fixture::new();
        let source = example_bundle(fixture.directory.path());

        install(&request(&fixture, &source, "installation-1")).expect("the bundle is installed");

        assert_eq!(
            std::fs::read(
                fixture
                    .applications
                    .join("Example.app")
                    .join("Contents")
                    .join("MacOS")
                    .join("Example")
            )
            .expect("read the installed executable"),
            thin_macho(CPU_TYPE_X86_64)
        );
    }

    #[test]
    fn a_source_named_like_a_bundle_but_not_a_directory_is_rejected() {
        let fixture = Fixture::new();
        let source = fixture.directory.path().join("Example.app");
        write_file(&source, b"not a bundle");

        let error = install(&request(&fixture, &source, "installation-1"))
            .expect_err("an extracted bundle is a directory");

        assert!(
            matches!(error, Error::BundleNotDirectory { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            error.to_string().contains("Example.app"),
            "the message must name the path: {error}"
        );
        assert_nothing_installed(&fixture);
        assert_failed_attempt(&fixture, "installation-1", "app", "not a directory");
    }

    #[test]
    fn a_row_the_catalogue_refuses_leaves_no_bundle_installed() {
        let fixture = Fixture::new();
        install(&request(
            &fixture,
            &example_bundle(fixture.directory.path()),
            "installation-1",
        ))
        .expect("the first version is installed");

        // A second source of the same application under another name has a free
        // destination, so only the catalogue can refuse it: the row id is the
        // bundle identifier, and the first install already holds it.
        let other_parent = fixture.directory.path().join("other");
        create_dir(&other_parent);
        let second = write_bundle(
            &other_parent,
            "Second.app",
            "Example",
            &plist(Some(EXAMPLE), "Second", Some("1.2.3"), "Example", None),
        );

        let error = install(&request(&fixture, &second, "installation-2"))
            .expect_err("the catalogue holds this application already");

        assert!(
            matches!(error, Error::Library { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            !fixture.applications.join("Second.app").exists(),
            "a bundle the catalogue refused is removed again"
        );
        let rows = fixture.applications_rows();
        assert_eq!(rows.len(), 1, "the installed row is left untouched");
        assert_eq!(
            rows[0].bundle_path,
            fixture.applications.join("Example.app")
        );
        assert_eq!(
            fixture.attempts().len(),
            2,
            "the refused attempt is recorded as failed"
        );
        fixture.assert_no_staging();
    }

    #[test]
    fn a_reused_installation_id_leaves_no_application_row_of_the_second_bundle() {
        let fixture = Fixture::new();
        install(&request(
            &fixture,
            &example_bundle(fixture.directory.path()),
            "installation-1",
        ))
        .expect("the first bundle is installed");

        // A different application installed with the first attempt's id: the
        // history already holds that id, so the second attempt cannot be
        // recorded, and the catalogue must not keep the application row of the
        // bundle the install then removes again.
        let other_parent = fixture.directory.path().join("other");
        create_dir(&other_parent);
        let second = write_bundle(
            &other_parent,
            "Other.app",
            "Other",
            &plist(
                Some("com.example.other"),
                "Other",
                Some("1.0"),
                "Other",
                None,
            ),
        );

        let error = install(&request(&fixture, &second, "installation-1"))
            .expect_err("the history already holds this installation id");

        assert!(
            error.to_string().contains("installation-1"),
            "the error must name the reused id: {error}"
        );
        let rows = fixture.applications_rows();
        assert_eq!(
            rows.len(),
            1,
            "the catalogue keeps no application of the attempt it could not record: {rows:?}"
        );
        assert_eq!(rows[0].id, EXAMPLE);
        assert!(
            !fixture.applications.join("Other.app").exists(),
            "the bundle of the failed attempt is removed again"
        );
        for row in &rows {
            assert!(
                row.bundle_path.exists(),
                "the catalogue points at '{}', which is not installed",
                row.bundle_path.display()
            );
        }
        fixture.assert_no_staging();
    }

    #[test]
    fn a_failure_that_cannot_be_recorded_keeps_the_original_error() {
        let fixture = Fixture::new();
        let source = example_bundle(fixture.directory.path());
        install(&request(&fixture, &source, "installation-1"))
            .expect("the first bundle is installed");

        // The same source installed again without the replace policy fails on the
        // existing destination, and it reuses the first attempt's id, so the
        // failed row cannot be written either.
        let error = install(&request(&fixture, &source, "installation-1"))
            .expect_err("an installed application is not overwritten silently");

        match &error {
            Error::FailureNotRecorded { install, record } => {
                assert!(
                    matches!(install.as_ref(), Error::DestinationExists { .. }),
                    "the original install failure is kept: {install:?}"
                );
                assert!(
                    record.to_string().contains("installation-1"),
                    "the record failure names the reused id: {record}"
                );
            }
            other => panic!("expected a failure that could not be recorded, got {other:?}"),
        }
        let message = error.to_string();
        assert!(
            message.contains("Example.app"),
            "the message keeps the original failure: {message}"
        );
        assert!(
            message.contains("could not be recorded"),
            "the message states that the attempt was not recorded: {message}"
        );
        assert_eq!(
            std::error::Error::source(&error)
                .expect("the original failure stays reachable")
                .to_string(),
            match &error {
                Error::FailureNotRecorded { install, .. } => install.to_string(),
                other => panic!("expected a failure that could not be recorded, got {other:?}"),
            }
        );
        assert_eq!(
            fixture.attempts().len(),
            1,
            "the unrecorded attempt leaves no second history row"
        );
        fixture.assert_no_staging();
    }

    #[test]
    fn declares_its_responsibility() {
        assert!(!env!("CARGO_PKG_DESCRIPTION").trim().is_empty());
    }
}
