//! Typed errors of the Application Library.
//!
//! Every failure is a variant of [`Error`], so callers can branch on the cause
//! instead of matching messages. The messages are written for a human operator
//! and name the file, the table or the value that caused the failure: a
//! catalogue that cannot store a row must say what is in the way and what to do
//! about it.

use std::fmt;
use std::path::PathBuf;

/// Convenient result alias for the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Every failure the Application Library can report.
#[derive(Debug)]
pub enum Error {
    /// The catalogue file or one of its directories cannot be created.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// A SQL statement failed for a reason the library cannot classify.
    Database {
        path: PathBuf,
        operation: &'static str,
        source: rusqlite::Error,
    },
    /// One schema step failed. It ran inside a transaction, so the previous
    /// schema and its rows are still intact.
    MigrationFailed {
        path: PathBuf,
        version: i64,
        source: rusqlite::Error,
    },
    /// The catalogue was written by a newer build, so this one cannot know what
    /// its rows mean.
    SchemaTooNew {
        path: PathBuf,
        found: i64,
        supported: i64,
    },
    /// A table already holds a row with this primary key.
    DuplicateId { table: &'static str, id: String },
    /// An application with this bundle id is already registered at this path.
    BundlePathTaken {
        bundle_id: String,
        bundle_path: PathBuf,
    },
    /// An installation names an application the catalogue does not hold.
    UnregisteredApplication { application_id: String },
    /// A path cannot be stored, because the columns are UTF-8 text.
    NonUtf8Path { field: &'static str, path: PathBuf },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(
                f,
                "I/O error on the application library '{}': {source}; the catalogue lives in a \
                 writable directory outside the DPREFIX",
                path.display()
            ),
            Self::Database {
                path,
                operation,
                source,
            } => write!(
                f,
                "cannot {operation} in the application library '{}': {source}",
                path.display()
            ),
            Self::MigrationFailed {
                path,
                version,
                source,
            } => write!(
                f,
                "schema migration {version} of the application library '{}' failed: {source}; the \
                 migration runs in a transaction, so the previous schema and its rows are intact \
                 and the catalogue can be opened again once the cause is fixed",
                path.display()
            ),
            Self::SchemaTooNew {
                path,
                found,
                supported,
            } => write!(
                f,
                "the application library '{}' uses schema version {found}, but this build only \
                 understands version {supported}; upgrade Darling Platform instead of opening it \
                 with an older build, or point the library at a different file",
                path.display()
            ),
            Self::DuplicateId { table, id } => write!(
                f,
                "the '{table}' table already holds a row with the id '{id}'; choose a different \
                 id, or query the existing row instead of inserting it again"
            ),
            Self::BundlePathTaken {
                bundle_id,
                bundle_path,
            } => write!(
                f,
                "the application '{}' at '{}' is already registered; only one row may hold the \
                 pair of bundle id and bundle path, so remove the existing entry or install the \
                 bundle at a different path",
                bundle_id,
                bundle_path.display()
            ),
            Self::UnregisteredApplication { application_id } => write!(
                f,
                "the installation refers to the application '{application_id}', which the \
                 application library does not hold; record the application before its \
                 installation history",
            ),
            Self::NonUtf8Path { field, path } => write!(
                f,
                "the {field} '{}' is not valid UTF-8, so it cannot be stored in the text \
                 catalogue; install the bundle under a path that is valid UTF-8",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Database { source, .. } => Some(source),
            Self::MigrationFailed { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundle_path_conflict_names_the_bundle_and_the_path() {
        let error = Error::BundlePathTaken {
            bundle_id: String::from("com.example.app"),
            bundle_path: PathBuf::from("/Applications/Example.app"),
        };
        let message = error.to_string();

        assert!(message.contains("com.example.app"), "message: {message}");
        assert!(
            message.contains("/Applications/Example.app"),
            "message: {message}"
        );
    }

    #[test]
    fn a_newer_schema_names_both_versions_and_the_file() {
        let error = Error::SchemaTooNew {
            path: PathBuf::from("/home/user/.local/share/darling-platform/library.sqlite3"),
            found: 7,
            supported: 1,
        };
        let message = error.to_string();

        assert!(message.contains("version 7"), "message: {message}");
        assert!(message.contains("version 1"), "message: {message}");
        assert!(
            message.contains("darling-platform/library.sqlite3"),
            "message: {message}"
        );
    }

    #[test]
    fn an_unregistered_application_names_the_id() {
        let error = Error::UnregisteredApplication {
            application_id: String::from("app-1"),
        };
        let message = error.to_string();

        assert!(message.contains("'app-1'"), "message: {message}");
    }

    #[test]
    fn source_is_exposed_only_for_wrapped_failures() {
        let io = Error::Io {
            path: PathBuf::from("/home/user/.local/share/darling-platform/library.sqlite3"),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        };
        assert!(std::error::Error::source(&io).is_some());

        let duplicate = Error::DuplicateId {
            table: "applications",
            id: String::from("app-1"),
        };
        assert!(std::error::Error::source(&duplicate).is_none());
    }
}
