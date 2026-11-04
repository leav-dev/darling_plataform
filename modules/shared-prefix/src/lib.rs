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

#[cfg(test)]
mod tests {
    #[test]
    fn declares_its_responsibility() {
        assert!(!env!("CARGO_PKG_DESCRIPTION").trim().is_empty());
    }
}
