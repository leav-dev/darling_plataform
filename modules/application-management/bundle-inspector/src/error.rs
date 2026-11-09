//! Typed errors of the Bundle Inspector.
//!
//! Every failure is a variant of [`Error`], so callers can branch on the cause
//! instead of matching messages. The messages are written for a human operator
//! and always name the file or the declared value that caused the failure: a
//! bundle that cannot be inspected must say which declaration is unacceptable
//! and what to change.

use std::fmt;
use std::path::PathBuf;

/// Convenient result alias for the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Every failure the Bundle Inspector can report.
#[derive(Debug)]
pub enum Error {
    /// The path handed to [`crate::inspect`] is not a directory.
    BundleNotDirectory { path: PathBuf },
    /// The bundle has no `Contents/Info.plist`, so it is not a macOS bundle.
    InfoPlistMissing { bundle: PathBuf },
    /// `Contents/Info.plist` exists but cannot be read or parsed.
    InfoPlistUnreadable { path: PathBuf, source: plist::Error },
    /// The property list root is not a dictionary.
    InfoPlistNotDictionary { path: PathBuf, found: &'static str },
    /// A key the bundle must declare is absent.
    MissingKey { path: PathBuf, key: &'static str },
    /// A declared key is present but unusable: empty, or not a string.
    InvalidKey {
        path: PathBuf,
        key: &'static str,
        reason: &'static str,
    },
    /// A declared path is absolute, so joining it would leave the bundle.
    AbsolutePath {
        bundle: PathBuf,
        role: &'static str,
        value: String,
    },
    /// A declared path contains `..` and can walk out of the bundle.
    ParentTraversal {
        bundle: PathBuf,
        role: &'static str,
        value: String,
    },
    /// A path that lexically stays inside the bundle resolves outside it
    /// through a symlink.
    EscapingSymlink {
        bundle: PathBuf,
        role: &'static str,
        value: String,
        target: PathBuf,
    },
    /// The declared executable does not exist inside the bundle.
    ExecutableMissing {
        bundle: PathBuf,
        value: String,
        path: PathBuf,
    },
    /// The declared executable exists but is not a regular file.
    ExecutableNotRegularFile {
        bundle: PathBuf,
        value: String,
        path: PathBuf,
    },
    /// No `CFBundleName` was declared and the bundle directory has no usable
    /// name to fall back to.
    BundleWithoutName { path: PathBuf },
    /// An I/O operation on `path` failed.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The declared executable does not start with a Mach-O magic.
    NotMachO { path: PathBuf, magic: [u8; 4] },
    /// The file ends before the header it announces.
    MachOTruncated {
        path: PathBuf,
        needed: u64,
        found: u64,
    },
    /// The universal header is well-formed enough to read the architecture
    /// table but describes slices the file cannot hold.
    MachOInvalidFat { path: PathBuf, reason: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BundleNotDirectory { path } => write!(
                f,
                "'{}' is not a directory; point the inspector at an extracted .app bundle, which \
                 is a directory containing Contents/Info.plist",
                path.display()
            ),
            Self::InfoPlistMissing { bundle } => write!(
                f,
                "'{}' has no Contents/Info.plist; it is not a macOS application bundle, or the \
                 copy is incomplete",
                bundle.display()
            ),
            Self::InfoPlistUnreadable { path, source } => write!(
                f,
                "cannot read the bundle metadata '{}': {source}; the file must be an XML or a \
                 binary property list",
                path.display()
            ),
            Self::InfoPlistNotDictionary { path, found } => write!(
                f,
                "the bundle metadata '{}' holds {found}, but a bundle Info.plist must hold a \
                 dictionary of keys",
                path.display()
            ),
            Self::MissingKey { path, key } => write!(
                f,
                "the bundle metadata '{}' declares no '{key}', which every macOS application \
                 bundle must declare",
                path.display()
            ),
            Self::InvalidKey { path, key, reason } => write!(
                f,
                "the bundle metadata '{}' declares an unusable '{key}': {reason}",
                path.display()
            ),
            Self::AbsolutePath {
                bundle,
                role,
                value,
            } => write!(
                f,
                "'{role}' declares the absolute path '{value}', which is outside the bundle '{}'; \
                 declare a path relative to the bundle",
                bundle.display()
            ),
            Self::ParentTraversal {
                bundle,
                role,
                value,
            } => write!(
                f,
                "'{role}' declares '{value}', which climbs out of the bundle '{}' with '..'; \
                 declare a path that stays inside the bundle",
                bundle.display()
            ),
            Self::EscapingSymlink {
                bundle,
                role,
                value,
                target,
            } => write!(
                f,
                "'{role}' declares '{value}', which resolves to '{}' through a symlink and \
                 leaves the bundle '{}'; remove the symlink or point it inside the bundle",
                target.display(),
                bundle.display()
            ),
            Self::ExecutableMissing {
                bundle,
                value,
                path,
            } => write!(
                f,
                "the bundle '{}' declares '{value}' in CFBundleExecutable, but '{}' does not \
                 exist; the copy is incomplete",
                bundle.display(),
                path.display()
            ),
            Self::ExecutableNotRegularFile {
                bundle,
                value,
                path,
            } => write!(
                f,
                "the bundle '{}' declares '{value}' in CFBundleExecutable, but '{}' is not a \
                 regular file",
                bundle.display(),
                path.display()
            ),
            Self::BundleWithoutName { path } => write!(
                f,
                "the bundle '{}' declares no CFBundleName and its directory name cannot stand in \
                 for one; declare CFBundleName in Contents/Info.plist",
                path.display()
            ),
            Self::Io { path, source } => {
                write!(f, "I/O error on '{}': {source}", path.display())
            }
            Self::NotMachO { path, magic } => write!(
                f,
                "'{}' is not a Mach-O executable: its first four bytes are {}; the declared \
                 executable must be a macOS binary",
                path.display(),
                hex_bytes(magic)
            ),
            Self::MachOTruncated {
                path,
                needed,
                found,
            } => write!(
                f,
                "the Mach-O file '{}' ends after {found} bytes, but its header needs {needed}; \
                 the executable is truncated",
                path.display()
            ),
            Self::MachOInvalidFat { path, reason } => write!(
                f,
                "the universal header of '{}' is invalid: {reason}; the executable is corrupt",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InfoPlistUnreadable { source, .. } => Some(source),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Renders four magic bytes as the space-separated lowercase hex an operator can
/// compare against `xxd` output.
fn hex_bytes(bytes: &[u8; 4]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_observed_magic_is_rendered_as_hex() {
        assert_eq!(hex_bytes(b"\x7fELF"), "7f 45 4c 46");
        assert_eq!(hex_bytes(b"#!/b"), "23 21 2f 62");
    }

    #[test]
    fn a_missing_executable_message_names_the_value_and_the_path() {
        let error = Error::ExecutableMissing {
            bundle: PathBuf::from("/Applications/Example.app"),
            value: String::from("Example"),
            path: PathBuf::from("/Applications/Example.app/Contents/MacOS/Example"),
        };
        let message = error.to_string();

        assert!(message.contains("'Example'"), "message: {message}");
        assert!(
            message.contains("/Applications/Example.app/Contents/MacOS/Example"),
            "message: {message}"
        );
    }

    #[test]
    fn source_is_exposed_only_for_wrapped_failures() {
        let io = Error::Io {
            path: PathBuf::from("/Applications/Example.app/Contents/Info.plist"),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        };
        assert!(std::error::Error::source(&io).is_some());

        let not_mach_o = Error::NotMachO {
            path: PathBuf::from("/Applications/Example.app/Contents/MacOS/Example"),
            magic: [0x23, 0x21, 0x2f, 0x62],
        };
        assert!(std::error::Error::source(&not_mach_o).is_none());
    }
}
