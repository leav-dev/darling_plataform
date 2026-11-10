//! The versioned capability profile.
//!
//! `deps/darling-capabilities.toml` is the knowledge base the analysis compares
//! an application's requirements against. It is embedded at compile time
//! ([`EMBEDDED_PROFILE`]), so the report always cites the profile this build was
//! reviewed with and a correction is a reviewed code change instead of an
//! ambient property of the machine.
//!
//! # Reporting rules
//!
//! * Every field is validated. An unusable profile is an error that names the
//!   field and the value it read, never a silent default: a verdict built on a
//!   guessed baseline is worse than no verdict.
//! * `supported` is a claim that needs evidence. An entry the project has not
//!   verified against the pinned Darling build is `unknown`, which the analysis
//!   reports as unknown instead of as compatibility.
//! * [`Profile::dylib_status`] matches a dylib by exact path. The same library
//!   referenced as `@rpath/...` is a different subject and is not matched, so an
//!   in-bundle copy is never mistaken for the system library the profile judged.
//! * [`ProfileStatus`] is its own enum, mapped one to one onto the platform's
//!   support vocabulary — the four levels the hardware report's `SupportLevel`
//!   uses — and serialized `snake_case` like every other report this platform
//!   emits.

use std::collections::BTreeSet;

use serde::Deserialize;

use crate::version::Version;
use crate::{Error, Result};

/// Contents of `deps/darling-capabilities.toml`, embedded at compile time.
pub const EMBEDDED_PROFILE: &str = include_str!("../../../deps/darling-capabilities.toml");

/// How well the pinned Darling build supports one subject.
///
/// The variants and their names are the platform's support vocabulary: the
/// hardware report serializes the same four levels for the hardware it
/// describes. Describing a component never turns it into a supported one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileStatus {
    /// Known to work with the pinned Darling build.
    Supported,
    /// Works with documented limitations.
    Partial,
    /// Known not to work.
    Unsupported,
    /// Nobody verified it against the pinned build yet.
    Unknown,
}

impl ProfileStatus {
    /// The status a profile entry names, or `None` when the name is not one of
    /// the documented four.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "supported" => Some(Self::Supported),
            "partial" => Some(Self::Partial),
            "unsupported" => Some(Self::Unsupported),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }

    /// Machine name of this status, which is exactly the value the profile
    /// declares.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Partial => "partial",
            Self::Unsupported => "unsupported",
            Self::Unknown => "unknown",
        }
    }
}

/// One framework the profile judges, by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameworkEntry {
    /// Framework name without the `.framework` suffix, such as `AppKit`.
    pub name: String,
    /// How well the pinned build supports it.
    pub status: ProfileStatus,
    /// Why it has that status, when the profile records a reason.
    pub note: Option<String>,
}

/// One dynamic library the profile judges, by exact path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DylibEntry {
    /// Install name as a binary declares it, such as `/usr/lib/libz.1.dylib`.
    pub path: String,
    /// How well the pinned build supports it.
    pub status: ProfileStatus,
    /// Why it has that status, when the profile records a reason.
    pub note: Option<String>,
}

/// A validated capability profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// Identifies the revision of the knowledge base the report used.
    pub profile_version: String,
    /// Darwin version of the pinned build, cited as reference only.
    pub darwin_version: String,
    /// macOS version that Darwin represents, the comparison baseline.
    pub macos_version: String,
    /// Frameworks, in file order.
    pub frameworks: Vec<FrameworkEntry>,
    /// Dynamic libraries, in file order.
    pub dylibs: Vec<DylibEntry>,
}

