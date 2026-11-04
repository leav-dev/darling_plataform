//! Darling App Installer (DAI).
//!
//! Responsibility: import `.app` bundles, ZIP and — once the flow is verified —
//! DMG and PKG archives, extract them into staging, validate paths and structure
//! with the bundle inspector, install into the shared DPREFIX and record the
//! outcome in the application library.
//!
//! Not responsible for: declaring compatibility, executing installer scripts
//! without explicit consent, or overwriting an existing installation without a
//! documented policy. Bundles, archives and DMG/PKG files are untrusted input:
//! extraction must reject path traversal, malicious symlinks and writes outside
//! the allowed destinations, and PKG must be treated as the higher-risk flow it
//! is.

#[cfg(test)]
mod tests {
    #[test]
    fn declares_its_responsibility() {
        assert!(!env!("CARGO_PKG_DESCRIPTION").trim().is_empty());
    }
}
