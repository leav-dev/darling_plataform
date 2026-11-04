//! Persisted lifecycle state of the shared prefix.
//!
//! Darling has no documented way to report whether its container is alive, so the
//! platform keeps its own phase and reports a phase that stayed mid-transition as
//! `error` instead of guessing. See `ADR-0008`.
//!
//! The on-disk JSON shape is a contract: the phase names are pinned by tests.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Lifecycle phase of the shared prefix.
///
/// These are the platform's orchestration phases; they are not Darling's internal
/// states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// No platform state has been recorded yet.
    NotInitialized,
    /// The prefix is being created through Darling.
    Initializing,
    /// The container is down and the prefix exists.
    Stopped,
    /// Darling is starting the container.
    Starting,
    /// The container is up.
    Running,
    /// An operation is in progress and blocks state-changing requests.
    Busy,
    /// Darling is shutting the container down.
    Stopping,
    /// A previous operation was interrupted or failed.
    Error,
}

impl Phase {
    /// Returns `true` for the phases a crash can leave behind half-done.
    pub fn is_transitional(self) -> bool {
        matches!(self, Self::Initializing | Self::Starting | Self::Stopping)
    }
}

/// One persisted state snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateRecord {
    /// On-disk schema version of this record.
    pub schema_version: u32,
    /// Phase the platform last committed.
    pub phase: Phase,
    /// Operation that was running, when the phase belongs to one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    /// Unix timestamp in seconds, supplied by the caller (never read from the clock here).
    pub updated_at: u64,
}

impl StateRecord {
    /// Highest schema version this build understands.
    pub const SCHEMA_VERSION: u32 = 1;

    /// Builds a record stamped with the current [`StateRecord::SCHEMA_VERSION`].
    pub fn new(phase: Phase, operation: Option<String>, updated_at: u64) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            phase,
            operation,
            updated_at,
        }
    }

    /// Returns `true` when the recorded phase is mid-transition.
    pub fn is_transitional(&self) -> bool {
        self.phase.is_transitional()
    }
}

/// Reads and writes the [`StateRecord`] stored in the platform data directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateStore {
    path: PathBuf,
}

