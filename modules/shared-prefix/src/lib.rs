//! Shared Prefix Manager (SPM).
//!
//! Responsibility: be the only authority of the platform over the lifecycle of
//! the dedicated DPREFIX — initialize it through Darling's supported
//! mechanisms, validate its structure, start and stop the environment, and hold
//! the read/write locks that serialise incompatible operations.
//!
//! Not responsible for: fabricating Darling's internal directory layout,
//! reimplementing `darlingserver` or `launchd`, or storing platform data inside
//! the prefix. Catalogue, icons, logs and configuration live outside it.
//!
//! The reported states are the platform's own orchestration states; they are not
//! Darling's official internal states and must not be presented as such.
//!
//! # Module map
//!
//! - [`error`] — the typed failures every operation reports.
//! - [`location`] — where the dedicated prefix and the platform data directory
//!   live, resolved from an injected environment (never from the process inside
//!   library code).
//! - [`state`] — the persisted lifecycle phase, written atomically.
//!
//! # Verified Darling facts this crate must respect
//!
//! `DPREFIX` selects the prefix and must be exported to child processes; there is
//! no initialisation command, so a prefix appears on the first Darling run; and
//! Darling cannot be asked whether its container is alive, so the phase is ours.
//! See `ADR-0008`.

pub mod error;
pub mod location;
pub mod state;

pub use error::{Error, Result};
pub use location::{MAX_PREFIX_PATH_BYTES, PrefixEnv, PrefixLocation};
pub use state::{Phase, StateRecord, StateStore};

#[cfg(test)]
mod tests {
    #[test]
    fn declares_its_responsibility() {
        assert!(!env!("CARGO_PKG_DESCRIPTION").trim().is_empty());
    }
}
