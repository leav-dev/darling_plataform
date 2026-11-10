//! Bundle Inspector.
//!
//! Responsibility: read a `.app` bundle without running it and extract
//! `CFBundleIdentifier`, name, display name, version, executable, icon,
//! `LSMinimumSystemVersion`, Mach-O architectures and the referenced frameworks
//! and resources, plus the dynamic libraries and declared minimum macOS version
//! its load commands name.
//!
//! Not responsible for: executing the binary, installing anything, or deciding
//! compatibility. Every path declared by the bundle must be proven to stay
//! inside the bundle, and the declared executable must be shown to exist before
//! its metadata is trusted. `Info.plist` and Mach-O headers and load commands
//! are untrusted input.
//!
//! # Reporting rules
//!
//! * `Contents/Info.plist` may be an XML or a binary property list; anything
//!   that is not a dictionary with a usable `CFBundleIdentifier` and
//!   `CFBundleExecutable` is rejected, and the error names the file.
//! * Only `CFBundleIdentifier`, `CFBundleName` (with the bundle directory name
//!   as the macOS fallback), `CFBundleDisplayName`, `CFBundleVersion`,
//!   `CFBundleExecutable`, `CFBundleIconFile` and `LSMinimumSystemVersion` are
//!   read. Keys this crate does not report are ignored, including asset-catalog
//!   icons.
//! * Every declared path is validated lexically (no absolute value, no `..`) and
//!   physically (no symlink whose canonical target leaves the bundle) before it
//!   is reported.
//! * A declared key of the wrong type is rejected instead of ignored, so a
//!   report never hides a malformed value.
//! * `path` and `executable_path` are the paths the caller and the bundle use;
//!   containment is proven against the canonical bundle, but the reported paths
//!   are not canonicalised, so the operator sees the bundle they named.
//! * Frameworks are the direct `Contents/Frameworks/*.framework` entries and
//!   resources are the direct children of `Contents/Resources`; both are
//!   bundle-relative, sorted and de-duplicated, and neither is recursive.
//! * [`macho::requirements`] reports what the executable's load commands declare
//!   — its dylibs and its minimum macOS version — verbatim and sorted; it never
//!   resolves an install name and never decides whether anything can be
//!   satisfied.
//! * Enums serialize as `snake_case` machine names, and an absent optional value
//!   is omitted instead of serialized as `null`, matching the hardware report:
//!   one rule for every JSON document this platform emits.

pub mod error;
pub mod macho;
mod paths;

#[cfg(test)]
mod test_support;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use plist::{Dictionary, Value};
use serde::Serialize;

pub use error::{Error, Result};
pub use macho::{Architecture, BinaryRequirements, requirements};

/// Directory that holds every part of a bundle. Each path below is joined one
/// component at a time, so the separators always match the host.
const CONTENTS: &str = "Contents";
/// Name of the metadata file inside [`CONTENTS`].
const INFO_PLIST: &str = "Info.plist";
/// Directory inside [`CONTENTS`] that holds the declared executable.
const MACOS: &str = "MacOS";
/// Directory inside [`CONTENTS`] the declared icon is resolved against.
const RESOURCES: &str = "Resources";
/// Directory inside [`CONTENTS`] that holds embedded frameworks.
const FRAMEWORKS: &str = "Frameworks";
/// Suffix that marks a framework directory.
const FRAMEWORK_SUFFIX: &str = ".framework";

