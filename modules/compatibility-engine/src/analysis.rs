//! The static analysis engine.
//!
//! [`analyze`] compares what an application's bundle declares — its Mach-O
//! slices, its dynamic libraries, the frameworks it references and its declared
//! minimum macOS version — with the capability profile ([`crate::Profile`]) and
//! reports one [`Requirement`] per evidence, plus the [`Verdict`] the worst of
//! them implies.
//!
//! A favourable verdict is evidence, never a promise: the analysis never runs the
//! application, never resolves an install name and never asserts that a library
//! works merely because the profile lists it. The report carries
//! [`ANALYSER_VERSION`] and the profile's version so a consumer can tell which
//! analyser and which knowledge base produced it.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::path::Path;

use dp_bundle_inspector::{Architecture, BinaryRequirements, BundleMetadata};
use serde::Serialize;

use crate::profile::{Profile, ProfileStatus};
use crate::version::Version;
use crate::{Error, Result};

/// Version of the analysis rules in this file.
///
/// Bumping it is how a consumer tells two differently reasoned reports apart,
/// for example after a verdict rule or a check changes meaning.
pub const ANALYSER_VERSION: &str = "1";

/// Version of the JSON shape emitted by [`CompatibilityReport`].
pub const SCHEMA_VERSION: &str = "1.0";

/// One application to analyse, plus the host fact the engine must not guess.
///
/// The host architecture is supplied by the caller (the CLI passes
/// `std::env::consts::ARCH`), so the engine stays deterministic and testable on
/// any machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalysisRequest<'a> {
    /// Bundle to analyse, as the operator named it.
    pub bundle: &'a Path,
    /// Architecture of the host that would run the application, such as
    /// `x86_64` or `aarch64`.
    pub host_architecture: &'a str,
}

/// Everything one analysis established, with the versions that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompatibilityReport {
    /// Shape identifier; see [`SCHEMA_VERSION`].
    pub schema_version: String,
    /// Worst requirement, see [`verdict`].
    pub verdict: Verdict,
    /// Rules that produced this report; see [`ANALYSER_VERSION`].
    pub analyser_version: String,
    /// Capability profile the comparison used.
    pub profile_version: String,
    /// One entry per piece of evidence, in a stable order.
    pub requirements: Vec<Requirement>,
}

/// Overall result of one analysis.
///
/// The variants are declared in the order [`verdict`] ranks them, from a fully
/// satisfied bundle to one that is definitely unsatisfied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Every checked requirement was satisfied.
    Compatible,
    /// Everything was satisfied or only advised against.
    Partial,
    /// At least one requirement is definitely unsatisfied.
    NotCompatible,
    /// Nothing failed, but at least one requirement cannot be decided.
    Unknown,
}

/// What kind of evidence one [`Requirement`] states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementKind {
    /// A Mach-O slice of the executable against the host architecture.
    Architecture,
    /// A library the executable's load commands name.
    DynamicLibrary,
    /// A framework referenced by the executable or shipped in the bundle.
    Framework,
    /// The minimum macOS version the application declares.
    MinimumOs,
}

/// How one requirement came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementStatus {
    /// Checked and satisfied.
    Satisfied,
    /// Satisfied with a documented limitation or a reason the profile does not
    /// decide it.
    Advisory,
    /// Not decided: nothing in the knowledge base covers it.
    Unknown,
    /// Checked and definitely not satisfied.
    Unsatisfied,
}

/// One piece of evidence and its outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Requirement {
    /// Kind of evidence, so a consumer can group or filter it.
    pub kind: RequirementKind,
    /// What was checked: an architecture, a library or framework name or path,
    /// or a declared version. It is never empty.
    pub subject: String,
    /// How the requirement came out.
    pub status: RequirementStatus,
    /// Why, in the language of the evidence: the paths, versions and profile
    /// entries the outcome rests on.
    pub detail: String,
}

/// Analyses `request` against the profile this build was compiled with.
pub fn analyze(request: &AnalysisRequest<'_>) -> Result<CompatibilityReport> {
    let profile = Profile::embedded()?;
    analyze_with_profile(request, &profile)
}

/// Analyses `request` against `profile`.
///
/// The embedded profile errors on a broken knowledge base; this entry point lets
/// a caller analyse with a profile it read itself, and lets the tests state an
/// explicit knowledge base instead of depending on the file's contents.
pub fn analyze_with_profile(
    request: &AnalysisRequest<'_>,
    profile: &Profile,
) -> Result<CompatibilityReport> {
    let metadata =
        dp_bundle_inspector::inspect(request.bundle).map_err(|source| Error::Bundle {
            path: request.bundle.to_path_buf(),
            source: Box::new(source),
        })?;
    let binary =
        dp_bundle_inspector::requirements(&metadata.executable_path).map_err(|source| {
            Error::Requirements {
                path: metadata.executable_path.clone(),
                source: Box::new(source),
            }
        })?;

    let mut requirements = architecture_requirements(&metadata, request.host_architecture);
    requirements.extend(dylib_requirements(&binary, profile));
    requirements.extend(framework_requirements(&metadata, &binary, profile));
    requirements.push(minimum_os_requirement(&metadata, &binary, profile));

    Ok(CompatibilityReport {
        schema_version: SCHEMA_VERSION.to_string(),
        verdict: verdict(&requirements),
        analyser_version: ANALYSER_VERSION.to_string(),
        profile_version: profile.profile_version.clone(),
        requirements,
    })
}

