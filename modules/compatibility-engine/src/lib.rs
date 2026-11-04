//! Compatibility Engine.
//!
//! Responsibility: analyse an application's requirements — Mach-O slices,
//! dynamic dependencies, referenced frameworks, declared minimum macOS version,
//! bundle metadata — and compare them with a versioned profile of the
//! capabilities of the pinned Darling build and the detected host.
//!
//! Not responsible for: executing the application to inspect it, mutating the
//! prefix, or promising compatibility. A favourable static analysis is evidence,
//! never a guarantee: the report must state what was checked, what failed and
//! what remains unknown, together with the analyser and capability-profile
//! versions it used.

#[cfg(test)]
mod tests {
    #[test]
    fn declares_its_responsibility() {
        assert!(!env!("CARGO_PKG_DESCRIPTION").trim().is_empty());
    }
}
