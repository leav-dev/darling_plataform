//! Typed errors of the Compatibility Engine.
//!
//! Every failure is a variant of [`Error`], so callers branch on the cause
//! instead of matching messages. The messages are written for a human operator
//! and always name the file or the declared value that caused the failure: a
//! profile entry that cannot be used must say which entry, which value and what
//! to write instead.

use std::fmt;
use std::path::PathBuf;

/// Convenient result alias for the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Every failure the compatibility engine can report.
#[derive(Debug)]
pub enum Error {
    /// The capability profile is not valid TOML.
    ProfileSyntax {
        /// Parser message, which names the line and the token it rejected.
        reason: String,
    },
    /// A top-level profile field holds a value the engine cannot use.
    ProfileField {
        /// Field that failed validation, such as `macos_version`.
        field: &'static str,
        /// Value as the file declares it, so the report quotes what was read.
        value: String,
        /// What is unacceptable about it and what to write instead.
        reason: &'static str,
    },
    /// A profile table declares the same subject twice.
    ProfileDuplicate {
        /// Table that holds the repetition, such as `frameworks`.
        table: &'static str,
        /// Repeated name or path.
        value: String,
    },
    /// A profile entry declares a status outside the documented set.
    ProfileStatus {
        /// Table that holds the entry, such as `frameworks`.
        table: &'static str,
        /// Name or path of the entry.
        subject: String,
        /// Status value as the file declares it.
        value: String,
    },
    /// The bundle cannot be inspected, so there is nothing to analyse.
    Bundle {
        /// Bundle the caller asked about.
        path: PathBuf,
        /// Why the inspector rejected it, boxed so the crate's `Result` stays
        /// small on the success path.
        source: Box<dp_bundle_inspector::Error>,
    },
    /// The bundle's executable requirements cannot be read.
    Requirements {
        /// Executable the inspector accepted but could not read requirements
        /// from.
        path: PathBuf,
        /// Why the load commands were rejected, boxed like [`Error::Bundle`].
        source: Box<dp_bundle_inspector::Error>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProfileSyntax { reason } => write!(
                f,
                "the capability profile is not valid TOML: {reason}; fix the file, the engine \
                 never falls back to a built-in default"
            ),
            Self::ProfileField {
                field,
                value,
                reason,
            } => write!(
                f,
                "the capability profile declares {field} = '{value}', which {reason}; the engine \
                 rejects an unusable profile instead of assuming a value"
            ),
            Self::ProfileDuplicate { table, value } => write!(
                f,
                "the capability profile declares '{value}' twice in {table}; keep one entry per \
                 subject, because a duplicate makes the verdict depend on which copy wins"
            ),
            Self::ProfileStatus {
                table,
                subject,
                value,
            } => write!(
                f,
                "the capability profile declares the status '{value}' for {table} entry \
                 '{subject}', which is not one of supported, partial, unsupported, unknown"
            ),
            Self::Bundle { path, source } => {
                write!(f, "cannot analyse '{}': {source}", path.display())
            }
            Self::Requirements { path, source } => write!(
                f,
                "cannot read the requirements of '{}': {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Bundle { source, .. } | Self::Requirements { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_error_names_the_field_and_the_value() {
        let error = Error::ProfileField {
            field: "macos_version",
            value: String::from("Big Sur"),
            reason: "is not a numeric dotted version such as 11.0",
        };
        let message = error.to_string();

        assert!(message.contains("macos_version"), "message: {message}");
        assert!(message.contains("Big Sur"), "message: {message}");
        assert!(message.contains("numeric dotted"), "message: {message}");
    }

    #[test]
    fn a_status_error_lists_the_accepted_values() {
        let error = Error::ProfileStatus {
            table: "frameworks",
            subject: String::from("AppKit"),
            value: String::from("mostly"),
        };
        let message = error.to_string();

        assert!(message.contains("mostly"), "message: {message}");
        assert!(message.contains("AppKit"), "message: {message}");
        for accepted in ["supported", "partial", "unsupported", "unknown"] {
            assert!(message.contains(accepted), "message: {message}");
        }
    }

    #[test]
    fn a_bundle_error_names_the_path_and_keeps_the_cause() {
        let source = dp_bundle_inspector::Error::BundleNotDirectory {
            path: PathBuf::from("/tmp/Example.app"),
        };
        let error = Error::Bundle {
            path: PathBuf::from("/tmp/Example.app"),
            source: Box::new(source),
        };
        let message = error.to_string();

        assert!(message.contains("/tmp/Example.app"), "message: {message}");
        assert!(std::error::Error::source(&error).is_some());
    }
}