/// Derives the verdict from the worst requirement, per the platform's severity
/// rule.
///
/// Any [`RequirementStatus::Unsatisfied`] is `not_compatible`; otherwise any
/// [`RequirementStatus::Unknown`] is `unknown`, because a requirement nobody
/// could decide must never be presented as compatible or as a failure;
/// otherwise any [`RequirementStatus::Advisory`] is `partial`; only an
/// all-satisfied, non-empty list is `compatible`. An empty list is `unknown`:
/// nothing was checked, so nothing is known.
pub fn verdict(requirements: &[Requirement]) -> Verdict {
    if requirements
        .iter()
        .any(|requirement| requirement.status == RequirementStatus::Unsatisfied)
    {
        return Verdict::NotCompatible;
    }
    if requirements
        .iter()
        .any(|requirement| requirement.status == RequirementStatus::Unknown)
    {
        return Verdict::Unknown;
    }
    if requirements
        .iter()
        .any(|requirement| requirement.status == RequirementStatus::Advisory)
    {
        return Verdict::Partial;
    }
    if requirements.is_empty() {
        return Verdict::Unknown;
    }
    Verdict::Compatible
}

/// Suffix that marks a framework directory component in a load command or in a
/// bundle-relative path.
const FRAMEWORK_SUFFIX: &str = ".framework";

/// Install-name prefixes with which the loader resolves a library inside the
/// application bundle instead of in the prefix.
const LOADER_PREFIXES: [&str; 3] = ["@rpath/", "@executable_path/", "@loader_path/"];

/// Checks the executable's slices against the host architecture.
///
/// One requirement per slice the host can run; when no slice can run, a single
/// unsatisfied requirement names every slice the executable ships, because their
/// union is the evidence that nothing translates the binary.
fn architecture_requirements(
    metadata: &BundleMetadata,
    host_architecture: &str,
) -> Vec<Requirement> {
    let host = host_architecture.trim();
    let mut shipped = Vec::with_capacity(metadata.architectures.len());
    let mut matching = Vec::new();
    for architecture in &metadata.architectures {
        let name = host_name(architecture);
        if name.eq_ignore_ascii_case(host) {
            matching.push(name.clone());
        }
        shipped.push(name);
    }

    if matching.is_empty() {
        let subject = if shipped.is_empty() {
            String::from("none")
        } else {
            shipped.join(", ")
        };
        return vec![Requirement {
            kind: RequirementKind::Architecture,
            subject: subject.clone(),
            status: RequirementStatus::Unsatisfied,
            detail: format!(
                "the executable ships {subject} and the host architecture is '{host}'; no slice \
                 can run on this host and nothing in the platform translates the binary"
            ),
        }];
    }

    matching
        .into_iter()
        .map(|name| Requirement {
            kind: RequirementKind::Architecture,
            subject: name.clone(),
            status: RequirementStatus::Satisfied,
            detail: format!(
                "the executable ships a {name} slice and the host architecture is '{host}', so \
                 this slice runs natively"
            ),
        })
        .collect()
}

/// Name the platform uses for the host architecture a Mach-O slice targets.
///
/// Mach-O spells Apple silicon `arm64`; the rest of the platform, and Rust,
/// spell it `aarch64`. The remaining names are already lowercase.
fn host_name(architecture: &Architecture) -> String {
    match architecture {
        Architecture::Arm64 => String::from("aarch64"),
        Architecture::X86_64 => String::from("x86_64"),
        Architecture::Arm => String::from("arm"),
        Architecture::I386 => String::from("i386"),
        Architecture::PowerPc => String::from("powerpc"),
        Architecture::Unknown(cputype) => format!("unknown-{cputype:08x}"),
    }
}

/// Whether the loader resolves `install_name` inside the application bundle.
fn is_bundle_relative(install_name: &str) -> bool {
    LOADER_PREFIXES
        .iter()
        .any(|prefix| install_name.starts_with(prefix))
}

/// Checks the libraries the load commands name, except the ones that name a
/// framework: those are judged once, as frameworks, in
/// [`framework_requirements`].
fn dylib_requirements(binary: &BinaryRequirements, profile: &Profile) -> Vec<Requirement> {
    binary
        .dylibs
        .iter()
        .filter(|install_name| framework_name(install_name).is_none())
        .map(|install_name| {
            if is_bundle_relative(install_name) {
                return Requirement {
                    kind: RequirementKind::DynamicLibrary,
                    subject: install_name.clone(),
                    status: RequirementStatus::Advisory,
                    detail: format!(
                        "'{install_name}' is a loader-relative install name, so the library ships \
                         inside the bundle; the capability profile does not decide it"
                    ),
                };
            }
            match profile.dylib_entry(install_name) {
                Some(entry) => judged_requirement(
                    RequirementKind::DynamicLibrary,
                    install_name,
                    install_name,
                    entry.status,
                    entry.note.as_deref(),
                    profile,
                ),
                None => {
                    unfamiliar_requirement(RequirementKind::DynamicLibrary, install_name, profile)
                }
            }
        })
        .collect()
}

/// One framework the executable or the bundle references.
struct FrameworkReference {
    /// Install name or bundle-relative path that proves the reference.
    evidence: String,
    /// Whether the bundle ships its own copy.
    embedded: bool,
}

