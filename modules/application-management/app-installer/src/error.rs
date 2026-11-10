//! Typed errors of the Darling App Installer.
//!
//! Every failure is a variant of [`Error`], so callers can branch on the cause
//! instead of matching messages. The messages are written for a human operator
//! and name the file, the archive entry or the declared value that caused the
//! failure together with what to do about it: an import that is refused must say
//! which entry was unsafe, and an import that cannot be placed must say what is
//! in the way.

use std::fmt;
use std::path::PathBuf;

/// Convenient result alias for the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Every failure the installer can report.
#[derive(Debug)]
pub enum Error {
    /// A file system operation failed. Staging, copying, renaming or cleanup.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The source is neither an extracted `.app` directory nor a `.zip`
    /// archive, so this cut cannot import it.
    UnsupportedFormat { path: PathBuf, found: &'static str },
    /// The caller's installation id cannot name a staging directory.
    InvalidInstallationId { value: String },
    /// The bundle's name cannot name a destination directory.
    InvalidBundleName { declared: String },
    /// A source that is named like a bundle is not a directory.
    BundleNotDirectory { path: PathBuf },
    /// An application is already installed at the destination and the request
    /// did not ask to replace it.
    DestinationExists { path: PathBuf },
    /// The staged import holds no `.app` bundle.
    NoBundleInStaging { staging: PathBuf },
    /// The staged import holds more than one `.app` bundle, so it is not one
    /// application.
    SeveralBundlesInStaging {
        staging: PathBuf,
        found: Vec<String>,
    },
    /// The bundle inspector refused the staged bundle.
    Inspection {
        bundle: PathBuf,
        source: Box<dp_bundle_inspector::Error>,
    },
    /// A symbolic link inside the source bundle resolves outside it.
    SymlinkEscapes {
        bundle: PathBuf,
        path: PathBuf,
        target: PathBuf,
    },
    /// The source bundle holds a symbolic link and this host cannot recreate it.
    SymlinkUnsupported { path: PathBuf },
    /// A bundle entry is neither a regular file, a directory nor a symbolic
    /// link, so it cannot be copied.
    UnsupportedEntry { path: PathBuf },
    /// The archive cannot be opened or one of its entries cannot be read.
    ArchiveUnreadable {
        archive: PathBuf,
        source: zip::result::ZipError,
    },
    /// The archive declares more entries than the installer accepts.
    ArchiveTooManyEntries {
        archive: PathBuf,
        found: usize,
        limit: usize,
    },
    /// The archive would write more bytes than the installer accepts.
    ArchiveTooLarge {
        archive: PathBuf,
        found: u64,
        limit: u64,
    },
    /// An archive entry names a path the extraction cannot contain.
    ArchiveEntryAbsolute { archive: PathBuf, entry: String },
    /// An archive entry climbs out of the staging area with `..`.
    ArchiveEntryTraversal { archive: PathBuf, entry: String },
    /// An archive entry name holds a NUL byte, which truncates a path.
    ArchiveEntryNul { archive: PathBuf, entry: String },
    /// An archive entry name is not a plain path inside the staging area.
    ArchiveEntryUnsafe { archive: PathBuf, entry: String },
    /// An archive entry is a symbolic link, which the extraction never creates.
    ArchiveEntrySymlink { archive: PathBuf, entry: String },
    /// The installed copy could not be moved aside and the previous
    /// installation could not be put back.
    RestoreFailed {
        destination: PathBuf,
        backup: PathBuf,
        source: std::io::Error,
    },
    /// The application library refused a row, so the import is not recorded.
    Library {
        operation: &'static str,
        source: Box<dp_app_library::Error>,
    },
    /// The attempt failed and its `failed` history row could not be written, so
    /// the catalogue holds no trace of it.
    ///
    /// `install` is the failure that stopped the attempt, which is what the
    /// operator has to act on, and `record` is why the attempt could not even be
    /// recorded. The installation id is the value that usually collides: it must
    /// be unique per attempt.
    FailureNotRecorded {
        install: Box<Error>,
        record: Box<Error>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "I/O error on '{}': {source}", path.display()),
            Self::UnsupportedFormat { path, found } => write!(
                f,
                "'{}' is {found}, which this installer cannot import; pass an extracted .app \
                 directory or a .zip archive holding one (DMG, PKG and .xip are not supported in \
                 this cut)",
                path.display()
            ),
            Self::InvalidInstallationId { value } => write!(
                f,
                "the installation id '{value}' cannot name a staging directory; pass a single path \
                 component without separators, such as 'installation-1'"
            ),
            Self::InvalidBundleName { declared } => write!(
                f,
                "the bundle declares the name '{declared}', which cannot name a destination \
                 directory; declare a CFBundleName that is not empty, not '.' and not '..'"
            ),
            Self::BundleNotDirectory { path } => write!(
                f,
                "'{}' is not a directory; an extracted .app bundle is a directory holding \
                 Contents/Info.plist, so pass the directory or a .zip archive holding it",
                path.display()
            ),
            Self::DestinationExists { path } => write!(
                f,
                "'{}' already holds an installed application; uninstall it first, or install with \
                 the replace policy so the installed copy is swapped for the new one",
                path.display()
            ),
            Self::NoBundleInStaging { staging } => write!(
                f,
                "the source holds no .app bundle: nothing in '{}' ends with .app; pass an extracted \
                 .app directory or a .zip archive holding one",
                staging.display()
            ),
            Self::SeveralBundlesInStaging { staging, found } => write!(
                f,
                "the source holds several .app bundles, so it is not one application: {} in '{}'; \
                 pass one bundle at a time",
                found.join(", "),
                staging.display()
            ),
            Self::Inspection { bundle, source } => write!(
                f,
                "the staged bundle '{}' cannot be installed: {source}",
                bundle.display()
            ),
            Self::SymlinkEscapes {
                bundle,
                path,
                target,
            } => write!(
                f,
                "the bundle '{}' holds the symbolic link '{}', which resolves to '{}' outside the \
                 bundle; remove the link or point it inside the bundle",
                bundle.display(),
                path.display(),
                target.display()
            ),
            Self::SymlinkUnsupported { path } => write!(
                f,
                "the bundle holds the symbolic link '{}', which this host cannot recreate; install \
                 the bundle on a filesystem that supports symbolic links",
                path.display()
            ),
            Self::UnsupportedEntry { path } => write!(
                f,
                "'{}' is neither a regular file, a directory nor a symbolic link, so it cannot be \
                 part of an installed bundle; remove it from the bundle",
                path.display()
            ),
            Self::ArchiveUnreadable { archive, source } => write!(
                f,
                "cannot read the archive '{}': {source}; the file must be a ZIP archive using the \
                 stored or deflate compression method",
                archive.display()
            ),
            Self::ArchiveTooManyEntries {
                archive,
                found,
                limit,
            } => write!(
                f,
                "the archive '{}' declares {found} entries, but at most {limit} are accepted; pass \
                 the extracted .app bundle instead",
                archive.display()
            ),
            Self::ArchiveTooLarge {
                archive,
                found,
                limit,
            } => write!(
                f,
                "the archive '{}' would write {found} bytes, but at most {limit} are accepted; \
                 pass the extracted .app bundle instead",
                archive.display()
            ),
            Self::ArchiveEntryAbsolute { archive, entry } => write!(
                f,
                "the archive '{}' holds the entry '{entry}', which is an absolute path and would be \
                 written outside the staging area; rebuild the archive with bundle-relative paths",
                archive.display()
            ),
            Self::ArchiveEntryTraversal { archive, entry } => write!(
                f,
                "the archive '{}' holds the entry '{entry}', which climbs out of the staging area \
                 with '..'; rebuild the archive with bundle-relative paths",
                archive.display()
            ),
            Self::ArchiveEntryNul { archive, entry } => write!(
                f,
                "the archive '{}' holds the entry name {entry:?}, which contains a NUL byte that \
                 would truncate the extracted path; rebuild the archive without it",
                archive.display()
            ),
            Self::ArchiveEntryUnsafe { archive, entry } => write!(
                f,
                "the archive '{}' holds the entry '{entry}', which is not a plain path inside the \
                 staging area; rebuild the archive with bundle-relative paths",
                archive.display()
            ),
            Self::ArchiveEntrySymlink { archive, entry } => write!(
                f,
                "the archive '{}' holds the entry '{entry}', which is a symbolic link; archives \
                 install no links, because a link can point at any file on this host, so pass the \
                 extracted .app bundle instead",
                archive.display()
            ),
            Self::RestoreFailed {
                destination,
                backup,
                source,
            } => write!(
                f,
                "cannot put the previous installation back at '{}': {source}; its copy is still at \
                 '{}', so move it into place by hand",
                destination.display(),
                backup.display()
            ),
            Self::Library { operation, source } => {
                write!(f, "cannot {operation} in the application library: {source}")
            }
            Self::FailureNotRecorded { install, record } => write!(
                f,
                "{install}; the failed attempt could not be recorded in the application library: \
                 {record}"
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Inspection { source, .. } => Some(source.as_ref()),
            Self::ArchiveUnreadable { source, .. } => Some(source),
            Self::RestoreFailed { source, .. } => Some(source),
            Self::Library { source, .. } => Some(source.as_ref()),
            Self::FailureNotRecorded { install, .. } => Some(install.as_ref()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unsupported_format_names_the_path_and_the_supported_ones() {
        let error = Error::UnsupportedFormat {
            path: PathBuf::from("/workspace/Example.dmg"),
            found: "a .dmg disk image",
        };
        let message = error.to_string();

        assert!(
            message.contains("/workspace/Example.dmg"),
            "message: {message}"
        );
        assert!(message.contains(".app"), "message: {message}");
        assert!(message.contains(".zip"), "message: {message}");
        assert!(message.contains("not supported"), "message: {message}");
    }

    #[test]
    fn an_unsafe_archive_entry_names_the_archive_and_the_entry() {
        for error in [
            Error::ArchiveEntryTraversal {
                archive: PathBuf::from("/workspace/Example.zip"),
                entry: String::from("../escaped.txt"),
            },
            Error::ArchiveEntryAbsolute {
                archive: PathBuf::from("/workspace/Example.zip"),
                entry: String::from("/tmp/escaped.txt"),
            },
            Error::ArchiveEntrySymlink {
                archive: PathBuf::from("/workspace/Example.zip"),
                entry: String::from("Example.app/link"),
            },
            Error::ArchiveEntryUnsafe {
                archive: PathBuf::from("/workspace/Example.zip"),
                entry: String::from("Example.app/../../escaped"),
            },
        ] {
            let message = error.to_string();

            assert!(
                message.contains("/workspace/Example.zip"),
                "the message must name the archive: {message}"
            );
            assert!(
                message.contains("Example.app") || message.contains("escaped.txt"),
                "the message must name the entry: {message}"
            );
        }
    }

    #[test]
    fn the_archive_limits_are_reported_with_the_values_they_compare() {
        let entries = Error::ArchiveTooManyEntries {
            archive: PathBuf::from("/workspace/Example.zip"),
            found: 200_001,
            limit: 200_000,
        };
        let message = entries.to_string();
        assert!(
            message.contains("200001") || message.contains("200_001"),
            "message: {message}"
        );
        assert!(
            message.contains("200000") || message.contains("200_000"),
            "message: {message}"
        );

        let bytes = Error::ArchiveTooLarge {
            archive: PathBuf::from("/workspace/Example.zip"),
            found: 9_000_000_000,
            limit: 8_589_934_592,
        };
        let message = bytes.to_string();
        assert!(message.contains("8589934592"), "message: {message}");
    }

    #[test]
    fn a_staging_area_without_a_bundle_says_what_was_staged() {
        let error = Error::NoBundleInStaging {
            staging: PathBuf::from("/Applications/.staging/installation-1"),
        };
        let message = error.to_string();

        assert!(message.contains("installation-1"), "message: {message}");
        assert!(message.contains(".app"), "message: {message}");
    }

    #[test]
    fn a_failure_that_could_not_be_recorded_names_both_causes() {
        let install = Error::DestinationExists {
            path: PathBuf::from("/Applications/Example.app"),
        };
        let expected_install = install.to_string();
        let error = Error::FailureNotRecorded {
            install: Box::new(install),
            record: Box::new(Error::Library {
                operation: "recording a failed installation",
                source: Box::new(dp_app_library::Error::DuplicateId {
                    table: "installations",
                    id: String::from("installation-1"),
                }),
            }),
        };

        let message = error.to_string();
        assert!(
            message.starts_with(&expected_install),
            "the original failure comes first: {message}"
        );
        assert!(
            message.contains("could not be recorded"),
            "the message says the attempt was not recorded: {message}"
        );
        assert!(
            message.contains("installation-1"),
            "the message names why it could not be recorded: {message}"
        );
        assert_eq!(
            std::error::Error::source(&error)
                .expect("the original failure stays reachable")
                .to_string(),
            expected_install,
            "source() is the failure that stopped the attempt"
        );
    }

    #[test]
    fn source_is_exposed_only_for_wrapped_failures() {
        let io = Error::Io {
            path: PathBuf::from("/Applications/.staging/installation-1"),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        };
        assert!(std::error::Error::source(&io).is_some());

        let destination = Error::DestinationExists {
            path: PathBuf::from("/Applications/Example.app"),
        };
        assert!(std::error::Error::source(&destination).is_none());
    }
}
