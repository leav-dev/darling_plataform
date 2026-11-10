//! The rows the Application Library stores.
//!
//! Both records mirror their table one field per column, so a row that is read
//! back is the row that was written. Timestamps are opaque RFC 3339 UTC text
//! produced by [`crate::now_utc`] or by the caller: the library never invents
//! them, so a test can pin them.

use std::path::PathBuf;

use serde::Serialize;

/// One installed application, as the `applications` table stores it.
///
/// The record describes what was installed. It is not proof that the files are
/// still there: callers must check `bundle_path` and `executable_path` before
/// launching or updating.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApplicationRecord {
    /// Stable identifier of this row, chosen by the caller. Uninstalling and
    /// installing again at the same path produces a new id, which keeps the
    /// history of each installation separate.
    pub id: String,
    /// `CFBundleIdentifier`. Two installed versions of the same application
    /// share it, so it is indexed but not unique.
    pub bundle_id: String,
    /// The name shown to the user.
    pub name: String,
    /// The installed version. A bundle may declare none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Where the bundle lives, normally inside `<DPREFIX>/Applications`. The
    /// pair of bundle id and bundle path is unique.
    pub bundle_path: PathBuf,
    /// The executable inside the bundle that the launcher runs.
    pub executable_path: PathBuf,
    /// The icon the bundle declares, when it declares one. The catalogue stores
    /// the path, not the image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_path: Option<PathBuf>,
    /// The architecture the bundle is installed for, such as `x86_64`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub architecture: Option<String>,
    /// How the bundle got here, such as `bundle` or `zip`. The installer owns
    /// this vocabulary; the library stores it verbatim.
    pub installation_type: String,
    /// An RFC 3339 UTC timestamp supplied by the caller.
    pub installed_at: String,
    /// The installation state, such as `installed`. The installer owns this
    /// vocabulary; the library stores it verbatim.
    pub installation_status: String,
}

/// One installation attempt, as the `installations` table stores it.
///
/// A failed attempt is history too, so `application_id` is optional: an attempt
/// that never produced an application row has no owner to point at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstallationRecord {
    /// Stable identifier of this attempt, chosen by the caller.
    pub id: String,
    /// The installed application this attempt belongs to, when it produced or
    /// targeted a stored one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub application_id: Option<String>,
    /// Where the bundle came from, as the caller saw it: a path, a ZIP file or
    /// a URL.
    pub origin: String,
    /// The input format, such as `app` or `zip`. The installer owns this
    /// vocabulary; the library stores it verbatim.
    pub format: String,
    /// The outcome, such as `installed` or `rejected`. The installer owns this
    /// vocabulary; the library stores it verbatim.
    pub result: String,
    /// Why the attempt ended this way, when there is something to say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// An RFC 3339 UTC timestamp supplied by the caller.
    pub occurred_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_application_serializes_its_paths_as_text() {
        let record = ApplicationRecord {
            id: String::from("app-1"),
            bundle_id: String::from("com.example.app"),
            name: String::from("Example"),
            version: None,
            bundle_path: PathBuf::from("/Applications/Example.app"),
            executable_path: PathBuf::from("/Applications/Example.app/Contents/MacOS/Example"),
            icon_path: None,
            architecture: Some(String::from("x86_64")),
            installation_type: String::from("bundle"),
            installed_at: String::from("2026-11-09T00:00:00Z"),
            installation_status: String::from("installed"),
        };

        let json = serde_json::to_string(&record).expect("the record serializes");

        assert!(
            json.contains(r#""bundle_path":"/Applications/Example.app""#),
            "json: {json}"
        );
        assert!(json.contains(r#""architecture":"x86_64""#), "json: {json}");
        assert!(
            !json.contains("version"),
            "a missing optional field is omitted: {json}"
        );
        assert!(
            !json.contains("icon_path"),
            "a missing optional field is omitted: {json}"
        );
    }

    #[test]
    fn an_installation_serializes_its_optional_owner() {
        let record = InstallationRecord {
            id: String::from("installation-1"),
            application_id: None,
            origin: String::from("/workspace/Example.app"),
            format: String::from("app"),
            result: String::from("rejected"),
            detail: Some(String::from("the bundle declares no executable")),
            occurred_at: String::from("2026-11-09T00:00:00Z"),
        };

        let json = serde_json::to_string(&record).expect("the record serializes");

        assert!(
            !json.contains("application_id"),
            "an attempt without an owner omits it: {json}"
        );
        assert!(json.contains(r#""detail":"the bundle declares no executable""#));
    }
}
