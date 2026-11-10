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
//!
//! # Reporting rules
//!
//! * The capability profile (`deps/darling-capabilities.toml`) is embedded at
//!   compile time and validated on every load; an unusable profile is an error,
//!   never a silent default.
//! * Every requirement carries the evidence that decided it: the path, the
//!   version and the profile entry the outcome rests on.
//! * Enums serialize as `snake_case` machine names, an absent optional value is
//!   omitted instead of serialized as `null`, and collections are always
//!   present, matching every other JSON document this platform emits.
//! * The verdict is the worst requirement: `not_compatible` for a requirement
//!   that is definitely unsatisfied, then `unknown` for one nobody could decide,
//!   then `partial` for advice, and `compatible` only when everything was
//!   checked and satisfied. An empty report is `unknown`, never `compatible`.
//!
//! # Determinism
//!
//! Nothing here reads the host: the host architecture arrives in
//! [`AnalysisRequest`], so the same bundle and the same profile always produce
//! the same report.

pub mod analysis;
pub mod profile;

mod error;
mod version;

#[cfg(test)]
mod test_support;

pub use analysis::{
    ANALYSER_VERSION, AnalysisRequest, CompatibilityReport, Requirement, RequirementKind,
    RequirementStatus, SCHEMA_VERSION, Verdict, analyze, analyze_with_profile, verdict,
};
pub use error::{Error, Result};
pub use profile::{DylibEntry, EMBEDDED_PROFILE, FrameworkEntry, Profile, ProfileStatus};

#[cfg(test)]
mod tests {
    #[test]
    fn declares_its_responsibility() {
        assert!(!env!("CARGO_PKG_DESCRIPTION").trim().is_empty());
    }
}