/// Everything the inspector could prove about one `.app` bundle.
///
/// The fields are the bundle's own declarations plus what its file tree and its
/// executable header establish; nothing is guessed except the documented
/// `CFBundleName` fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BundleMetadata {
    /// The bundle directory as it was passed to [`inspect`], not canonicalised.
    pub path: PathBuf,
    /// `CFBundleIdentifier`. It is never empty.
    pub bundle_id: String,
    /// `CFBundleName`, or the bundle directory name without `.app` when the
    /// bundle declares no name, which is the fallback macOS itself uses.
    pub name: String,
    /// `CFBundleDisplayName`, the name shown to users, when the bundle declares
    /// one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// `CFBundleVersion`, Apple's build version (not the marketing version,
    /// which lives in `CFBundleShortVersionString`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// `CFBundleExecutable` as the bundle declares it.
    pub executable: String,
    /// The declared executable resolved inside the bundle. It exists and is a
    /// regular file, and its canonical path stays inside the bundle.
    pub executable_path: PathBuf,
    /// `CFBundleIconFile` as the bundle declares it, relative to
    /// `Contents/Resources`. A declared icon that is missing is still reported:
    /// the installer decides whether that is fatal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_file: Option<String>,
    /// `LSMinimumSystemVersion`, the oldest macOS the bundle claims to support.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_system_version: Option<String>,
    /// Architectures of the executable, in Mach-O slice order and de-duplicated.
    pub architectures: Vec<Architecture>,
    /// Embedded frameworks, relative to the bundle, sorted. Their contents are
    /// not inspected.
    pub frameworks: Vec<String>,
    /// Direct children of `Contents/Resources`, relative to the bundle, sorted.
    pub resources: Vec<String>,
}

/// Reads `bundle` without executing it and reports its metadata.
///
/// The bundle must be a directory holding `Contents/Info.plist` and the
/// executable that plist declares. Every declared path is validated against the
/// bundle before it is reported, so a returned [`BundleMetadata`] only contains
/// paths that cannot reach outside the bundle.
pub fn inspect(bundle: &Path) -> Result<BundleMetadata> {
    let metadata = std::fs::metadata(bundle).map_err(|source| Error::Io {
        path: bundle.to_path_buf(),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(Error::BundleNotDirectory {
            path: bundle.to_path_buf(),
        });
    }
    let root = bundle.canonicalize().map_err(|source| Error::Io {
        path: bundle.to_path_buf(),
        source,
    })?;

    let plist_path = bundle.join(CONTENTS).join(INFO_PLIST);
    let dictionary = read_info_plist(bundle, &plist_path)?;

    let bundle_id = required_string(&plist_path, &dictionary, "CFBundleIdentifier")?;
    let executable = required_string(&plist_path, &dictionary, "CFBundleExecutable")?;
    let name = match optional_string(&plist_path, &dictionary, "CFBundleName")? {
        Some(name) => name,
        None => bundle_name(bundle)?,
    };
    let display_name = optional_string(&plist_path, &dictionary, "CFBundleDisplayName")?;
    let version = optional_string(&plist_path, &dictionary, "CFBundleVersion")?;
    let icon_file = optional_string(&plist_path, &dictionary, "CFBundleIconFile")?;
    let minimum_system_version =
        optional_string(&plist_path, &dictionary, "LSMinimumSystemVersion")?;

    let executable_path = paths::declared(
        bundle,
        &[CONTENTS, MACOS],
        "CFBundleExecutable",
        &executable,
    )?;
    prove_executable(bundle, &root, &executable, &executable_path)?;
    if let Some(icon) = &icon_file {
        prove_icon(bundle, &root, icon)?;
    }

    let architectures = macho::architectures(&executable_path)?;
    let frameworks = list_contents(&root, bundle, FRAMEWORKS, "framework", FRAMEWORK_SUFFIX)?;
    let resources = list_contents(&root, bundle, RESOURCES, "resource", "")?;

    Ok(BundleMetadata {
        path: bundle.to_path_buf(),
        bundle_id,
        name,
        display_name,
        version,
        executable,
        executable_path,
        icon_file,
        minimum_system_version,
        architectures,
        frameworks,
        resources,
    })
}

/// Reads and structurally validates `Contents/Info.plist`.
fn read_info_plist(bundle: &Path, path: &Path) -> Result<Dictionary> {
    let value = Value::from_file(path).map_err(|source| {
        if source
            .as_io()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            Error::InfoPlistMissing {
                bundle: bundle.to_path_buf(),
            }
        } else {
            Error::InfoPlistUnreadable {
                path: path.to_path_buf(),
                source,
            }
        }
    })?;
    let found = describe(&value);
    value
        .into_dictionary()
        .ok_or_else(|| Error::InfoPlistNotDictionary {
            path: path.to_path_buf(),
            found,
        })
}