/// Collects the frameworks the executable links against and the ones the bundle
/// ships, keyed by framework name so each is judged once.
fn framework_references(
    metadata: &BundleMetadata,
    binary: &BinaryRequirements,
) -> BTreeMap<String, FrameworkReference> {
    let mut references = BTreeMap::new();
    for install_name in &binary.dylibs {
        if let Some(name) = framework_name(install_name) {
            record_framework(
                &mut references,
                name,
                install_name.clone(),
                is_bundle_relative(install_name),
            );
        }
    }
    // Everything the inspector lists lives under Contents/Frameworks, so it is
    // inside the bundle by construction.
    for path in &metadata.frameworks {
        if let Some(name) = framework_name(path) {
            record_framework(&mut references, name, path.clone(), true);
        }
    }
    references
}

/// Records one reference, keeping an embedded copy as the deciding evidence.
///
/// A framework the bundle ships is resolved by the loader from inside the
/// bundle, so a load command that also names a system copy does not make the
/// profile decide it.
fn record_framework(
    references: &mut BTreeMap<String, FrameworkReference>,
    name: String,
    evidence: String,
    embedded: bool,
) {
    match references.entry(name) {
        Entry::Occupied(mut entry) => {
            if embedded && !entry.get().embedded {
                entry.insert(FrameworkReference { evidence, embedded });
            }
        }
        Entry::Vacant(entry) => {
            entry.insert(FrameworkReference { evidence, embedded });
        }
    }
}

/// Checks the frameworks the executable references against the profile.
///
/// A framework shipped inside the bundle is advisory: the loader resolves it
/// from the application, so the profile's entry for a system copy of the same
/// name says nothing about the copy that will actually load.
fn framework_requirements(
    metadata: &BundleMetadata,
    binary: &BinaryRequirements,
    profile: &Profile,
) -> Vec<Requirement> {
    framework_references(metadata, binary)
        .into_iter()
        .map(|(name, reference)| {
            let FrameworkReference { evidence, embedded } = reference;
            if embedded {
                return Requirement {
                    kind: RequirementKind::Framework,
                    subject: name,
                    status: RequirementStatus::Advisory,
                    detail: format!(
                        "the framework ships inside the bundle ('{evidence}'), so the loader \
                         resolves it from the application and the capability profile does not \
                         decide it"
                    ),
                };
            }
            match profile.framework_entry(&name) {
                Some(entry) => judged_requirement(
                    RequirementKind::Framework,
                    &name,
                    &evidence,
                    entry.status,
                    entry.note.as_deref(),
                    profile,
                ),
                None => unfamiliar_requirement(RequirementKind::Framework, &name, profile),
            }
        })
        .collect()
}

/// The framework a path names, if any: the component that ends in
/// `.framework`, without the suffix.
fn framework_name(path: &str) -> Option<String> {
    path.split('/').find_map(|component| {
        component
            .strip_suffix(FRAMEWORK_SUFFIX)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
    })
}

/// Turns a profile entry into a requirement, quoting the status, the evidence
/// and the profile's own note.
fn judged_requirement(
    kind: RequirementKind,
    subject: &str,
    evidence: &str,
    profile_status: ProfileStatus,
    note: Option<&str>,
    profile: &Profile,
) -> Requirement {
    let profile_version = &profile.profile_version;
    let (status, mut detail) = match profile_status {
        ProfileStatus::Supported => (
            RequirementStatus::Satisfied,
            format!(
                "the capability profile (version {profile_version}) records '{subject}' as \
                 supported"
            ),
        ),
        ProfileStatus::Partial => (
            RequirementStatus::Advisory,
            format!(
                "the capability profile (version {profile_version}) records '{subject}' as \
                 partial: it works with documented limitations"
            ),
        ),
        ProfileStatus::Unsupported => (
            RequirementStatus::Unsatisfied,
            format!(
                "the capability profile (version {profile_version}) records '{subject}' as \
                 unsupported: it does not work under the pinned Darling build"
            ),
        ),
        ProfileStatus::Unknown => (
            RequirementStatus::Unknown,
            format!(
                "the capability profile (version {profile_version}) records '{subject}' as \
                 unknown: nobody has verified it against the pinned Darling build"
            ),
        ),
    };
    if evidence != subject {
        detail.push_str(&format!("; the executable references '{evidence}'"));
    }
    if let Some(note) = note {
        detail.push_str(&format!("; the profile notes: {note}"));
    }
    Requirement {
        kind,
        subject: subject.to_string(),
        status,
        detail,
    }
}

/// The requirement for a subject the profile does not judge at all. Absence is
/// reported as unknown, never as a pass.
fn unfamiliar_requirement(kind: RequirementKind, subject: &str, profile: &Profile) -> Requirement {
    Requirement {
        kind,
        subject: subject.to_string(),
        status: RequirementStatus::Unknown,
        detail: format!(
            "the capability profile (version {}) has no entry for '{subject}', so nothing is \
             known about it",
            profile.profile_version
        ),
    }
}

