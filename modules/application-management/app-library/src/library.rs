//! The SQLite catalogue: opening the file, migrating it and storing rows.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, Row, params};

use crate::error::{Error, Result};
use crate::records::{ApplicationRecord, InstallationRecord};
use crate::schema;

/// SQLite extended result codes the library turns into actionable errors.
///
/// The primary code of every one of them is `SQLITE_CONSTRAINT`; the extended
/// code is what names the constraint that the row collided with.
const CONSTRAINT_PRIMARY_KEY: i32 = 1_555;
const CONSTRAINT_UNIQUE: i32 = 2_067;
const CONSTRAINT_FOREIGN_KEY: i32 = 787;

const INSERT_APPLICATION: &str = "\
INSERT INTO applications (
    id, bundle_id, name, version, bundle_path, executable_path, icon_path, architecture,
    installation_type, installed_at, installation_status
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)";

/// Every mutable column of an existing row is overwritten, so a replacement
/// never keeps the version, the icon or the timestamp of the row it replaced.
const REPLACE_APPLICATION: &str = "\
INSERT INTO applications (
    id, bundle_id, name, version, bundle_path, executable_path, icon_path, architecture,
    installation_type, installed_at, installation_status
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
ON CONFLICT(id) DO UPDATE SET
    bundle_id = excluded.bundle_id,
    name = excluded.name,
    version = excluded.version,
    bundle_path = excluded.bundle_path,
    executable_path = excluded.executable_path,
    icon_path = excluded.icon_path,
    architecture = excluded.architecture,
    installation_type = excluded.installation_type,
    installed_at = excluded.installed_at,
    installation_status = excluded.installation_status";

const SELECT_APPLICATIONS: &str = "\
SELECT id, bundle_id, name, version, bundle_path, executable_path, icon_path, architecture,
       installation_type, installed_at, installation_status
FROM applications
ORDER BY name, id";

const INSERT_INSTALLATION: &str = "\
INSERT INTO installations (id, application_id, origin, format, result, detail, occurred_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";

const SELECT_INSTALLATIONS: &str = "\
SELECT id, application_id, origin, format, result, detail, occurred_at
FROM installations
WHERE application_id = ?1
ORDER BY occurred_at, id";

const SELECT_ALL_INSTALLATIONS: &str = "\
SELECT id, application_id, origin, format, result, detail, occurred_at
FROM installations
ORDER BY occurred_at, id";

/// The persistent catalogue of installed applications and their history.
///
/// A catalogue is one SQLite file outside the DPREFIX. [`Library::open`]
/// creates it and applies the pending schema migrations, so callers never see
/// an unmigrated file. Every record is stored as written: the library does not
/// invent ids, timestamps or status values.
pub struct Library {
    connection: Connection,
    path: PathBuf,
}