/// Reads a key the bundle must declare, rejecting an empty or non-string value.
fn required_string(path: &Path, dictionary: &Dictionary, key: &'static str) -> Result<String> {
    let value = dictionary.get(key).ok_or_else(|| Error::MissingKey {
        path: path.to_path_buf(),
        key,
    })?;
    let Some(text) = value.as_string() else {
        return Err(Error::InvalidKey {
            path: path.to_path_buf(),
            key,
            reason: "it is not a <string> value; declare it as a <string>",
        });
    };
    if text.trim().is_empty() {
        return Err(Error::InvalidKey {
            path: path.to_path_buf(),
            key,
            reason: "it is empty; declare a non-empty value",
        });
    }
    Ok(text.to_string())
}

/// Reads a key the bundle may omit. An absent key, or one whose value is blank,
/// is reported as absent so the caller never renders an empty field.
fn optional_string(
    path: &Path,
    dictionary: &Dictionary,
    key: &'static str,
) -> Result<Option<String>> {
    let Some(value) = dictionary.get(key) else {
        return Ok(None);
    };
    let Some(text) = value.as_string() else {
        return Err(Error::InvalidKey {
            path: path.to_path_buf(),
            key,
            reason: "it is not a <string> value; declare it as a <string>",
        });
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(text.to_string()))
}

/// The bundle directory's own name without `.app`, the documented fallback for a
/// bundle that declares no `CFBundleName`.
fn bundle_name(bundle: &Path) -> Result<String> {
    bundle
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::BundleWithoutName {
            path: bundle.to_path_buf(),
        })
}

/// Proves that the declared executable exists, is a regular file and stays
/// inside the bundle once symlinks are resolved.
fn prove_executable(bundle: &Path, root: &Path, executable: &str, path: &Path) -> Result<()> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return Err(Error::ExecutableNotRegularFile {
                bundle: bundle.to_path_buf(),
                value: executable.to_string(),
                path: path.to_path_buf(),
            });
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::ExecutableMissing {
                bundle: bundle.to_path_buf(),
                value: executable.to_string(),
                path: path.to_path_buf(),
            });
        }
        Err(source) => {
            return Err(Error::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    }
    paths::verify(root, bundle, "CFBundleExecutable", executable, path)
}

/// Proves that a declared icon path stays inside the bundle. A missing icon is
/// not an error here; the installer decides whether it needs the file.
fn prove_icon(bundle: &Path, root: &Path, icon: &str) -> Result<()> {
    let path = paths::declared(bundle, &[CONTENTS, RESOURCES], "CFBundleIconFile", icon)?;
    match std::fs::symlink_metadata(&path) {
        Ok(_) => paths::verify(root, bundle, "CFBundleIconFile", icon, &path),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::Io { path, source }),
    }
}

/// Lists the direct children of `directory`, skipping names that do not end with
/// `suffix`, verifies each one stays inside the bundle, and returns them sorted
/// and de-duplicated. A directory that does not exist contributes nothing.
///
/// Non-UTF-8 entry names cannot appear in the report, so they are skipped rather
/// than mangled; a bundle that ships one is caught when the installer copies it.
fn list_contents(
    root: &Path,
    bundle: &Path,
    directory: &str,
    role: &'static str,
    suffix: &str,
) -> Result<Vec<String>> {
    let path = bundle.join(CONTENTS).join(directory);
    let entries = match std::fs::read_dir(&path) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(Error::Io { path, source }),
    };
    let mut found = BTreeSet::new();
    for entry in entries {
        let entry = entry.map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !name.ends_with(suffix) {
            continue;
        }
        // Reported paths are bundle-relative and always use '/', which is what
        // an Info.plist and the platform's own reports use.
        let relative = format!("{CONTENTS}/{directory}/{name}");
        paths::verify(root, bundle, role, &relative, &entry.path())?;
        found.insert(relative);
    }
    Ok(found.into_iter().collect())
}

