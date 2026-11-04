//! The pinned Darling build.
//!
//! The platform runs against exactly one Darling release, recorded in
//! `deps/darling.toml` at the repository root. That file is embedded at compile
//! time, so the binary always reports the version it was built for and a bump is a
//! reviewed code change rather than an ambient property of the machine.
//!
//! Not responsible for: installing Darling, detecting the installed version, or
//! deciding whether the installed build is compatible. Upstream exposes no
//! documented command that reports its own version, so the platform verifies the
//! release asset **before** installing it (see `scripts/dev/install-darling-pin.sh`)
//! instead of guessing afterwards.

/// Contents of `deps/darling.toml`, embedded at compile time.
pub const PIN_FILE: &str = include_str!("../../../deps/darling.toml");

/// A pinned Darling release.
///
/// Only the fields the platform reports are modelled. The pin file carries more
/// (assets, hashes, patch registry) for the installer script and for humans; the
/// binary needs the identity it can print and compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct DarlingPin {
    /// Release tag, for example `v0.1.20260608`.
    pub version: &'static str,
    /// Exact commit the tag points at.
    pub commit: &'static str,
}

/// The pin of this build.
///
/// Values are parsed from [`PIN_FILE`] on every call: the file is a compile-time
/// constant, so this is a cheap scan, and keeping one source of truth is worth
/// more than caching three strings.
pub fn pinned() -> DarlingPin {
    DarlingPin {
        version: required_field(PIN_FILE, "version"),
        commit: required_field(PIN_FILE, "commit"),
    }
}

/// First quoted value of `key = "value"` in `document`.
///
/// Deliberately not a TOML parser: the pin file is owned by this repository, holds
/// one `version`/`commit` pair, and a missing key must fail loudly in tests rather
/// than fall back to a default.
fn field<'a>(document: &'a str, key: &str) -> Option<&'a str> {
    document.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        if name.trim() != key {
            return None;
        }

        value.trim().strip_prefix('"')?.strip_suffix('"')
    })
}

fn required_field(document: &'static str, key: &str) -> &'static str {
    field(document, key).unwrap_or_else(|| panic!("deps/darling.toml must define `{key}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pin this platform is built for. Changing `deps/darling.toml` must update
    /// this test in the same commit: it is the review gate for a version bump.
    #[test]
    fn pin_matches_the_release_this_platform_was_reviewed_against() {
        let pin = pinned();

        assert_eq!(pin.version, "v0.1.20260608");
        assert_eq!(pin.commit, "e947f0d5a3c6bba27e3631d9bf0da2a9840e6894");
    }

    #[test]
    fn the_pin_is_a_tag_and_never_a_branch() {
        assert!(
            pinned().version.starts_with('v'),
            "the pin must be a release tag, never a branch name"
        );
        assert!(
            !PIN_FILE.contains("channel = \"master\""),
            "a moving branch cannot be diagnosed"
        );
    }

    #[test]
    fn assets_record_complete_hashes() {
        for key in ["debs_sha256", "source_sha256"] {
            let hash = field(PIN_FILE, key).unwrap_or_else(|| panic!("`{key}` must be pinned"));
            assert_eq!(hash.len(), 64, "`{key}` must be a complete sha256");
        }

        assert_eq!(field(PIN_FILE, "debs_name"), Some("debs_20260608.zip"));
    }

    #[test]
    fn a_missing_field_is_none_and_a_present_one_is_parsed() {
        assert_eq!(
            field("[x]\nversion = \"1.2.3\"\n", "version"),
            Some("1.2.3")
        );
        assert_eq!(field("[x]\n# version = \"1.2.3\"\n", "version"), None);
        assert_eq!(field("version = 3\n", "version"), None);
    }

    #[test]
    #[should_panic(expected = "must define `commit`")]
    fn a_missing_required_field_panics_instead_of_defaulting() {
        required_field("[darling]\nversion = \"v1\"\n", "commit");
    }

    #[test]
    fn the_pin_serializes_with_the_fields_the_cli_reports() {
        let json = serde_json::to_string(&pinned()).expect("pin must serialize");

        assert!(
            json.contains("\"version\":\"v0.1.20260608\""),
            "json: {json}"
        );
        assert!(json.contains("\"commit\":\"e947f0d"), "json: {json}");
    }
}
