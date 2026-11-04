//! Shared domain primitives for the Darling Platform modules.
//!
//! Responsibility: stable application and instance identifiers, typed errors
//! and diagnostic records that every other module can depend on without
//! depending on a concrete interface.
//!
//! Not responsible for: reading hardware, touching the DPREFIX, or talking to
//! D-Bus. This crate stays free of platform-specific side effects.

#[cfg(test)]
mod tests {
    #[test]
    fn declares_its_responsibility() {
        assert!(!env!("CARGO_PKG_DESCRIPTION").trim().is_empty());
    }
}
