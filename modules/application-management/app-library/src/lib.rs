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

#[cfg(test)]
mod tests {
    #[test]
    fn declares_its_responsibility() {
        assert!(!env!("CARGO_PKG_DESCRIPTION").trim().is_empty());
    }
}
