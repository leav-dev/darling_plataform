//! Darling Execution Manager (DEM).
//!
//! Responsibility: run as the per-session user daemon that resolves a typed
//! launch request against the catalogue, checks bundle and executable existence,
//! surfaces known compatibility risks, ensures the DPREFIX is running, launches
//! through a verified Darling interface, supervises the process and publishes
//! state events. It keeps the prefix alive while managed applications run and
//! refuses to stop it while any of them are active.
//!
//! Not responsible for: reimplementing `darlingserver`, `launchd` or any other
//! Darling internal, nor for executing arbitrary shell commands on behalf of a
//! frontend. The IPC surface is typed and validated; a `host_pid` is a Linux
//! process, never a process inside the Darwin environment.

#[cfg(test)]
mod tests {
    #[test]
    fn declares_its_responsibility() {
        assert!(!env!("CARGO_PKG_DESCRIPTION").trim().is_empty());
    }
}
