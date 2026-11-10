//! Application Library.
//!
//! Responsibility: persist the catalogue of applications, their installation
//! history and their running instances in SQLite, with versioned schema
//! migrations, outside the DPREFIX.
//!
//! Not responsible for: being the single source of truth about files on disk.
//! Before launching, updating or uninstalling, callers must verify that the
//! recorded bundle and executable paths still exist and are valid. The
//! catalogue also must not collapse "installed" and "compatible" into one state:
//! an application can be installed and unlaunchable at the same time.
//!
//! Stored so far: the `applications` table described by the plan and an
//! `installations` table that records every attempt, successful or not. Icon
//! files and caches are stored next to the catalogue by the installer, not by
//! this crate, and running instances are not tracked yet: nothing launches an
//! application in this cut, so there is no truth to record.
//!
//! The library stores text and never fabricates it. Ids, timestamps and status
//! values arrive from the caller, so a caller can pin them and a test can read
//! them back unchanged.
//!
mod error;
mod library;
mod records;
mod schema;
mod time;

pub use error::{Error, Result};
pub use library::Library;
pub use records::{ApplicationRecord, InstallationRecord};
pub use time::now_utc;

#[cfg(test)]
mod tests {
    #[test]
    fn declares_its_responsibility() {
        assert!(!env!("CARGO_PKG_DESCRIPTION").trim().is_empty());
    }
}
