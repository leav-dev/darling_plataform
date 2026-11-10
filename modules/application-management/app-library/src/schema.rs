//! Versioned schema of the Application Library.
//!
//! The schema version lives in SQLite's `PRAGMA user_version`, so the file
//! carries the version with it: a catalogue copied or moved keeps its version
//! and no side table can drift from reality. Each step runs inside a
//! transaction, so a failing migration leaves the previous schema and its rows
//! intact instead of a half-created one.

use std::path::Path;

use rusqlite::Connection;

use crate::error::{Error, Result};

/// One step of the schema, applied once and never rewritten.
struct Migration {
    /// The `user_version` the step produces. Steps are contiguous from 1.
    version: i64,
    /// The statements that take the schema from the previous version to this
    /// one. They must be backwards compatible with the rows already stored.
    sql: &'static str,
}

/// Creates the tables of version 1 and the indexes the queries need.
///
/// The pair (`bundle_id`, `bundle_path`) is unique, while `bundle_id` alone is
/// not: two installed versions of the same application share a bundle id. The
/// unique constraint already builds an index whose leftmost column is
/// `bundle_id`, so a lookup by bundle id uses it and a second index on
/// `bundle_id` alone would only add write cost. The history index leads with
/// `application_id` and continues with the sort columns, so one index serves
/// both the filter and the order of [`crate::Library::histories`].
const SCHEMA_V1: &str = "\
CREATE TABLE applications (
    id TEXT NOT NULL PRIMARY KEY,
    bundle_id TEXT NOT NULL,
    name TEXT NOT NULL,
    version TEXT,
    bundle_path TEXT NOT NULL,
    executable_path TEXT NOT NULL,
    icon_path TEXT,
    architecture TEXT,
    installation_type TEXT NOT NULL,
    installed_at TEXT NOT NULL,
    installation_status TEXT NOT NULL,
    CONSTRAINT applications_bundle_path_unique UNIQUE (bundle_id, bundle_path)
);

CREATE TABLE installations (
    id TEXT NOT NULL PRIMARY KEY,
    application_id TEXT REFERENCES applications (id) ON DELETE SET NULL,
    origin TEXT NOT NULL,
    format TEXT NOT NULL,
    result TEXT NOT NULL,
    detail TEXT,
    occurred_at TEXT NOT NULL
);

CREATE INDEX installations_by_application
    ON installations (application_id, occurred_at, id);
";

/// Every step of the schema, in the order they are applied.
const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: SCHEMA_V1,
}];

/// The version a catalogue has once every step has been applied.
pub(crate) fn latest_version() -> i64 {
    MIGRATIONS.last().map_or(0, |migration| migration.version)
}

/// Reads the version the catalogue declares.
pub(crate) fn version(connection: &Connection, path: &Path) -> Result<i64> {
    connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|source| Error::Database {
            path: path.to_path_buf(),
            operation: "reading the schema version",
            source,
        })
}

/// Brings `connection` to [`latest_version`], applying only the missing steps.
///
/// Opening a catalogue that is already current does nothing, which keeps
/// `Library::open` idempotent. A catalogue from a newer build is rejected
/// instead of being read with a schema this build does not understand.
pub(crate) fn migrate(connection: &mut Connection, path: &Path) -> Result<()> {
    let current = version(connection, path)?;
    let latest = latest_version();
    if current > latest {
        return Err(Error::SchemaTooNew {
            path: path.to_path_buf(),
            found: current,
            supported: latest,
        });
    }
    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version > current)
    {
        apply(connection, path, migration)?;
    }
    Ok(())
}

/// Applies one step atomically: either the schema and the announced version
/// move together, or neither moves.
fn apply(connection: &mut Connection, path: &Path, migration: &Migration) -> Result<()> {
    let failed = |source: rusqlite::Error| Error::MigrationFailed {
        path: path.to_path_buf(),
        version: migration.version,
        source,
    };
    let transaction = connection.transaction().map_err(failed)?;
    transaction.execute_batch(migration.sql).map_err(failed)?;
    transaction
        .pragma_update(None, "user_version", migration.version)
        .map_err(failed)?;
    transaction.commit().map_err(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_migrations_are_contiguous_from_one() {
        assert!(!MIGRATIONS.is_empty(), "the catalogue needs a first step");
        for (index, migration) in MIGRATIONS.iter().enumerate() {
            assert_eq!(
                migration.version,
                index as i64 + 1,
                "a step was renumbered or removed; append a step instead"
            );
            assert!(
                !migration.sql.trim().is_empty(),
                "every step must carry statements"
            );
        }
    }

    #[test]
    fn the_latest_version_is_the_last_step() {
        assert_eq!(
            latest_version(),
            MIGRATIONS.last().expect("there is a step").version
        );
        assert_eq!(latest_version(), 1);
    }
}