impl Profile {
    /// Parses and validates a profile document.
    ///
    /// Every field is required except `note` and the two entry tables, because a
    /// profile without a baseline cannot produce a report. Unknown keys are
    /// rejected: a misspelled `status` must fail loudly instead of leaving the
    /// entry at a default the analysis would then trust.
    pub fn parse(text: &str) -> Result<Self> {
        let raw: RawProfile = toml::from_str(text).map_err(|error| Error::ProfileSyntax {
            reason: error.to_string(),
        })?;

        if raw.profile_version.trim().is_empty() {
            return Err(Error::ProfileField {
                field: "profile_version",
                value: raw.profile_version,
                reason: "is empty; declare the revision of the knowledge base it identifies",
            });
        }
        validate_version(
            "darwin_version",
            &raw.darwin_version,
            "is not a numeric Darwin version such as 20.6.0",
        )?;
        validate_version(
            "macos_version",
            &raw.macos_version,
            "is not a numeric macOS version such as 11.0",
        )?;

        let mut names = BTreeSet::new();
        let mut frameworks = Vec::with_capacity(raw.frameworks.len());
        for entry in raw.frameworks {
            if entry.name.trim().is_empty() {
                return Err(Error::ProfileField {
                    field: "frameworks.name",
                    value: entry.name,
                    reason: "is empty; name the framework without the .framework suffix",
                });
            }
            if !names.insert(entry.name.clone()) {
                return Err(Error::ProfileDuplicate {
                    table: "frameworks",
                    value: entry.name,
                });
            }
            let status = status_of("frameworks", &entry.name, entry.status)?;
            frameworks.push(FrameworkEntry {
                name: entry.name,
                status,
                note: entry.note,
            });
        }

        let mut paths = BTreeSet::new();
        let mut dylibs = Vec::with_capacity(raw.dylibs.len());
        for entry in raw.dylibs {
            if entry.path.trim().is_empty() {
                return Err(Error::ProfileField {
                    field: "dylibs.path",
                    value: entry.path,
                    reason: "is empty; declare the install name the binary uses, such as \
                             /usr/lib/libz.1.dylib",
                });
            }
            if !paths.insert(entry.path.clone()) {
                return Err(Error::ProfileDuplicate {
                    table: "dylibs",
                    value: entry.path,
                });
            }
            let status = status_of("dylibs", &entry.path, entry.status)?;
            dylibs.push(DylibEntry {
                path: entry.path,
                status,
                note: entry.note,
            });
        }

        Ok(Self {
            profile_version: raw.profile_version,
            darwin_version: raw.darwin_version,
            macos_version: raw.macos_version,
            frameworks,
            dylibs,
        })
    }

    /// The profile this build was compiled with.
    pub fn embedded() -> Result<Self> {
        Self::parse(EMBEDDED_PROFILE)
    }

    /// The framework entry whose name is exactly `name`.
    pub fn framework_entry(&self, name: &str) -> Option<&FrameworkEntry> {
        self.frameworks.iter().find(|entry| entry.name == name)
    }

    /// The dylib entry whose path is exactly `path`.
    pub fn dylib_entry(&self, path: &str) -> Option<&DylibEntry> {
        self.dylibs.iter().find(|entry| entry.path == path)
    }

    /// Status the profile records for framework `name`, or `None` when it has no
    /// entry for it.
    pub fn framework_status(&self, name: &str) -> Option<ProfileStatus> {
        self.framework_entry(name).map(|entry| entry.status)
    }

    /// Status the profile records for the exact dylib path `path`, or `None`.
    pub fn dylib_status(&self, path: &str) -> Option<ProfileStatus> {
        self.dylib_entry(path).map(|entry| entry.status)
    }
}