impl Library {
    /// Opens the catalogue at `path`, creating the file, its missing parent
    /// directories and the current schema when they do not exist.
    ///
    /// Opening an existing catalogue applies only the migrations it is missing,
    /// so calling this twice is safe. A catalogue written by a newer build is
    /// rejected with [`Error::SchemaTooNew`] instead of being read with a
    /// schema this build does not understand.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let mut connection = Connection::open(path).map_err(|source| Error::Database {
            path: path.to_path_buf(),
            operation: "opening the catalogue",
            source,
        })?;
        // SQLite enforces foreign keys per connection and has them disabled by
        // default: without this, an installation could point at an application
        // that does not exist.
        connection
            .execute_batch("PRAGMA foreign_keys = ON")
            .map_err(|source| Error::Database {
                path: path.to_path_buf(),
                operation: "enabling foreign keys",
                source,
            })?;
        schema::migrate(&mut connection, path)?;

        Ok(Self {
            connection,
            path: path.to_path_buf(),
        })
    }

    /// Reports the schema version the catalogue declares.
    pub fn schema_version(&self) -> Result<i64> {
        schema::version(&self.connection, &self.path)
    }

    /// Stores one application.
    ///
    /// The row is rejected when its `id` is already present or when the pair
    /// (`bundle_id`, `bundle_path`) is already registered; two versions at two
    /// paths are allowed and share the bundle id.
    pub fn record_application(&self, record: &ApplicationRecord) -> Result<()> {
        let bundle_path = text_path("bundle_path", &record.bundle_path)?;
        let executable_path = text_path("executable_path", &record.executable_path)?;
        let icon_path = match &record.icon_path {
            Some(path) => Some(text_path("icon_path", path)?),
            None => None,
        };

        self.connection
            .execute(
                INSERT_APPLICATION,
                params![
                    record.id,
                    record.bundle_id,
                    record.name,
                    record.version,
                    bundle_path,
                    executable_path,
                    icon_path,
                    record.architecture,
                    record.installation_type,
                    record.installed_at,
                    record.installation_status,
                ],
            )
            .map(|_| ())
            .map_err(|error| self.application_insert_error(record, error))
    }

    /// Stores one application, replacing the row its `id` already names.
    ///
    /// The whole row is written: a replacement carries the new name, version,
    /// paths, architecture, installation type, timestamp and status, so no value
    /// of the overwritten row survives. An `id` the catalogue does not hold is
    /// inserted, so the installer can use one call whether or not the previous
    /// installation is still registered.
    ///
    /// The pair (`bundle_id`, `bundle_path`) is still unique: a replacement that
    /// would collide with a row of another `id` is rejected with
    /// [`Error::BundlePathTaken`], and the existing rows are left untouched.
    pub fn replace_application(&self, record: &ApplicationRecord) -> Result<()> {
        let bundle_path = text_path("bundle_path", &record.bundle_path)?;
        let executable_path = text_path("executable_path", &record.executable_path)?;
        let icon_path = match &record.icon_path {
            Some(path) => Some(text_path("icon_path", path)?),
            None => None,
        };

        self.connection
            .execute(
                REPLACE_APPLICATION,
                params![
                    record.id,
                    record.bundle_id,
                    record.name,
                    record.version,
                    bundle_path,
                    executable_path,
                    icon_path,
                    record.architecture,
                    record.installation_type,
                    record.installed_at,
                    record.installation_status,
                ],
            )
            .map(|_| ())
            .map_err(|error| self.application_insert_error(record, error))
    }

    /// Stores one installation attempt, successful or not.
    ///
    /// An attempt that names an application must name one the catalogue holds;
    /// an attempt that produced no application row carries no owner and is
    /// stored as it is. Such a rejected attempt is not returned by
    /// [`Self::histories`], because it belongs to no application: the installer
    /// reports it to the operator when it happens, and a later cut that lists
    /// failed attempts will add the query for it.
    pub fn record_installation(&self, record: &InstallationRecord) -> Result<()> {
        self.connection
            .execute(
                INSERT_INSTALLATION,
                params![
                    record.id,
                    record.application_id,
                    record.origin,
                    record.format,
                    record.result,
                    record.detail,
                    record.occurred_at,
                ],
            )
            .map(|_| ())
            .map_err(|error| self.installation_insert_error(record, error))
    }

    /// Lists every stored application, ordered by name and then by id.
    ///
    /// The order is part of the contract: two catalogues holding the same rows
    /// list them the same way, whatever order they were inserted in.
    pub fn list_applications(&self) -> Result<Vec<ApplicationRecord>> {
        let mut statement = self
            .connection
            .prepare(SELECT_APPLICATIONS)
            .map_err(|source| self.database("preparing the application query", source))?;
        let rows = statement
            .query_map([], application_from_row)
            .map_err(|source| self.database("querying the applications", source))?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|source| self.database("reading the applications", source))
    }

    /// Lists the installation attempts of one application, oldest first.
    ///
    /// Rows sharing `occurred_at` are ordered by id, so the result never depends
    /// on the order SQLite happens to return. An application that does not exist
    /// has no history, which is an empty list rather than an error: the caller
    /// asks about an application id it may have just failed to store.
    pub fn histories(&self, application_id: &str) -> Result<Vec<InstallationRecord>> {
        let mut statement = self
            .connection
            .prepare(SELECT_INSTALLATIONS)
            .map_err(|source| self.database("preparing the history query", source))?;
        let rows = statement
            .query_map([application_id], installation_from_row)
            .map_err(|source| self.database("querying the installation history", source))?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|source| self.database("reading the installation history", source))
    }

    /// Every recorded attempt, in `occurred_at` then `id` order.
    ///
    /// Unlike [`Library::histories`], this includes the attempts that never
    /// produced an application row — a rejected archive or a failed install —
    /// because the plan requires a full installation history and an attempt that
    /// left nothing installed is exactly what an operator needs to see.
    pub fn installations(&self) -> Result<Vec<InstallationRecord>> {
        let mut statement = self
            .connection
            .prepare(SELECT_ALL_INSTALLATIONS)
            .map_err(|source| self.database("preparing the installations query", source))?;
        let rows = statement
            .query_map([], installation_from_row)
            .map_err(|source| self.database("querying the installations", source))?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|source| self.database("reading the installations", source))
    }

    /// Classifies a failed application insert by the constraint it violated.
    fn application_insert_error(
        &self,
        record: &ApplicationRecord,
        error: rusqlite::Error,
    ) -> Error {
        match extended_constraint(&error) {
            Some(CONSTRAINT_PRIMARY_KEY) => Error::DuplicateId {
                table: "applications",
                id: record.id.clone(),
            },
            Some(CONSTRAINT_UNIQUE) => Error::BundlePathTaken {
                bundle_id: record.bundle_id.clone(),
                bundle_path: record.bundle_path.clone(),
            },
            _ => self.database("recording an application", error),
        }
    }

    /// Classifies a failed installation insert by the constraint it violated.
    fn installation_insert_error(
        &self,
        record: &InstallationRecord,
        error: rusqlite::Error,
    ) -> Error {
        match (extended_constraint(&error), &record.application_id) {
            (Some(CONSTRAINT_FOREIGN_KEY), Some(application_id)) => {
                Error::UnregisteredApplication {
                    application_id: application_id.clone(),
                }
            }
            (Some(CONSTRAINT_PRIMARY_KEY), _) => Error::DuplicateId {
                table: "installations",
                id: record.id.clone(),
            },
            _ => self.database("recording an installation", error),
        }
    }

    /// Wraps a SQLite failure that the library cannot classify further.
    fn database(&self, operation: &'static str, source: rusqlite::Error) -> Error {
        Error::Database {
            path: self.path.clone(),
            operation,
            source,
        }
    }
}