/// Compares the application's declared minimum macOS version with the profile's
/// baseline.
///
/// The plist declaration wins over the load commands': it is the one macOS
/// itself reports to the user. A declared version that cannot be parsed is
/// unknown and names the value, never a failure.
fn minimum_os_requirement(
    metadata: &BundleMetadata,
    binary: &BinaryRequirements,
    profile: &Profile,
) -> Requirement {
    let declared = metadata
        .minimum_system_version
        .as_deref()
        .or(binary.minimum_os.as_deref());
    let profile_macos = &profile.macos_version;
    let Some(declared) = declared else {
        return Requirement {
            kind: RequirementKind::MinimumOs,
            subject: String::from("none"),
            status: RequirementStatus::Unknown,
            detail: format!(
                "neither the bundle metadata nor the executable's load commands declare a minimum \
                 macOS version, so nothing can be compared with the profile's macOS {profile_macos}"
            ),
        };
    };
    let Some(declared_version) = Version::parse(declared) else {
        return Requirement {
            kind: RequirementKind::MinimumOs,
            subject: declared.to_string(),
            status: RequirementStatus::Unknown,
            detail: format!(
                "the application declares the minimum macOS version '{declared}', which is not a \
                 numeric dotted version, so it cannot be compared with the profile's macOS \
                 {profile_macos}"
            ),
        };
    };
    let Some(baseline) = Version::parse(profile_macos) else {
        return Requirement {
            kind: RequirementKind::MinimumOs,
            subject: declared.to_string(),
            status: RequirementStatus::Unknown,
            detail: format!(
                "the capability profile declares the macOS version '{profile_macos}', which is \
                 not a numeric dotted version, so '{declared}' cannot be compared with it"
            ),
        };
    };

    let (status, detail) = if declared_version <= baseline {
        (
            RequirementStatus::Satisfied,
            format!(
                "the application declares macOS '{declared}', which the profile's macOS \
                 {profile_macos} (Darwin {}) covers",
                profile.darwin_version
            ),
        )
    } else {
        (
            RequirementStatus::Unsatisfied,
            format!(
                "the application declares macOS '{declared}', which is newer than the profile's \
                 macOS {profile_macos} (Darwin {}); the pinned Darling build cannot provide what \
                 it asks for",
                profile.darwin_version
            ),
        )
    };
    Requirement {
        kind: RequirementKind::MinimumOs,
        subject: declared.to_string(),
        status,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Profile;
    use crate::test_support::{
        CPU_TYPE_ARM64, CPU_TYPE_X86_64, PLATFORM_MACOS, app_named, build_version_command,
        create_dir, dylib_command, fat_macho, packed_version, thin_macho, write_app, write_file,
    };

    /// A profile built from the given `(name, status)` and `(path, status)`
    /// pairs, with a fixed baseline of macOS 11.0.
    fn profile_of(frameworks: &[(&str, &str)], dylibs: &[(&str, &str)]) -> Profile {
        let mut text = String::from(
            "profile_version = \"test\"\ndarwin_version = \"20.6.0\"\nmacos_version = \"11.0\"\n",
        );
        for (name, status) in frameworks {
            text.push_str(&format!(
                "\n[[frameworks]]\nname = \"{name}\"\nstatus = \"{status}\"\n"
            ));
        }
        for (path, status) in dylibs {
            text.push_str(&format!(
                "\n[[dylibs]]\npath = \"{path}\"\nstatus = \"{status}\"\n"
            ));
        }
        Profile::parse(&text).expect("the fixture profile is valid")
    }

    /// An x86_64 executable carrying `commands`.
    fn x86_64_app(commands: &[Vec<u8>]) -> Vec<u8> {
        thin_macho(CPU_TYPE_X86_64, commands)
    }

    /// The `AppKit` dylib `LC_LOAD_DYLIB` path a real application declares.
    const APPKIT: &str = "/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit";
    /// The zlib dylib a real application declares.
    const LIBZ: &str = "/usr/lib/libz.1.dylib";

    fn report_for(bundle: &Path, host: &str, profile: &Profile) -> CompatibilityReport {
        analyze_with_profile(
            &AnalysisRequest {
                bundle,
                host_architecture: host,
            },
            profile,
        )
        .expect("the fixture bundle is analysable")
    }

    fn requirement<'a>(
        report: &'a CompatibilityReport,
        kind: RequirementKind,
        subject: &str,
    ) -> &'a Requirement {
        report
            .requirements
            .iter()
            .find(|requirement| requirement.kind == kind && requirement.subject == subject)
            .unwrap_or_else(|| {
                panic!(
                    "the report must hold a {kind:?} requirement for '{subject}': {:?}",
                    report.requirements
                )
            })
    }

    fn status_of(
        report: &CompatibilityReport,
        kind: RequirementKind,
        subject: &str,
    ) -> RequirementStatus {
        requirement(report, kind, subject).status
    }

    #[test]
    fn a_fully_satisfied_bundle_is_compatible() {
        let (_directory, bundle) = app_named("Compatible.app");
        write_app(
            &bundle,
            &x86_64_app(&[dylib_command(LIBZ), dylib_command(APPKIT)]),
            Some("10.13"),
        );
        let profile = profile_of(&[("AppKit", "supported")], &[(LIBZ, "supported")]);

        let report = report_for(&bundle, "x86_64", &profile);

        assert_eq!(report.verdict, Verdict::Compatible);
        assert_eq!(
            status_of(&report, RequirementKind::Architecture, "x86_64"),
            RequirementStatus::Satisfied
        );
        assert_eq!(
            status_of(&report, RequirementKind::DynamicLibrary, LIBZ),
            RequirementStatus::Satisfied
        );
        assert_eq!(
            status_of(&report, RequirementKind::Framework, "AppKit"),
            RequirementStatus::Satisfied
        );
        let minimum = requirement(&report, RequirementKind::MinimumOs, "10.13");
        assert_eq!(minimum.status, RequirementStatus::Satisfied);
        assert!(
            minimum.detail.contains("11.0"),
            "the detail must name the profile baseline: {}",
            minimum.detail
        );
    }

    #[test]
    fn an_unsupported_framework_makes_the_bundle_not_compatible() {
        let (_directory, bundle) = app_named("Unsupported.app");
        write_app(
            &bundle,
            &x86_64_app(&[dylib_command(APPKIT)]),
            Some("10.13"),
        );
        let profile = profile_of(&[("AppKit", "unsupported")], &[]);

        let report = report_for(&bundle, "x86_64", &profile);

        assert_eq!(report.verdict, Verdict::NotCompatible);
        let framework = requirement(&report, RequirementKind::Framework, "AppKit");
        assert_eq!(framework.status, RequirementStatus::Unsatisfied);
        assert!(
            framework.detail.contains("unsupported"),
            "the detail must name the profile status: {}",
            framework.detail
        );
        assert!(
            framework.detail.contains(APPKIT),
            "the detail must name the evidence: {}",
            framework.detail
        );
    }

    #[test]
    fn a_framework_the_profile_does_not_judge_is_unknown() {
        let (_directory, bundle) = app_named("Unknown.app");
        write_app(
            &bundle,
            &x86_64_app(&[dylib_command(APPKIT)]),
            Some("10.13"),
        );
        let profile = profile_of(&[], &[]);

        let report = report_for(&bundle, "x86_64", &profile);

        assert_eq!(report.verdict, Verdict::Unknown);
        let framework = requirement(&report, RequirementKind::Framework, "AppKit");
        assert_eq!(framework.status, RequirementStatus::Unknown);
        assert!(
            framework.detail.contains("AppKit"),
            "the detail must name the subject: {}",
            framework.detail
        );
    }

    #[test]
    fn a_partial_framework_is_advisory_and_the_verdict_is_partial() {
        let (_directory, bundle) = app_named("Partial.app");
        write_app(
            &bundle,
            &x86_64_app(&[dylib_command(APPKIT)]),
            Some("10.13"),
        );
        let profile = profile_of(&[("AppKit", "partial")], &[]);

        let report = report_for(&bundle, "x86_64", &profile);

        assert_eq!(report.verdict, Verdict::Partial);
        assert_eq!(
            status_of(&report, RequirementKind::Framework, "AppKit"),
            RequirementStatus::Advisory
        );
    }

    #[test]
    fn a_framework_marked_unknown_in_the_profile_is_unknown_with_its_note() {
        let (_directory, bundle) = app_named("NotVerified.app");
        write_app(
            &bundle,
            &x86_64_app(&[dylib_command(APPKIT)]),
            Some("10.13"),
        );
        let profile = Profile::parse(
            "profile_version = \"test\"\ndarwin_version = \"20.6.0\"\nmacos_version = \"11.0\"\n\
             \n[[frameworks]]\nname = \"AppKit\"\nstatus = \"unknown\"\n\
             note = \"sin verificación registrada\"\n",
        )
        .expect("the fixture profile is valid");

        let report = report_for(&bundle, "x86_64", &profile);

        assert_eq!(report.verdict, Verdict::Unknown);
        let framework = requirement(&report, RequirementKind::Framework, "AppKit");
        assert_eq!(framework.status, RequirementStatus::Unknown);
        assert!(
            framework.detail.contains("sin verificación registrada"),
            "the detail must carry the profile's note: {}",
            framework.detail
        );
    }

    #[test]
    fn an_embedded_framework_is_advisory_and_never_judged_by_the_profile() {
        let (_directory, bundle) = app_named("Embedded.app");
        write_app(
            &bundle,
            &x86_64_app(&[dylib_command("@rpath/Plugin.framework/Versions/A/Plugin")]),
            Some("10.13"),
        );
        write_file(
            &bundle.join("Contents/Frameworks/Plugin.framework/Plugin"),
            b"framework binary",
        );
        // The profile would doom a system copy of `Plugin`; the bundle ships its
        // own, so the profile must not decide it.
        let profile = profile_of(&[("Plugin", "unsupported")], &[]);

        let report = report_for(&bundle, "x86_64", &profile);

        assert_eq!(report.verdict, Verdict::Partial);
        let framework = requirement(&report, RequirementKind::Framework, "Plugin");
        assert_eq!(framework.status, RequirementStatus::Advisory);
        assert!(
            framework.detail.contains("ships inside the bundle"),
            "the detail must say why the profile does not decide it: {}",
            framework.detail
        );
        assert_eq!(
            report
                .requirements
                .iter()
                .filter(|requirement| requirement.kind == RequirementKind::DynamicLibrary)
                .count(),
            0,
            "a framework path is judged once, as a framework: {:?}",
            report.requirements
        );
    }

    #[test]
    fn a_minimum_os_above_the_profile_is_not_compatible() {
        let (_directory, bundle) = app_named("Newer.app");
        write_app(&bundle, &x86_64_app(&[]), Some("12.0"));
        let profile = profile_of(&[], &[]);

        let report = report_for(&bundle, "x86_64", &profile);

        assert_eq!(report.verdict, Verdict::NotCompatible);
        let minimum = requirement(&report, RequirementKind::MinimumOs, "12.0");
        assert_eq!(minimum.status, RequirementStatus::Unsatisfied);
        assert!(
            minimum.detail.contains("11.0"),
            "the detail must name the profile baseline: {}",
            minimum.detail
        );
    }

    #[test]
    fn a_minimum_os_below_the_profile_is_satisfied() {
        let (_directory, bundle) = app_named("Older.app");
        write_app(&bundle, &x86_64_app(&[]), Some("10.13"));
        let profile = profile_of(&[], &[]);

        let report = report_for(&bundle, "x86_64", &profile);

        assert_eq!(
            status_of(&report, RequirementKind::MinimumOs, "10.13"),
            RequirementStatus::Satisfied
        );
        assert_eq!(report.verdict, Verdict::Compatible);
    }

    #[test]
    fn a_minimum_os_equal_to_the_profile_is_satisfied() {
        let (_directory, bundle) = app_named("Exact.app");
        write_app(&bundle, &x86_64_app(&[]), Some("11.0"));

        let report = report_for(&bundle, "x86_64", &profile_of(&[], &[]));

        assert_eq!(
            status_of(&report, RequirementKind::MinimumOs, "11.0"),
            RequirementStatus::Satisfied
        );
    }

    #[test]
    fn a_minimum_os_compared_numerically_beats_the_same_text_order() {
        // 10.9 is older than 10.15 as a number, but sorts after it as text.
        let (_directory, bundle) = app_named("Numeric.app");
        write_app(&bundle, &x86_64_app(&[]), Some("10.9"));
        let profile = Profile::parse(
            "profile_version = \"test\"\ndarwin_version = \"20.6.0\"\nmacos_version = \"10.15\"\n",
        )
        .expect("the fixture profile is valid");

        let report = report_for(&bundle, "x86_64", &profile);

        assert_eq!(
            status_of(&report, RequirementKind::MinimumOs, "10.9"),
            RequirementStatus::Satisfied
        );
    }

    #[test]
    fn no_declared_minimum_os_is_unknown() {
        let (_directory, bundle) = app_named("Silent.app");
        write_app(&bundle, &x86_64_app(&[]), None);

        let report = report_for(&bundle, "x86_64", &profile_of(&[], &[]));

        assert_eq!(report.verdict, Verdict::Unknown);
        let minimum = requirement(&report, RequirementKind::MinimumOs, "none");
        assert_eq!(minimum.status, RequirementStatus::Unknown);
        assert!(
            minimum.detail.contains("declare"),
            "the detail must say nothing was declared: {}",
            minimum.detail
        );
    }

    #[test]
    fn the_load_command_minimum_os_stands_in_for_an_absent_plist_key() {
        let (_directory, bundle) = app_named("LoadCommand.app");
        write_app(
            &bundle,
            &x86_64_app(&[build_version_command(
                PLATFORM_MACOS,
                packed_version(12, 0, 0),
            )]),
            None,
        );

        let report = report_for(&bundle, "x86_64", &profile_of(&[], &[]));

        let minimum = requirement(&report, RequirementKind::MinimumOs, "12.0.0");
        assert_eq!(minimum.status, RequirementStatus::Unsatisfied);
        assert_eq!(report.verdict, Verdict::NotCompatible);
    }

    #[test]
    fn an_unparsable_minimum_os_is_unknown_and_names_the_value() {
        let (_directory, bundle) = app_named("Vague.app");
        write_app(&bundle, &x86_64_app(&[]), Some("Big Sur"));

        let report = report_for(&bundle, "x86_64", &profile_of(&[], &[]));

        let minimum = requirement(&report, RequirementKind::MinimumOs, "Big Sur");
        assert_eq!(minimum.status, RequirementStatus::Unknown);
        assert!(
            minimum.detail.contains("Big Sur"),
            "the detail must name the value: {}",
            minimum.detail
        );
        assert_eq!(report.verdict, Verdict::Unknown);
    }

    #[test]
    fn an_architecture_mismatch_is_not_compatible() {
        let (_directory, bundle) = app_named("Rosetta.app");
        write_app(&bundle, &thin_macho(CPU_TYPE_ARM64, &[]), Some("10.13"));

        let report = report_for(&bundle, "x86_64", &profile_of(&[], &[]));

        assert_eq!(report.verdict, Verdict::NotCompatible);
        let architecture = &report.requirements[0];
        assert_eq!(architecture.kind, RequirementKind::Architecture);
        assert_eq!(architecture.subject, "aarch64");
        assert_eq!(architecture.status, RequirementStatus::Unsatisfied);
        assert!(
            architecture.detail.contains("x86_64"),
            "the detail must name the host: {}",
            architecture.detail
        );
    }

    #[test]
    fn arm64_maps_to_the_aarch64_host_name() {
        let (_directory, bundle) = app_named("AppleSilicon.app");
        write_app(&bundle, &thin_macho(CPU_TYPE_ARM64, &[]), Some("10.13"));

        let report = report_for(&bundle, "aarch64", &profile_of(&[], &[]));

        assert_eq!(
            status_of(&report, RequirementKind::Architecture, "aarch64"),
            RequirementStatus::Satisfied
        );
        assert_eq!(report.verdict, Verdict::Compatible);
    }

    #[test]
    fn a_universal_binary_with_one_matching_slice_is_satisfied() {
        let (_directory, bundle) = app_named("Universal.app");
        write_app(
            &bundle,
            &fat_macho(&[(CPU_TYPE_ARM64, Vec::new()), (CPU_TYPE_X86_64, Vec::new())]),
            Some("10.13"),
        );

        let report = report_for(&bundle, "x86_64", &profile_of(&[], &[]));

        let architectures: Vec<&Requirement> = report
            .requirements
            .iter()
            .filter(|requirement| requirement.kind == RequirementKind::Architecture)
            .collect();
        assert_eq!(
            architectures.len(),
            1,
            "only the slice the host can run is reported: {architectures:?}"
        );
        assert_eq!(architectures[0].subject, "x86_64");
        assert_eq!(architectures[0].status, RequirementStatus::Satisfied);
        assert_eq!(report.verdict, Verdict::Compatible);
    }

    #[test]
    fn a_bundle_with_only_in_bundle_dylibs_is_partial() {
        let (_directory, bundle) = app_named("SelfContained.app");
        write_app(
            &bundle,
            &x86_64_app(&[
                dylib_command("@executable_path/../Frameworks/libsupport.dylib"),
                dylib_command("@loader_path/libloader.dylib"),
            ]),
            Some("10.13"),
        );

        let report = report_for(&bundle, "x86_64", &profile_of(&[], &[]));

        assert_eq!(report.verdict, Verdict::Partial);
        for subject in [
            "@executable_path/../Frameworks/libsupport.dylib",
            "@loader_path/libloader.dylib",
        ] {
            let dylib = requirement(&report, RequirementKind::DynamicLibrary, subject);
            assert_eq!(dylib.status, RequirementStatus::Advisory);
            assert!(
                dylib.detail.contains("ships inside the bundle"),
                "the detail must say why: {}",
                dylib.detail
            );
        }
    }

    #[test]
    fn a_dylib_absent_from_the_profile_is_unknown_and_named() {
        let (_directory, bundle) = app_named("OddDylib.app");
        write_app(
            &bundle,
            &x86_64_app(&[dylib_command("/usr/lib/libwho.dylib")]),
            Some("10.13"),
        );

        let report = report_for(&bundle, "x86_64", &profile_of(&[], &[]));

        let dylib = requirement(
            &report,
            RequirementKind::DynamicLibrary,
            "/usr/lib/libwho.dylib",
        );
        assert_eq!(dylib.status, RequirementStatus::Unknown);
        assert!(
            dylib.detail.contains("/usr/lib/libwho.dylib"),
            "the detail must name the path: {}",
            dylib.detail
        );
        assert_eq!(report.verdict, Verdict::Unknown);
    }

    #[test]
    fn an_in_bundle_copy_is_not_matched_against_the_system_library() {
        let (_directory, bundle) = app_named("Shadowed.app");
        write_app(
            &bundle,
            &x86_64_app(&[dylib_command("@rpath/libz.1.dylib")]),
            Some("10.13"),
        );
        // The profile judges the system copy as supported; the bundle's own copy
        // is a different subject and stays advisory.
        let profile = profile_of(&[], &[(LIBZ, "supported")]);

        let report = report_for(&bundle, "x86_64", &profile);

        let dylib = requirement(
            &report,
            RequirementKind::DynamicLibrary,
            "@rpath/libz.1.dylib",
        );
        assert_eq!(dylib.status, RequirementStatus::Advisory);
        assert_eq!(report.verdict, Verdict::Partial);
    }

    #[test]
    fn an_unsupported_dylib_is_not_compatible() {
        let (_directory, bundle) = app_named("MissingLib.app");
        write_app(&bundle, &x86_64_app(&[dylib_command(LIBZ)]), Some("10.13"));

        let report = report_for(
            &bundle,
            "x86_64",
            &profile_of(&[], &[(LIBZ, "unsupported")]),
        );

        assert_eq!(
            status_of(&report, RequirementKind::DynamicLibrary, LIBZ),
            RequirementStatus::Unsatisfied
        );
        assert_eq!(report.verdict, Verdict::NotCompatible);
    }

    #[test]
    fn requirements_are_reported_in_a_stable_order() {
        let (_directory, bundle) = app_named("Ordered.app");
        write_app(
            &bundle,
            &x86_64_app(&[dylib_command(APPKIT), dylib_command(LIBZ)]),
            Some("10.13"),
        );
        let profile = profile_of(&[("AppKit", "supported")], &[(LIBZ, "supported")]);

        let report = report_for(&bundle, "x86_64", &profile);
        let kinds: Vec<RequirementKind> = report
            .requirements
            .iter()
            .map(|requirement| requirement.kind)
            .collect();

        let mut expected = kinds.clone();
        expected.sort_unstable();
        assert_eq!(kinds, expected, "requirements follow their kind order");
    }

    #[test]
    fn the_verdict_follows_the_worst_requirement() {
        let requirement = |status| Requirement {
            kind: RequirementKind::Framework,
            subject: String::from("AppKit"),
            status,
            detail: String::from("a detail"),
        };

        assert_eq!(verdict(&[]), Verdict::Unknown);
        assert_eq!(
            verdict(&[requirement(RequirementStatus::Satisfied)]),
            Verdict::Compatible
        );
        assert_eq!(
            verdict(&[requirement(RequirementStatus::Advisory)]),
            Verdict::Partial
        );
        assert_eq!(
            verdict(&[requirement(RequirementStatus::Unknown)]),
            Verdict::Unknown
        );
        assert_eq!(
            verdict(&[requirement(RequirementStatus::Unsatisfied)]),
            Verdict::NotCompatible
        );
        assert_eq!(
            verdict(&[
                requirement(RequirementStatus::Unsatisfied),
                requirement(RequirementStatus::Unknown),
                requirement(RequirementStatus::Advisory),
            ]),
            Verdict::NotCompatible
        );
        assert_eq!(
            verdict(&[
                requirement(RequirementStatus::Unknown),
                requirement(RequirementStatus::Advisory),
            ]),
            Verdict::Unknown
        );
        assert_eq!(
            verdict(&[
                requirement(RequirementStatus::Satisfied),
                requirement(RequirementStatus::Advisory),
            ]),
            Verdict::Partial
        );
    }

    #[test]
    fn the_report_serializes_with_the_documented_keys_and_no_nulls() {
        let (_directory, bundle) = app_named("Serialized.app");
        write_app(
            &bundle,
            &x86_64_app(&[dylib_command(LIBZ), dylib_command(APPKIT)]),
            Some("10.13"),
        );
        let profile = profile_of(&[("AppKit", "partial")], &[(LIBZ, "supported")]);

        let report = report_for(&bundle, "x86_64", &profile);
        let json = serde_json::to_value(&report).expect("the report serializes");

        let object = json.as_object().expect("the report is a JSON object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "analyser_version",
                "profile_version",
                "requirements",
                "schema_version",
                "verdict",
            ]
        );
        assert_eq!(json["schema_version"], SCHEMA_VERSION);
        assert_eq!(json["analyser_version"], ANALYSER_VERSION);
        assert_eq!(json["profile_version"], "test");
        assert_eq!(json["verdict"], "partial");
        assert!(
            json.to_string().find("null").is_none(),
            "no field may serialize as null: {json}"
        );

        let requirements = json["requirements"]
            .as_array()
            .expect("requirements is an array");
        assert!(!requirements.is_empty());
        for requirement in requirements {
            let object = requirement.as_object().expect("a requirement is an object");
            let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
            keys.sort_unstable();
            assert_eq!(keys, vec!["detail", "kind", "status", "subject"]);
        }
        assert_eq!(requirements[0]["kind"], "architecture");
        assert_eq!(requirements[0]["status"], "satisfied");
    }

    #[test]
    fn enum_machine_names_are_snake_case() {
        for (value, expected) in [
            (Verdict::Compatible, "\"compatible\""),
            (Verdict::Partial, "\"partial\""),
            (Verdict::NotCompatible, "\"not_compatible\""),
            (Verdict::Unknown, "\"unknown\""),
        ] {
            assert_eq!(
                serde_json::to_string(&value).expect("the value serializes"),
                expected
            );
        }
        for (value, expected) in [
            (RequirementKind::MinimumOs, "\"minimum_os\""),
            (RequirementKind::DynamicLibrary, "\"dynamic_library\""),
        ] {
            assert_eq!(
                serde_json::to_string(&value).expect("the value serializes"),
                expected
            );
        }
        assert_eq!(
            serde_json::to_string(&RequirementStatus::Unsatisfied).expect("the value serializes"),
            "\"unsatisfied\""
        );
    }

    #[test]
    fn the_embedded_profile_is_used_by_default_and_its_version_is_reported() {
        let (_directory, bundle) = app_named("Embedded.app");
        write_app(
            &bundle,
            &x86_64_app(&[dylib_command(LIBZ), dylib_command(APPKIT)]),
            Some("10.13"),
        );

        let report = analyze(&AnalysisRequest {
            bundle: &bundle,
            host_architecture: "x86_64",
        })
        .expect("the bundle is analysable");

        assert_eq!(report.analyser_version, ANALYSER_VERSION);
        assert_eq!(
            report.profile_version,
            crate::profile::Profile::embedded()
                .expect("the embedded profile is valid")
                .profile_version
        );
        // The embedded profile records nothing as verified, so the honest
        // answer is unknown, never compatible.
        assert_eq!(report.verdict, Verdict::Unknown);
    }

    #[test]
    fn an_uninspectable_bundle_is_an_error_that_names_the_path() {
        let directory = tempfile::tempdir().expect("create the temporary directory");
        let path = directory.path().join("Absent.app");

        let error = analyze(&AnalysisRequest {
            bundle: &path,
            host_architecture: "x86_64",
        })
        .expect_err("a missing bundle cannot be analysed");

        assert!(
            matches!(error, Error::Bundle { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            error.to_string().contains(&path.display().to_string()),
            "the message must name the path: {error}"
        );
    }

    #[test]
    fn an_executable_with_unreadable_load_commands_is_reported_separately() {
        let (_directory, bundle) = app_named("Corrupt.app");
        let mut bytes = x86_64_app(&[]);
        // Announce one command that the header does not carry.
        bytes[20..24].copy_from_slice(&64u32.to_le_bytes());
        write_app(&bundle, &bytes, Some("10.13"));

        let error = analyze(&AnalysisRequest {
            bundle: &bundle,
            host_architecture: "x86_64",
        })
        .expect_err("the load commands are not in the file");

        assert!(
            matches!(error, Error::Requirements { .. }),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn the_analysis_only_reads_the_bundle() {
        let (_directory, bundle) = app_named("ReadOnly.app");
        write_app(
            &bundle,
            &x86_64_app(&[dylib_command(LIBZ), dylib_command(APPKIT)]),
            Some("10.13"),
        );
        create_dir(&bundle.join("Contents/Resources"));

        let report = report_for(&bundle, "x86_64", &profile_of(&[], &[(LIBZ, "supported")]));

        assert!(
            report
                .requirements
                .iter()
                .any(|requirement| requirement.status == RequirementStatus::Satisfied)
        );
        assert!(
            bundle.join("Contents/Resources").is_dir(),
            "the analysis writes nothing into the bundle"
        );
    }
}