impl StateStore {
    /// Creates a store backed by `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Path of the state file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads the persisted record, or `Ok(None)` when the file does not exist.
    ///
    /// A record written by a newer schema is rejected instead of being
    /// misinterpreted.
    pub fn load(&self) -> Result<Option<StateRecord>> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(Error::Io {
                    path: self.path.clone(),
                    source,
                });
            }
        };

        let record: StateRecord =
            serde_json::from_slice(&bytes).map_err(|source| Error::StateJson {
                path: self.path.clone(),
                source,
            })?;

        if record.schema_version > Self::SUPPORTED_SCHEMA_VERSION {
            return Err(Error::StateSchemaUnsupported {
                path: self.path.clone(),
                found: record.schema_version,
                supported: Self::SUPPORTED_SCHEMA_VERSION,
            });
        }

        Ok(Some(record))
    }

    /// Persists `record` atomically: write a temporary file in the same
    /// directory, flush it, then rename it over the state file.
    ///
    /// A reader therefore sees either the previous record or the new one, never
    /// a half-written file. Temporary files are removed on failure.
    pub fn save(&self, record: &StateRecord) -> Result<()> {
        let data_dir = self.data_dir();
        std::fs::create_dir_all(data_dir).map_err(|source| Error::Io {
            path: data_dir.to_path_buf(),
            source,
        })?;

        let payload = serde_json::to_vec_pretty(record).map_err(|source| Error::StateJson {
            path: self.path.clone(),
            source,
        })?;

        let temporary = self.temporary_path(data_dir);
        let mut file = std::fs::File::create(&temporary).map_err(|source| Error::Io {
            path: temporary.clone(),
            source,
        })?;

        let written = file
            .write_all(&payload)
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.sync_all());
        if let Err(source) = written {
            drop(file);
            let _ = std::fs::remove_file(&temporary);
            return Err(Error::Io {
                path: temporary,
                source,
            });
        }
        drop(file);

        std::fs::rename(&temporary, &self.path).map_err(|source| {
            let _ = std::fs::remove_file(&temporary);
            Error::Io {
                path: self.path.clone(),
                source,
            }
        })
    }

    /// Highest schema version this build can read.
    const SUPPORTED_SCHEMA_VERSION: u32 = StateRecord::SCHEMA_VERSION;

    /// Directory that holds the state file.
    fn data_dir(&self) -> &Path {
        match self.path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        }
    }

    /// Unique sibling path used to stage an atomic replacement.
    fn temporary_path(&self, data_dir: &Path) -> PathBuf {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

        let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let name = self
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| String::from("state.json"));
        data_dir.join(format!(".{name}.tmp.{}.{id}", std::process::id()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_in(dir: &tempfile::TempDir) -> StateStore {
        StateStore::new(dir.path().join("darling-platform").join("state.json"))
    }

    fn record() -> StateRecord {
        StateRecord::new(Phase::Running, Some(String::from("start")), 1_700_000_000)
    }

    #[test]
    fn save_then_load_keeps_every_field() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = store_in(&dir);

        store.save(&record()).expect("save must succeed");
        let loaded = store
            .load()
            .expect("load must succeed")
            .expect("record must exist");

        assert_eq!(loaded, record());
        assert_eq!(loaded.schema_version, StateRecord::SCHEMA_VERSION);
        assert_eq!(loaded.phase, Phase::Running);
        assert_eq!(loaded.operation.as_deref(), Some("start"));
        assert_eq!(loaded.updated_at, 1_700_000_000);
    }

    #[test]
    fn missing_file_loads_as_none() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = store_in(&dir);

        assert_eq!(store.load().expect("absent state is not an error"), None);
    }

    #[test]
    fn invalid_json_is_reported_as_state_json() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = store_in(&dir);
        std::fs::create_dir_all(store.path().parent().unwrap()).expect("create data dir");
        std::fs::write(store.path(), b"{ not json").expect("write broken state");

        let error = store.load().expect_err("broken JSON must fail");
        match error {
            Error::StateJson { path, .. } => assert_eq!(path, store.path()),
            other => panic!("expected StateJson, got {other:?}"),
        }
    }

    #[test]
    fn a_newer_schema_is_rejected_explicitly() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = store_in(&dir);
        std::fs::create_dir_all(store.path().parent().unwrap()).expect("create data dir");
        std::fs::write(
            store.path(),
            b"{\"schema_version\":2,\"phase\":\"running\",\"updated_at\":1}",
        )
        .expect("write newer state");

        let error = store.load().expect_err("a newer schema must be rejected");
        match error {
            Error::StateSchemaUnsupported {
                path,
                found,
                supported,
            } => {
                assert_eq!(path, store.path());
                assert_eq!(found, 2);
                assert_eq!(supported, StateRecord::SCHEMA_VERSION);
            }
            other => panic!("expected StateSchemaUnsupported, got {other:?}"),
        }
    }

    #[test]
    fn phase_names_are_the_on_disk_contract() {
        let expected = [
            (Phase::NotInitialized, "not_initialized"),
            (Phase::Initializing, "initializing"),
            (Phase::Stopped, "stopped"),
            (Phase::Starting, "starting"),
            (Phase::Running, "running"),
            (Phase::Busy, "busy"),
            (Phase::Stopping, "stopping"),
            (Phase::Error, "error"),
        ];

        for (phase, name) in expected {
            let json = serde_json::to_value(StateRecord::new(phase, None, 7)).expect("serialize");
            assert_eq!(
                json["phase"],
                serde_json::Value::String(String::from(name)),
                "phase {phase:?} must serialize as {name}"
            );
            let parsed: Phase =
                serde_json::from_value(serde_json::Value::String(String::from(name)))
                    .expect("deserialize");
            assert_eq!(parsed, phase);
        }
    }

    #[test]
    fn only_three_phases_are_transitional() {
        for phase in [
            Phase::NotInitialized,
            Phase::Stopped,
            Phase::Running,
            Phase::Busy,
            Phase::Error,
        ] {
            assert!(!phase.is_transitional(), "{phase:?} must be stable");
            assert!(!StateRecord::new(phase, None, 1).is_transitional());
        }

        for phase in [Phase::Initializing, Phase::Starting, Phase::Stopping] {
            assert!(phase.is_transitional(), "{phase:?} must be transitional");
            assert!(StateRecord::new(phase, None, 1).is_transitional());
        }
    }

    #[test]
    fn save_leaves_no_temporary_file_and_overwrites() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = store_in(&dir);
        let data_dir = store.path().parent().expect("data dir").to_path_buf();

        store.save(&record()).expect("first save");

        let mut names: Vec<_> = std::fs::read_dir(&data_dir)
            .expect("read data dir")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![std::ffi::OsString::from("state.json")],
            "save must not leave temporary files behind"
        );

        let replacement = StateRecord::new(Phase::Stopped, None, 1_700_000_100);
        store.save(&replacement).expect("second save");
        assert_eq!(store.load().expect("load"), Some(replacement));

        let entries = std::fs::read_dir(&data_dir).expect("read data dir").count();
        assert_eq!(entries, 1, "overwriting must not add files");
    }
}