/// Returns the extended code of a constraint violation, if `error` is one.
fn extended_constraint(error: &rusqlite::Error) -> Option<i32> {
    match error {
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Some(failure.extended_code)
        }
        _ => None,
    }
}

/// Converts a path the catalogue stores as UTF-8 text.
///
/// The columns are text so the file can be read by any SQLite client, which
/// makes a path that is not valid UTF-8 unrepresentable instead of silently
/// lossy.
fn text_path<'a>(field: &'static str, path: &'a Path) -> Result<&'a str> {
    path.to_str().ok_or_else(|| Error::NonUtf8Path {
        field,
        path: path.to_path_buf(),
    })
}

/// Reads one `applications` row. The column names match [`SELECT_APPLICATIONS`].
fn application_from_row(row: &Row<'_>) -> rusqlite::Result<ApplicationRecord> {
    Ok(ApplicationRecord {
        id: row.get("id")?,
        bundle_id: row.get("bundle_id")?,
        name: row.get("name")?,
        version: row.get("version")?,
        bundle_path: PathBuf::from(row.get::<_, String>("bundle_path")?),
        executable_path: PathBuf::from(row.get::<_, String>("executable_path")?),
        icon_path: row
            .get::<_, Option<String>>("icon_path")?
            .map(PathBuf::from),
        architecture: row.get("architecture")?,
        installation_type: row.get("installation_type")?,
        installed_at: row.get("installed_at")?,
        installation_status: row.get("installation_status")?,
    })
}