/// Names a property list root by kind, so the error explains what was found.
fn describe(value: &Value) -> &'static str {
    match value {
        Value::Array(_) => "an array",
        Value::Dictionary(_) => "a dictionary",
        Value::Boolean(_) => "a boolean",
        Value::Data(_) => "a data blob",
        Value::Date(_) => "a date",
        Value::Real(_) => "a real number",
        Value::Integer(_) => "an integer",
        Value::String(_) => "a string",
        Value::Uid(_) => "a UID",
        _ => "an unrecognised value",
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::test_support::{
        CPU_TYPE_X86_64, create_dir, thin_macho, write_binary_plist, write_file, write_xml_plist,
    };

    /// Every key this crate reports, as a real `Info.plist` would declare them.
    const FULL_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>com.example.example</string>
	<key>CFBundleName</key>
	<string>Example</string>
	<key>CFBundleDisplayName</key>
	<string>Example App</string>
	<key>CFBundleVersion</key>
	<string>1.2.3</string>
	<key>CFBundleExecutable</key>
	<string>Example</string>
	<key>CFBundleIconFile</key>
	<string>Example.icns</string>
	<key>LSMinimumSystemVersion</key>
	<string>10.13</string>
</dict>
</plist>
"#;

    /// An empty temporary directory holding a bundle directory called `name`.
    fn app_named(name: &str) -> (TempDir, PathBuf) {
        let directory = tempfile::tempdir().expect("create the temporary directory");
        let bundle = directory.path().join(name);
        create_dir(&bundle);
        (directory, bundle)
    }

    /// A plist whose required keys are usable and whose only optional key is the
    /// name; callers add the executable they need.
    fn minimal_plist(executable: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>com.example.minimal</string>
	<key>CFBundleName</key>
	<string>Minimal</string>
	<key>CFBundleExecutable</key>
	<string>{executable}</string>
</dict>
</plist>
"#
        )
    }

    /// [`FULL_PLIST`] without the key and its value, to exercise required keys.
    fn plist_without(key: &str) -> String {
        let full = FULL_PLIST;
        let start = full
            .find(&format!("\t<key>{key}</key>"))
            .unwrap_or_else(|| panic!("the full plist declares {key}"));
        let value_start = full[start..]
            .find('\n')
            .map(|offset| start + offset + 1)
            .expect("the key line ends");
        let value_end = full[value_start..]
            .find('\n')
            .map(|offset| value_start + offset + 1)
            .expect("the value line ends");
        let mut plist = String::from(&full[..start]);
        plist.push_str(&full[value_end..]);
        plist
    }

    #[test]
    fn extracts_the_declared_metadata_and_the_inferred_contents() {
        let (_directory, bundle) = app_named("Example.app");
        write_xml_plist(&bundle, FULL_PLIST);
        write_file(
            &bundle.join("Contents/MacOS/Example"),
            &thin_macho(CPU_TYPE_X86_64),
        );
        write_file(
            &bundle.join("Contents/Frameworks/Example.framework/Example"),
            b"framework binary",
        );
        write_file(&bundle.join("Contents/Resources/Example.icns"), b"icon");
        write_file(
            &bundle.join("Contents/Resources/en.lproj/Localizable.strings"),
            b"\"a\" = \"b\";",
        );

        let metadata = inspect(&bundle).expect("the bundle is well formed");

        assert_eq!(metadata.path, bundle);
        assert_eq!(metadata.bundle_id, "com.example.example");
        assert_eq!(metadata.name, "Example");
        assert_eq!(metadata.display_name.as_deref(), Some("Example App"));
        assert_eq!(metadata.version.as_deref(), Some("1.2.3"));
        assert_eq!(metadata.executable, "Example");
        assert_eq!(
            metadata.executable_path,
            bundle.join("Contents").join("MacOS").join("Example")
        );
        assert_eq!(metadata.icon_file.as_deref(), Some("Example.icns"));
        assert_eq!(metadata.minimum_system_version.as_deref(), Some("10.13"));
        assert_eq!(metadata.architectures, vec![Architecture::X86_64]);
        assert_eq!(
            metadata.frameworks,
            vec![String::from("Contents/Frameworks/Example.framework")]
        );
        assert_eq!(
            metadata.resources,
            vec![
                String::from("Contents/Resources/Example.icns"),
                String::from("Contents/Resources/en.lproj"),
            ]
        );
    }

    #[test]
    fn reads_a_binary_property_list() {
        let (_directory, bundle) = app_named("Binary.app");
        write_binary_plist(
            &bundle,
            &[
                ("CFBundleIdentifier", "com.example.binary"),
                ("CFBundleName", "Binary"),
                ("CFBundleExecutable", "Binary"),
                ("CFBundleVersion", "2.0"),
            ],
        );
        write_file(
            &bundle.join("Contents/MacOS/Binary"),
            &thin_macho(CPU_TYPE_X86_64),
        );

        let metadata = inspect(&bundle).expect("a binary property list is accepted");

        assert_eq!(metadata.bundle_id, "com.example.binary");
        assert_eq!(metadata.name, "Binary");
        assert_eq!(metadata.version.as_deref(), Some("2.0"));
        assert_eq!(metadata.architectures, vec![Architecture::X86_64]);
    }

    #[test]
    fn optional_keys_are_reported_as_absent() {
        let (_directory, bundle) = app_named("Minimal.app");
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>com.example.minimal</string>
	<key>CFBundleName</key>
	<string>Minimal</string>
	<key>CFBundleExecutable</key>
	<string>Minimal</string>
</dict>
</plist>
"#;
        write_xml_plist(&bundle, plist);
        write_file(
            &bundle.join("Contents/MacOS/Minimal"),
            &thin_macho(CPU_TYPE_X86_64),
        );

        let metadata = inspect(&bundle).expect("only two keys are required");

        assert_eq!(metadata.display_name, None);
        assert_eq!(metadata.version, None);
        assert_eq!(metadata.icon_file, None);
        assert_eq!(metadata.minimum_system_version, None);
        assert!(metadata.frameworks.is_empty());
        assert!(metadata.resources.is_empty());
    }

    #[test]
    fn the_name_falls_back_to_the_bundle_directory_name() {
        let (_directory, bundle) = app_named("Fallback.app");
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>com.example.fallback</string>
	<key>CFBundleExecutable</key>
	<string>Fallback</string>
</dict>
</plist>
"#;
        write_xml_plist(&bundle, plist);
        write_file(
            &bundle.join("Contents/MacOS/Fallback"),
            &thin_macho(CPU_TYPE_X86_64),
        );

        let metadata = inspect(&bundle).expect("CFBundleName is not required");

        assert_eq!(metadata.name, "Fallback");
    }

    #[test]
    fn missing_bundle_identifier_names_the_key_and_the_file() {
        let (_directory, bundle) = app_named("NoId.app");
        let plist = write_xml_plist(&bundle, &plist_without("CFBundleIdentifier"));
        write_file(
            &bundle.join("Contents/MacOS/Example"),
            &thin_macho(CPU_TYPE_X86_64),
        );

        let error = inspect(&bundle).expect_err("CFBundleIdentifier is required");
        let message = error.to_string();

        assert!(
            matches!(
                error,
                Error::MissingKey {
                    key: "CFBundleIdentifier",
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
        assert!(
            message.contains("CFBundleIdentifier"),
            "the message must name the key: {message}"
        );
        assert!(
            message.contains(&plist.display().to_string()),
            "the message must name the file: {message}"
        );
    }

    #[test]
    fn missing_executable_key_names_the_key_and_the_file() {
        let (_directory, bundle) = app_named("NoExecutable.app");
        let plist = write_xml_plist(&bundle, &plist_without("CFBundleExecutable"));

        let error = inspect(&bundle).expect_err("CFBundleExecutable is required");
        let message = error.to_string();

        assert!(
            matches!(
                error,
                Error::MissingKey {
                    key: "CFBundleExecutable",
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
        assert!(
            message.contains("CFBundleExecutable"),
            "the message must name the key: {message}"
        );
        assert!(
            message.contains(&plist.display().to_string()),
            "the message must name the file: {message}"
        );
    }

    #[test]
    fn an_empty_bundle_identifier_is_rejected() {
        let (_directory, bundle) = app_named("EmptyId.app");
        write_xml_plist(&bundle, &FULL_PLIST.replace("com.example.example", ""));
        write_file(
            &bundle.join("Contents/MacOS/Example"),
            &thin_macho(CPU_TYPE_X86_64),
        );

        let error = inspect(&bundle).expect_err("an empty identifier identifies nothing");
        let message = error.to_string();

        assert!(
            matches!(
                error,
                Error::InvalidKey {
                    key: "CFBundleIdentifier",
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
        assert!(
            message.contains("empty"),
            "the message must say why the value is unusable: {message}"
        );
    }

    #[test]
    fn a_malformed_property_list_names_the_file() {
        let (_directory, bundle) = app_named("Malformed.app");
        let plist = write_xml_plist(
            &bundle,
            "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>oops",
        );

        let error = inspect(&bundle).expect_err("a truncated property list is not metadata");
        let message = error.to_string();

        assert!(
            matches!(error, Error::InfoPlistUnreadable { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            message.contains(&plist.display().to_string()),
            "the message must name the file: {message}"
        );
    }

    #[test]
    fn a_property_list_that_is_not_a_dictionary_is_rejected() {
        let (_directory, bundle) = app_named("Array.app");
        let plist = write_xml_plist(
            &bundle,
            "<?xml version=\"1.0\"?><plist version=\"1.0\"><array/></plist>",
        );

        let error = inspect(&bundle).expect_err("bundle metadata is a dictionary");
        let message = error.to_string();

        assert!(
            matches!(
                error,
                Error::InfoPlistNotDictionary {
                    found: "an array",
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
        assert!(
            message.contains(&plist.display().to_string()),
            "the message must name the file: {message}"
        );
    }

    #[test]
    fn a_bundle_without_info_plist_is_rejected() {
        let (_directory, bundle) = app_named("Empty.app");

        let error = inspect(&bundle).expect_err("a bundle needs Contents/Info.plist");

        assert!(
            matches!(error, Error::InfoPlistMissing { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            error.to_string().contains("Info.plist"),
            "the message must name the file: {error}"
        );
    }

    #[test]
    fn a_file_that_is_not_a_directory_is_rejected() {
        let directory = tempfile::tempdir().expect("create the temporary directory");
        let path = directory.path().join("Example.app");
        write_file(&path, b"not a bundle");

        let error = inspect(&path).expect_err("a bundle is a directory");

        assert!(
            matches!(error, Error::BundleNotDirectory { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            error.to_string().contains(&path.display().to_string()),
            "the message must name the path: {error}"
        );
    }

    #[test]
    fn a_missing_executable_names_the_declared_value() {
        let (_directory, bundle) = app_named("Incomplete.app");
        write_xml_plist(&bundle, &minimal_plist("Missing"));

        let error = inspect(&bundle).expect_err("the declared executable must exist");
        let message = error.to_string();

        assert!(
            matches!(error, Error::ExecutableMissing { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            message.contains("Missing"),
            "the message must name the declared value: {message}"
        );
    }

    #[test]
    fn an_executable_that_is_not_a_regular_file_is_rejected() {
        let (_directory, bundle) = app_named("Directory.app");
        write_xml_plist(&bundle, &minimal_plist("Directory"));
        create_dir(&bundle.join("Contents/MacOS/Directory"));

        let error = inspect(&bundle).expect_err("an executable is a regular file");

        assert!(
            matches!(error, Error::ExecutableNotRegularFile { .. }),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn an_absolute_executable_path_is_rejected() {
        let (_directory, bundle) = app_named("Absolute.app");
        write_xml_plist(&bundle, &minimal_plist("/usr/bin/true"));

        let error = inspect(&bundle).expect_err("a bundle path is relative to the bundle");
        let message = error.to_string();

        assert!(
            matches!(
                error,
                Error::AbsolutePath {
                    role: "CFBundleExecutable",
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
        assert!(
            message.contains("/usr/bin/true"),
            "the message must name the offending value: {message}"
        );
    }

    #[test]
    fn a_parent_traversal_in_the_executable_path_is_rejected() {
        let (_directory, bundle) = app_named("Traversal.app");
        write_xml_plist(&bundle, &minimal_plist("../../../usr/bin/true"));

        let error = inspect(&bundle).expect_err("'..' leaves the bundle");
        let message = error.to_string();

        assert!(
            matches!(
                error,
                Error::ParentTraversal {
                    role: "CFBundleExecutable",
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
        assert!(
            message.contains("../../../usr/bin/true"),
            "the message must name the offending value: {message}"
        );
    }

    #[test]
    fn a_parent_traversal_in_the_icon_path_is_rejected() {
        let (_directory, bundle) = app_named("Icon.app");
        write_file(
            &bundle.join("Contents/MacOS/Example"),
            &thin_macho(CPU_TYPE_X86_64),
        );
        let plist = FULL_PLIST.replace("Example.icns", "../../../tmp/Example.icns");
        write_xml_plist(&bundle, &plist);

        let error = inspect(&bundle).expect_err("every declared path stays inside the bundle");

        assert!(
            matches!(
                error,
                Error::ParentTraversal {
                    role: "CFBundleIconFile",
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn an_executable_that_is_not_mach_o_names_the_path_and_the_magic() {
        let (_directory, bundle) = app_named("Script.app");
        write_xml_plist(&bundle, &minimal_plist("Script"));
        write_file(&bundle.join("Contents/MacOS/Script"), b"#!/bin/sh\n");

        let error = inspect(&bundle).expect_err("the executable must be a Mach-O binary");
        let message = error.to_string();

        assert!(
            matches!(error, Error::NotMachO { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            message.contains("Script"),
            "the message must name the path: {message}"
        );
        assert!(
            message.contains("23 21 2f 62"),
            "the message must report the observed magic: {message}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_executable_that_escapes_the_bundle_is_rejected() {
        let (_outside, outside) = app_named("Outside.app");
        let target = outside.join("Contents").join("MacOS").join("Escaped");
        write_file(&target, &thin_macho(CPU_TYPE_X86_64));

        let (_directory, bundle) = app_named("Linked.app");
        write_xml_plist(&bundle, &minimal_plist("Linked"));
        create_dir(&bundle.join("Contents/MacOS"));
        std::os::unix::fs::symlink(&target, bundle.join("Contents/MacOS/Linked"))
            .expect("create the escaping symlink");

        let error = inspect(&bundle).expect_err("a symlink may not smuggle a binary in");

        assert!(
            matches!(error, Error::EscapingSymlink { .. }),
            "unexpected error: {error:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_executable_inside_the_bundle_is_accepted() {
        let (_directory, bundle) = app_named("Linked.app");
        write_xml_plist(&bundle, &minimal_plist("Linked"));
        write_file(
            &bundle.join("Contents/MacOS/Real"),
            &thin_macho(CPU_TYPE_X86_64),
        );
        std::os::unix::fs::symlink("Real", bundle.join("Contents/MacOS/Linked"))
            .expect("create the internal symlink");

        let metadata = inspect(&bundle).expect("an internal symlink stays inside the bundle");

        assert_eq!(metadata.architectures, vec![Architecture::X86_64]);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_resource_that_escapes_the_bundle_is_rejected() {
        let directory = tempfile::tempdir().expect("create the temporary directory");
        let outside = directory.path().join("outside.txt");
        std::fs::write(&outside, b"outside").expect("write the outside file");

        let (_app, bundle) = app_named("Resource.app");
        write_xml_plist(&bundle, &minimal_plist("Resource"));
        write_file(
            &bundle.join("Contents/MacOS/Resource"),
            &thin_macho(CPU_TYPE_X86_64),
        );
        create_dir(&bundle.join("Contents/Resources"));
        std::os::unix::fs::symlink(&outside, bundle.join("Contents/Resources/escape.txt"))
            .expect("create the escaping symlink");

        let error = inspect(&bundle).expect_err("a resource may not leave the bundle");

        assert!(
            matches!(
                error,
                Error::EscapingSymlink {
                    role: "resource",
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn absent_optionals_are_omitted_like_the_rest_of_the_platform() {
        let (_directory, bundle) = app_named("Example.app");
        write_xml_plist(&bundle, &minimal_plist("Example"));
        write_file(
            &bundle.join("Contents/MacOS/Example"),
            &thin_macho(CPU_TYPE_X86_64),
        );

        let metadata = inspect(&bundle).expect("the bundle is well formed");
        let json = serde_json::to_value(&metadata).expect("the metadata serializes");

        for absent in [
            "display_name",
            "version",
            "icon_file",
            "minimum_system_version",
        ] {
            assert!(
                json.get(absent).is_none(),
                "{absent} must be omitted when absent, not serialized as null: {json}"
            );
        }
        assert_eq!(json["bundle_id"], "com.example.minimal");
    }

    #[test]
    fn the_metadata_serializes_with_the_reported_field_names() {
        let (_directory, bundle) = app_named("Example.app");
        write_xml_plist(&bundle, FULL_PLIST);
        write_file(
            &bundle.join("Contents/MacOS/Example"),
            &thin_macho(CPU_TYPE_X86_64),
        );

        let metadata = inspect(&bundle).expect("the bundle is well formed");
        let json = serde_json::to_value(&metadata).expect("the metadata serializes");

        for field in [
            "path",
            "bundle_id",
            "name",
            "display_name",
            "version",
            "executable",
            "executable_path",
            "icon_file",
            "minimum_system_version",
            "architectures",
            "frameworks",
            "resources",
        ] {
            assert!(
                json.get(field).is_some(),
                "the report must expose {field}: {json}"
            );
        }
        assert_eq!(json["bundle_id"], "com.example.example");
        assert_eq!(json["architectures"][0], "x86_64");
    }

    #[test]
    fn architectures_serialize_as_snake_case_machine_names() {
        assert_eq!(
            serde_json::to_string(&Architecture::X86_64).expect("serialize"),
            "\"x86_64\""
        );
        assert_eq!(
            serde_json::to_string(&Architecture::Arm64).expect("serialize"),
            "\"arm64\""
        );
        assert_eq!(
            serde_json::to_string(&Architecture::Arm).expect("serialize"),
            "\"arm\""
        );
        assert_eq!(
            serde_json::to_string(&Architecture::I386).expect("serialize"),
            "\"i386\""
        );
        assert_eq!(
            serde_json::to_string(&Architecture::PowerPc).expect("serialize"),
            "\"powerpc\""
        );
        assert_eq!(
            serde_json::to_string(&Architecture::Unknown(0x0200_000c)).expect("serialize"),
            "{\"unknown\":33554444}"
        );
    }

    #[test]
    fn declares_its_responsibility() {
        assert!(!env!("CARGO_PKG_DESCRIPTION").trim().is_empty());
    }
}
