//! Bundle Inspector.
//!
//! Responsibility: read a `.app` bundle without running it and extract
//! `CFBundleIdentifier`, name, display name, version, executable, icon,
//! `LSMinimumSystemVersion`, Mach-O architectures and the referenced frameworks
//! and resources.
//!
//! Not responsible for: executing the binary, installing anything, or deciding
//! compatibility. Every path declared by the bundle must be proven to stay
//! inside the bundle, and the declared executable must be shown to exist before
//! its metadata is trusted. `Info.plist` and Mach-O headers are untrusted input.

#[cfg(test)]
mod tests {
    #[test]
    fn declares_its_responsibility() {
        assert!(!env!("CARGO_PKG_DESCRIPTION").trim().is_empty());
    }
}