/// Reads one `installations` row. The column names match [`SELECT_INSTALLATIONS`].
fn installation_from_row(row: &Row<'_>) -> rusqlite::Result<InstallationRecord> {
    Ok(InstallationRecord {
        id: row.get("id")?,
        application_id: row.get("application_id")?,
        origin: row.get("origin")?,
        format: row.get("format")?,
        result: row.get("result")?,
        detail: row.get("detail")?,
        occurred_at: row.get("occurred_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::records::{ApplicationRecord, InstallationRecord};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn catalogue(directory: &TempDir) -> PathBuf {
        directory.path().join("library.sqlite3")
    }

    fn library(directory: &TempDir) -> Library {
        Library::open(&catalogue(directory)).expect("a fresh catalogue opens")
    }

    fn application(id: &str, name: &str, bundle_id: &str, bundle_path: &str) -> ApplicationRecord {
        ApplicationRecord {
            id: id.to_string(),
            bundle_id: bundle_id.to_string(),
            name: name.to_string(),
            version: Some("6.6.1".to_string()),
            bundle_path: PathBuf::from(bundle_path),
            executable_path: PathBuf::from(format!("{bundle_path}/Contents/MacOS/{name}")),
            icon_path: Some(PathBuf::from(format!(
                "{bundle_path}/Contents/Resources/{name}.icns"
            ))),
            architecture: Some("x86_64".to_string()),
            installation_type: "bundle".to_string(),
            installed_at: "2026-11-09T00:00:00Z".to_string(),
            installation_status: "installed".to_string(),
        }
    }

    fn installation(
        id: &str,
        application_id: Option<&str>,
        occurred_at: &str,
    ) -> InstallationRecord {
        InstallationRecord {
            id: id.to_string(),
            application_id: application_id.map(str::to_string),
            origin: "/workspace/Example.app".to_string(),
            format: "app".to_string(),
            result: "installed".to_string(),
            detail: Some("copied 42 files".to_string()),
            occurred_at: occurred_at.to_string(),
        }
    }

    fn record_example(library: &Library, id: &str) {
        library
            .record_application(&application(
                id,
                "Example",
                "com.example.app",
                &format!("/Applications/Example {id}.app"),
            ))
            .expect("the application is recorded");
    }

    #[test]
    fn a_fresh_catalogue_ends_at_the_current_schema_version() {
        let directory = TempDir::new().expect("a temporary directory");
        let catalogue = catalogue(&directory);

        let library = Library::open(&catalogue).expect("a fresh catalogue opens");

        assert_eq!(
            library.schema_version().expect("the version is readable"),
            crate::schema::latest_version()
        );
        assert!(catalogue.is_file(), "the catalogue file is created");
    }

    #[test]
    fn opening_an_existing_catalogue_is_idempotent() {
        let directory = TempDir::new().expect("a temporary directory");
        let path = catalogue(&directory);

        {
            let library = Library::open(&path).expect("a fresh catalogue opens");
            record_example(&library, "app-1");
        }

        let reopened = Library::open(&path).expect("an existing catalogue reopens");

        assert_eq!(
            reopened.schema_version().expect("the version is readable"),
            crate::schema::latest_version()
        );
        assert_eq!(
            reopened
                .list_applications()
                .expect("the applications are readable")
                .len(),
            1,
            "reopening must not recreate the schema or drop the rows"
        );
    }

    #[test]
    fn opening_a_catalogue_creates_its_missing_directory() {
        let directory = TempDir::new().expect("a temporary directory");
        let nested = directory
            .path()
            .join("darling-platform")
            .join("library.sqlite3");

        let library = Library::open(&nested).expect("a catalogue in a new directory opens");

        assert_eq!(
            library.schema_version().expect("the version is readable"),
            crate::schema::latest_version()
        );
    }

    #[test]
    fn an_application_round_trips_every_field() {
        let directory = TempDir::new().expect("a temporary directory");
        let library = library(&directory);
        let record = application(
            "app-1",
            "Example",
            "com.example.app",
            "/Applications/Example.app",
        );

        library
            .record_application(&record)
            .expect("the application is recorded");

        assert_eq!(
            library
                .list_applications()
                .expect("the applications are readable"),
            vec![record]
        );
    }

    #[test]
    fn an_application_without_the_optional_fields_round_trips() {
        let directory = TempDir::new().expect("a temporary directory");
        let library = library(&directory);
        let record = ApplicationRecord {
            version: None,
            icon_path: None,
            architecture: None,
            ..application(
                "app-1",
                "Example",
                "com.example.app",
                "/Applications/Example.app",
            )
        };

        library
            .record_application(&record)
            .expect("the application is recorded");

        assert_eq!(
            library
                .list_applications()
                .expect("the applications are readable"),
            vec![record]
        );
    }

    #[test]
    fn an_installation_round_trips_every_field() {
        let directory = TempDir::new().expect("a temporary directory");
        let library = library(&directory);
        let record = installation("installation-1", Some("app-1"), "2026-11-09T10:00:00Z");
        record_example(&library, "app-1");

        library
            .record_installation(&record)
            .expect("the installation is recorded");

        assert_eq!(
            library.histories("app-1").expect("the history is readable"),
            vec![record]
        );
    }

    #[test]
    fn an_installation_that_names_no_application_is_not_attributed() {
        let directory = TempDir::new().expect("a temporary directory");
        let library = library(&directory);
        let record = InstallationRecord {
            application_id: None,
            detail: None,
            result: "rejected".to_string(),
            ..installation("installation-1", None, "2026-11-09T10:00:00Z")
        };

        library
            .record_installation(&record)
            .expect("an installation that names no application is recorded");

        let mut stored: Vec<InstallationRecord> = Vec::new();
        for application_id in ["app-1", "app-2", ""] {
            stored.extend(
                library
                    .histories(application_id)
                    .expect("the history is readable"),
            );
        }
        assert!(stored.is_empty(), "a nameless installation has no owner");
    }

    #[test]
    fn installations_include_attempts_that_never_produced_an_application() {
        let directory = TempDir::new().expect("a temporary directory");
        let library = library(&directory);
        record_example(&library, "app-1");
        library
            .record_installation(&installation(
                "installation-1",
                Some("app-1"),
                "2026-11-09T10:00:00Z",
            ))
            .expect("a successful attempt is recorded");
        let failed = InstallationRecord {
            application_id: None,
            detail: Some("destination exists".to_string()),
            result: "failed".to_string(),
            ..installation("installation-2", None, "2026-11-09T11:00:00Z")
        };
        library
            .record_installation(&failed)
            .expect("a failed attempt is recorded");

        let all = library
            .installations()
            .expect("the installations are readable");

        assert_eq!(
            all.len(),
            2,
            "the history keeps the failed attempt: {all:?}"
        );
        assert_eq!(all[0].id, "installation-1", "ordered by occurred_at");
        assert_eq!(all[1].application_id, None);
        assert_eq!(all[1].result, "failed");
        assert_eq!(all[1].detail.as_deref(), Some("destination exists"));
    }

    #[test]
    fn applications_are_listed_by_name_then_id() {
        let directory = TempDir::new().expect("a temporary directory");
        let library = library(&directory);
        for (id, name) in [
            ("app-b", "Zebra"),
            ("app-c", "Aardvark"),
            ("app-a", "Zebra"),
        ] {
            library
                .record_application(&application(
                    id,
                    name,
                    "com.example.app",
                    &format!("/Applications/{name} {id}.app"),
                ))
                .expect("the application is recorded");
        }

        let listed: Vec<(String, String)> = library
            .list_applications()
            .expect("the applications are readable")
            .into_iter()
            .map(|record| (record.name, record.id))
            .collect();

        assert_eq!(
            listed,
            vec![
                ("Aardvark".to_string(), "app-c".to_string()),
                ("Zebra".to_string(), "app-a".to_string()),
                ("Zebra".to_string(), "app-b".to_string()),
            ]
        );
        assert_eq!(
            library
                .list_applications()
                .expect("the applications are readable"),
            library
                .list_applications()
                .expect("the applications are readable"),
            "the order does not depend on the insertion order"
        );
    }

    #[test]
    fn the_history_filter_by_application_and_orders_by_occurrence() {
        let directory = TempDir::new().expect("a temporary directory");
        let library = library(&directory);
        record_example(&library, "app-1");
        library
            .record_application(&application(
                "app-2",
                "Other",
                "com.example.other",
                "/Applications/Other.app",
            ))
            .expect("the second application is recorded");
        for record in [
            installation("installation-3", Some("app-1"), "2026-11-09T12:00:00Z"),
            installation("installation-1", Some("app-1"), "2026-11-09T10:00:00Z"),
            installation("installation-b", Some("app-1"), "2026-11-09T11:00:00Z"),
            installation("installation-a", Some("app-1"), "2026-11-09T11:00:00Z"),
            installation("installation-2", Some("app-2"), "2026-11-09T10:30:00Z"),
        ] {
            library
                .record_installation(&record)
                .expect("the installation is recorded");
        }

        let history: Vec<String> = library
            .histories("app-1")
            .expect("the history is readable")
            .into_iter()
            .map(|record| record.id)
            .collect();

        assert_eq!(
            history,
            vec![
                "installation-1".to_string(),
                "installation-a".to_string(),
                "installation-b".to_string(),
                "installation-3".to_string(),
            ]
        );
        assert_eq!(
            library
                .histories("app-2")
                .expect("the history is readable")
                .len(),
            1,
            "another application's history is not mixed in"
        );
        assert!(
            library
                .histories("app-missing")
                .expect("an unknown application has no history")
                .is_empty()
        );
    }

    #[test]
    fn the_same_bundle_at_the_same_path_cannot_be_recorded_twice() {
        let directory = TempDir::new().expect("a temporary directory");
        let library = library(&directory);
        let record = application(
            "app-1",
            "Example",
            "com.example.app",
            "/Applications/Example.app",
        );
        library
            .record_application(&record)
            .expect("the application is recorded");

        let duplicate = ApplicationRecord {
            id: "app-2".to_string(),
            ..record.clone()
        };
        let error = library
            .record_application(&duplicate)
            .expect_err("the same bundle cannot be registered at the same path twice");

        match &error {
            Error::BundlePathTaken {
                bundle_id,
                bundle_path,
            } => {
                assert_eq!(bundle_id, "com.example.app");
                assert_eq!(bundle_path, &PathBuf::from("/Applications/Example.app"));
            }
            other => panic!("expected a bundle path conflict, got {other:?}"),
        }
        let message = error.to_string();
        assert!(message.contains("com.example.app"), "message: {message}");
        assert!(
            message.contains("/Applications/Example.app"),
            "message: {message}"
        );
        assert_eq!(
            library
                .list_applications()
                .expect("the applications are readable"),
            vec![record],
            "the rejected row is not stored"
        );
    }

    #[test]
    fn the_same_bundle_at_another_path_is_allowed() {
        let directory = TempDir::new().expect("a temporary directory");
        let library = library(&directory);
        library
            .record_application(&application(
                "app-1",
                "Example",
                "com.example.app",
                "/Applications/Example.app",
            ))
            .expect("the first version is recorded");

        library
            .record_application(&application(
                "app-2",
                "Example",
                "com.example.app",
                "/Applications/Example 2.app",
            ))
            .expect("two installed versions may share the bundle id");

        assert_eq!(
            library
                .list_applications()
                .expect("the applications are readable")
                .len(),
            2
        );
    }

    #[test]
    fn a_replacement_overwrites_every_mutable_column_of_the_named_row() {
        let directory = TempDir::new().expect("a temporary directory");
        let library = library(&directory);
        library
            .record_application(&application(
                "app-1",
                "Example",
                "com.example.app",
                "/Applications/Example.app",
            ))
            .expect("the application is recorded");
        let replacement = ApplicationRecord {
            name: String::from("Example Renamed"),
            version: Some(String::from("7.0.0")),
            bundle_path: PathBuf::from("/Applications/Example 7.app"),
            executable_path: PathBuf::from("/Applications/Example 7.app/Contents/MacOS/Example"),
            icon_path: None,
            architecture: Some(String::from("arm64")),
            installation_type: String::from("zip"),
            installed_at: String::from("2026-11-10T00:00:00Z"),
            installation_status: String::from("installed"),
            ..application(
                "app-1",
                "Example",
                "com.example.app",
                "/Applications/Example.app",
            )
        };

        library
            .replace_application(&replacement)
            .expect("the row is replaced");

        assert_eq!(
            library
                .list_applications()
                .expect("the applications are readable"),
            vec![replacement],
            "the row keeps no column of the application it replaced"
        );
    }

    #[test]
    fn a_replacement_of_an_unknown_id_is_inserted() {
        let directory = TempDir::new().expect("a temporary directory");
        let library = library(&directory);
        let record = application(
            "app-1",
            "Example",
            "com.example.app",
            "/Applications/Example.app",
        );

        library
            .replace_application(&record)
            .expect("an unheld id is inserted");

        assert_eq!(
            library
                .list_applications()
                .expect("the applications are readable"),
            vec![record]
        );
    }

    #[test]
    fn a_replacement_that_collides_with_another_row_is_rejected() {
        let directory = TempDir::new().expect("a temporary directory");
        let library = library(&directory);
        library
            .record_application(&application(
                "app-1",
                "Example",
                "com.example.app",
                "/Applications/Example.app",
            ))
            .expect("the first application is recorded");
        library
            .record_application(&application(
                "app-2",
                "Other",
                "com.example.app",
                "/Applications/Other.app",
            ))
            .expect("the second application is recorded");

        let error = library
            .replace_application(&application(
                "app-2",
                "Other",
                "com.example.app",
                "/Applications/Example.app",
            ))
            .expect_err("the bundle path is unique across rows");

        assert!(
            matches!(&error, Error::BundlePathTaken { bundle_path, .. } if bundle_path == &PathBuf::from("/Applications/Example.app")),
            "unexpected error: {error:?}"
        );
        assert_eq!(
            library
                .list_applications()
                .expect("the applications are readable")
                .len(),
            2,
            "the rejected replacement leaves both rows untouched"
        );
    }

    #[test]
    fn a_duplicate_application_id_is_rejected() {
        let directory = TempDir::new().expect("a temporary directory");
        let library = library(&directory);
        library
            .record_application(&application(
                "app-1",
                "Example",
                "com.example.app",
                "/Applications/Example.app",
            ))
            .expect("the application is recorded");

        let error = library
            .record_application(&application(
                "app-1",
                "Other",
                "com.example.other",
                "/Applications/Other.app",
            ))
            .expect_err("an application id is unique");

        assert!(
            matches!(&error, Error::DuplicateId { table, id } if *table == "applications" && id == "app-1"),
            "unexpected error: {error:?}"
        );
        assert!(error.to_string().contains("app-1"), "{error}");
    }

    #[test]
    fn a_duplicate_installation_id_is_rejected() {
        let directory = TempDir::new().expect("a temporary directory");
        let library = library(&directory);
        record_example(&library, "app-1");
        let record = installation("installation-1", Some("app-1"), "2026-11-09T10:00:00Z");
        library
            .record_installation(&record)
            .expect("the installation is recorded");

        let error = library
            .record_installation(&InstallationRecord {
                occurred_at: "2026-11-09T11:00:00Z".to_string(),
                ..record.clone()
            })
            .expect_err("an installation id is unique");

        assert!(
            matches!(&error, Error::DuplicateId { table, id } if *table == "installations" && id == "installation-1"),
            "unexpected error: {error:?}"
        );
        assert_eq!(
            library.histories("app-1").expect("the history is readable"),
            vec![record]
        );
    }

    #[test]
    fn an_installation_of_an_unregistered_application_is_rejected() {
        let directory = TempDir::new().expect("a temporary directory");
        let library = library(&directory);
        let record = installation(
            "installation-1",
            Some("app-missing"),
            "2026-11-09T10:00:00Z",
        );

        let error = library
            .record_installation(&record)
            .expect_err("the history cannot point at an application the library does not hold");

        assert!(
            matches!(&error, Error::UnregisteredApplication { application_id } if application_id == "app-missing"),
            "unexpected error: {error:?}"
        );
        assert!(error.to_string().contains("app-missing"), "{error}");
        assert!(
            library
                .histories("app-missing")
                .expect("the history is readable")
                .is_empty()
        );
    }

    #[test]
    fn a_catalogue_from_a_newer_build_is_rejected() {
        let directory = TempDir::new().expect("a temporary directory");
        let path = catalogue(&directory);
        let future = crate::schema::latest_version() + 1;
        {
            let connection = rusqlite::Connection::open(&path).expect("the raw catalogue opens");
            connection
                .pragma_update(None, "user_version", future)
                .expect("the future version is written");
        }

        let error = match Library::open(&path) {
            Ok(_) => panic!("a newer schema cannot be opened"),
            Err(error) => error,
        };

        match &error {
            Error::SchemaTooNew {
                found, supported, ..
            } => {
                assert_eq!(*found, future);
                assert_eq!(*supported, crate::schema::latest_version());
            }
            other => panic!("expected a schema version conflict, got {other:?}"),
        }
        let message = error.to_string();
        assert!(message.contains(&future.to_string()), "message: {message}");
        assert!(
            message.contains(&crate::schema::latest_version().to_string()),
            "message: {message}"
        );
        let connection = rusqlite::Connection::open(&path).expect("the raw catalogue opens");
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("the version is readable");
        assert_eq!(version, future, "the rejected file is left untouched");
    }

    #[test]
    fn a_path_that_is_not_utf8_cannot_be_stored() {
        let directory = TempDir::new().expect("a temporary directory");
        let library = library(&directory);
        #[cfg(unix)]
        let bundle_path = {
            use std::ffi::OsStr;
            use std::os::unix::ffi::OsStrExt;
            PathBuf::from(OsStr::from_bytes(b"/Applications/\xff.app"))
        };
        #[cfg(windows)]
        let bundle_path = {
            use std::ffi::OsString;
            use std::os::windows::ffi::OsStringExt;
            PathBuf::from(OsString::from_wide(&[
                0x2f, 0x41, 0xd800, 0x2e, 0x61, 0x70, 0x70,
            ]))
        };

        let error = library
            .record_application(&ApplicationRecord {
                bundle_path,
                ..application(
                    "app-1",
                    "Example",
                    "com.example.app",
                    "/Applications/Example.app",
                )
            })
            .expect_err("a non-UTF-8 path cannot be stored as text");

        assert!(
            matches!(&error, Error::NonUtf8Path { field, .. } if *field == "bundle_path"),
            "unexpected error: {error:?}"
        );
        assert!(
            library
                .list_applications()
                .expect("the applications are readable")
                .is_empty()
        );
    }
}