/// The profile as the TOML declares it, before validation.
///
/// Statuses are read as text so the error can quote the value; `serde`'s own
/// unknown-variant message would bury it in a parser diagnostic.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProfile {
    profile_version: String,
    darwin_version: String,
    macos_version: String,
    #[serde(default)]
    frameworks: Vec<RawFramework>,
    #[serde(default)]
    dylibs: Vec<RawDylib>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFramework {
    name: String,
    status: String,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDylib {
    path: String,
    status: String,
    #[serde(default)]
    note: Option<String>,
}

/// Rejects a version field the engine cannot order against an application's
/// declaration.
fn validate_version(field: &'static str, value: &str, reason: &'static str) -> Result<()> {
    if Version::parse(value).is_none() {
        return Err(Error::ProfileField {
            field,
            value: value.to_string(),
            reason,
        });
    }
    Ok(())
}

/// Maps the declared status name of one entry to [`ProfileStatus`].
fn status_of(table: &'static str, subject: &str, value: String) -> Result<ProfileStatus> {
    let status = ProfileStatus::from_name(&value).ok_or_else(|| Error::ProfileStatus {
        table,
        subject: subject.to_string(),
        value: value.clone(),
    })?;
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A profile with the three version fields and nothing else.
    const MINIMAL: &str = r#"
profile_version = "7"
darwin_version = "20.6.0"
macos_version = "11.0"
"#;

    /// A profile with one entry per table at the given statuses.
    fn with_entries(framework_status: &str, dylib_status: &str) -> String {
        format!(
            r#"
profile_version = "test"
darwin_version = "20.6.0"
macos_version = "11.0"

[[frameworks]]
name = "AppKit"
status = "{framework_status}"
note = "sin verificación registrada"

[[dylibs]]
path = "/usr/lib/libz.1.dylib"
status = "{dylib_status}"
"#
        )
    }

    #[test]
    fn the_embedded_profile_parses_and_is_versioned() {
        let profile = Profile::embedded().expect("the embedded profile is valid");

        assert_eq!(profile.profile_version, "1");
        assert_eq!(profile.darwin_version, "20.6.0");
        assert_eq!(profile.macos_version, "11.6");
        assert!(!profile.frameworks.is_empty());
        assert!(!profile.dylibs.is_empty());
    }

    #[test]
    fn the_embedded_profile_claims_nothing_it_has_not_verified() {
        let profile = Profile::embedded().expect("the embedded profile is valid");

        // The honest baseline: this cut recorded no verification against the pin,
        // so every entry is `unknown`. A verified entry is a reviewed change that
        // updates this test with its evidence.
        for framework in &profile.frameworks {
            assert_eq!(
                framework.status,
                ProfileStatus::Unknown,
                "{} claims a status no evidence supports",
                framework.name
            );
        }
        for dylib in &profile.dylibs {
            assert_eq!(
                dylib.status,
                ProfileStatus::Unknown,
                "{} claims a status no evidence supports",
                dylib.path
            );
        }
    }

    #[test]
    fn the_embedded_profile_names_the_subjects_the_cut_must_judge() {
        let profile = Profile::embedded().expect("the embedded profile is valid");

        for name in [
            "AppKit",
            "Foundation",
            "CoreFoundation",
            "CoreGraphics",
            "Metal",
            "Security",
        ] {
            assert_eq!(
                profile.framework_status(name),
                Some(ProfileStatus::Unknown),
                "the profile must record {name}"
            );
        }
        for path in [
            "/usr/lib/libSystem.B.dylib",
            "/usr/lib/libz.1.dylib",
            "/usr/lib/libsqlite3.dylib",
        ] {
            assert_eq!(
                profile.dylib_status(path),
                Some(ProfileStatus::Unknown),
                "the profile must record {path}"
            );
        }
    }

    #[test]
    fn a_minimal_profile_parses_with_empty_tables() {
        let profile = Profile::parse(MINIMAL).expect("the three fields are enough");

        assert_eq!(profile.profile_version, "7");
        assert_eq!(profile.darwin_version, "20.6.0");
        assert_eq!(profile.macos_version, "11.0");
        assert!(profile.frameworks.is_empty());
        assert!(profile.dylibs.is_empty());
    }

    #[test]
    fn entries_expose_their_status_note_and_exact_subject() {
        let profile =
            Profile::parse(&with_entries("partial", "unsupported")).expect("the fixture is valid");

        let framework = profile
            .framework_entry("AppKit")
            .expect("the framework is recorded");
        assert_eq!(framework.status, ProfileStatus::Partial);
        assert_eq!(
            framework.note.as_deref(),
            Some("sin verificación registrada")
        );

        let dylib = profile
            .dylib_entry("/usr/lib/libz.1.dylib")
            .expect("the dylib is recorded");
        assert_eq!(dylib.status, ProfileStatus::Unsupported);
        assert_eq!(dylib.note, None);
    }

    #[test]
    fn every_documented_status_name_round_trips() {
        for (name, status) in [
            ("supported", ProfileStatus::Supported),
            ("partial", ProfileStatus::Partial),
            ("unsupported", ProfileStatus::Unsupported),
            ("unknown", ProfileStatus::Unknown),
        ] {
            assert_eq!(ProfileStatus::from_name(name), Some(status));
            assert_eq!(status.as_str(), name);
        }
        assert_eq!(ProfileStatus::from_name("mostly"), None);
        assert_eq!(ProfileStatus::from_name("Supported"), None);
    }

    #[test]
    fn lookups_report_nothing_for_subjects_the_profile_does_not_judge() {
        let profile =
            Profile::parse(&with_entries("supported", "supported")).expect("the fixture is valid");

        assert_eq!(profile.framework_status("CoreAudio"), None);
        assert_eq!(profile.dylib_status("/usr/lib/libc++.1.dylib"), None);
        assert_eq!(profile.framework_entry("CoreAudio"), None);
        assert_eq!(profile.dylib_entry("/usr/lib/libc++.1.dylib"), None);
    }

    #[test]
    fn a_dylib_referenced_with_a_loader_prefix_is_not_matched_by_exact_path() {
        let profile =
            Profile::parse(&with_entries("supported", "supported")).expect("the fixture is valid");

        assert_eq!(
            profile.dylib_status("/usr/lib/libz.1.dylib"),
            Some(ProfileStatus::Supported)
        );
        assert_eq!(profile.dylib_status("@rpath/libz.1.dylib"), None);
        assert_eq!(profile.dylib_status("@executable_path/libz.1.dylib"), None);
        assert_eq!(profile.dylib_status("libz.1.dylib"), None);
    }

    #[test]
    fn an_empty_profile_version_is_rejected_with_the_field_and_value() {
        let text = MINIMAL.replace("\"7\"", "\"\"");
        let error = Profile::parse(&text).expect_err("an empty version identifies nothing");
        let message = error.to_string();

        assert!(
            matches!(
                error,
                Error::ProfileField {
                    field: "profile_version",
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
        assert!(message.contains("profile_version"), "message: {message}");
        assert!(message.contains("empty"), "message: {message}");
    }

    #[test]
    fn an_unparsable_macos_version_names_the_value() {
        let text = MINIMAL.replace("\"11.0\"", "\"Big Sur\"");
        let error = Profile::parse(&text).expect_err("a version must be numeric");
        let message = error.to_string();

        assert!(
            matches!(
                error,
                Error::ProfileField {
                    field: "macos_version",
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
        assert!(message.contains("Big Sur"), "message: {message}");
        assert!(message.contains("macos_version"), "message: {message}");
    }

    #[test]
    fn an_unparsable_darwin_version_names_the_value() {
        let text = MINIMAL.replace("\"20.6.0\"", "\"twenty\"");
        let error = Profile::parse(&text).expect_err("a version must be numeric");
        let message = error.to_string();

        assert!(
            matches!(
                error,
                Error::ProfileField {
                    field: "darwin_version",
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
        assert!(message.contains("twenty"), "message: {message}");
    }

    #[test]
    fn a_duplicate_framework_name_is_rejected_with_the_value() {
        let text = format!(
            "{MINIMAL}\n[[frameworks]]\nname = \"AppKit\"\nstatus = \"unknown\"\n\
             \n[[frameworks]]\nname = \"AppKit\"\nstatus = \"supported\"\n"
        );
        let error = Profile::parse(&text).expect_err("a duplicate makes the verdict ambiguous");
        let message = error.to_string();

        assert!(
            matches!(
                error,
                Error::ProfileDuplicate {
                    table: "frameworks",
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
        assert!(message.contains("AppKit"), "message: {message}");
    }

    #[test]
    fn a_duplicate_dylib_path_is_rejected_with_the_value() {
        let text = format!(
            "{MINIMAL}\n[[dylibs]]\npath = \"/usr/lib/libz.1.dylib\"\nstatus = \"unknown\"\n\
             \n[[dylibs]]\npath = \"/usr/lib/libz.1.dylib\"\nstatus = \"unknown\"\n"
        );
        let error = Profile::parse(&text).expect_err("a duplicate makes the verdict ambiguous");

        assert!(
            matches!(
                error,
                Error::ProfileDuplicate {
                    table: "dylibs",
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
        assert!(
            error.to_string().contains("/usr/lib/libz.1.dylib"),
            "the message must name the value: {error}"
        );
    }

    #[test]
    fn an_unknown_status_is_rejected_with_the_table_subject_and_value() {
        let text = with_entries("mostly", "unknown");
        let error = Profile::parse(&text).expect_err("a status is one of four names");
        let message = error.to_string();

        assert!(
            matches!(
                error,
                Error::ProfileStatus {
                    table: "frameworks",
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
        assert!(message.contains("mostly"), "message: {message}");
        assert!(message.contains("AppKit"), "message: {message}");
    }

    #[test]
    fn an_unknown_dylib_status_is_rejected_too() {
        let error = Profile::parse(&with_entries("unknown", "probably"))
            .expect_err("a status is one of four names");

        assert!(
            matches!(
                error,
                Error::ProfileStatus {
                    table: "dylibs",
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
        assert!(
            error.to_string().contains("probably"),
            "the message must name the value: {error}"
        );
    }

    #[test]
    fn an_entry_without_a_subject_is_rejected() {
        let text = format!("{MINIMAL}\n[[frameworks]]\nname = \"\"\nstatus = \"unknown\"\n");
        let error = Profile::parse(&text).expect_err("an entry must name its subject");

        assert!(
            matches!(
                error,
                Error::ProfileField {
                    field: "frameworks.name",
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );

        let text = format!("{MINIMAL}\n[[dylibs]]\npath = \" \"\nstatus = \"unknown\"\n");
        let error = Profile::parse(&text).expect_err("an entry must name its subject");

        assert!(
            matches!(
                error,
                Error::ProfileField {
                    field: "dylibs.path",
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn a_missing_required_field_is_rejected() {
        let error = Profile::parse("darwin_version = \"20.6.0\"\nmacos_version = \"11.0\"\n")
            .expect_err("the profile version is required");

        assert!(
            matches!(error, Error::ProfileSyntax { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            error.to_string().contains("profile_version"),
            "the message must name the field: {error}"
        );
    }

    #[test]
    fn a_misspelled_key_is_rejected_instead_of_ignored() {
        let text = format!("{MINIMAL}\n[[frameworks]]\nname = \"AppKit\"\nstatu = \"unknown\"\n");
        let error = Profile::parse(&text).expect_err("a misspelled key must not default");

        assert!(
            matches!(error, Error::ProfileSyntax { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            error.to_string().contains("statu"),
            "the message must name the unknown key: {error}"
        );
    }

    #[test]
    fn a_document_that_is_not_toml_is_rejected() {
        let error = Profile::parse("profile_version = ").expect_err("that is not a document");

        assert!(
            matches!(error, Error::ProfileSyntax { .. }),
            "unexpected error: {error:?}"
        );
    }
}
