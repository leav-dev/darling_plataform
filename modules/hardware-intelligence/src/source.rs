//! Host access indirection.
//!
//! Every provider reads the host through [`HostSource`] instead of calling
//! [`std::fs`] directly. That keeps the parsers deterministic: a test feeds a
//! captured `/proc` or `/sys` tree and asserts the resulting report, while the
//! real run uses [`LinuxHost`].
//!
//! Providers never fail globally. Each one answers with a [`Subsystem`] that
//! carries whatever it could determine plus the diagnostics explaining the rest.

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::model::{Diagnostic, DiagnosticLevel};

/// Read-only access to the host filesystem.
pub trait HostSource {
    /// Reads a whole file as UTF-8 text.
    fn read(&self, path: &str) -> io::Result<String>;

    /// Lists the entry names of a directory, sorted so scans are deterministic.
    fn read_dir(&self, path: &str) -> io::Result<Vec<String>>;

    /// Reports whether a path exists, without reading it.
    fn exists(&self, path: &str) -> bool;
}

/// The real host: reads the filesystem of the machine running the scan.
///
/// Every path is absolute and read-only; no scan needs root and nothing is
/// written back.
#[derive(Debug, Clone, Copy, Default)]
pub struct LinuxHost;

impl HostSource for LinuxHost {
    fn read(&self, path: &str) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn read_dir(&self, path: &str) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }

    fn exists(&self, path: &str) -> bool {
        Path::new(path).exists()
    }
}

/// A [`HostSource`] backed by in-memory maps.
///
/// Useful to replay a captured host and, mainly, to test providers against real
/// fixture text instead of the machine running the test suite. Missing paths
/// behave like a missing file on the real host.
#[derive(Debug, Clone, Default)]
pub struct MappingSource {
    files: BTreeMap<String, String>,
    directories: BTreeMap<String, Vec<String>>,
}

impl MappingSource {
    /// Creates an empty source.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a readable file at `path`.
    pub fn with_file(mut self, path: impl Into<String>, content: impl Into<String>) -> Self {
        self.files.insert(path.into(), content.into());
        self
    }

    /// Adds a directory whose children are `entries`.
    pub fn with_dir<I, S>(mut self, path: impl Into<String>, entries: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut names: Vec<String> = entries.into_iter().map(Into::into).collect();
        names.sort();
        self.directories.insert(path.into(), names);
        self
    }

    /// Overrides the existence answer for a path that has no file or directory.
    ///
    /// Real hosts publish some facts only through a path's presence, such as a
    /// dynamic loader shared object.
    pub fn with_present_path(mut self, path: impl Into<String>) -> Self {
        self.files
            .insert(path.into(), String::new());
        self
    }
}

/// Error returned when a [`MappingSource`] is asked for a path it does not map.
fn missing(path: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{path} is not present in this source"),
    )
}

impl HostSource for MappingSource {
    fn read(&self, path: &str) -> io::Result<String> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| missing(path))
    }

    fn read_dir(&self, path: &str) -> io::Result<Vec<String>> {
        self.directories
            .get(path)
            .cloned()
            .ok_or_else(|| missing(path))
    }

    fn exists(&self, path: &str) -> bool {
        self.files.contains_key(path) || self.directories.contains_key(path)
    }
}

/// What one provider could determine, plus the findings that explain the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subsystem<T> {
    /// `Some` when the provider could describe the subsystem.
    pub value: Option<T>,
    /// Findings about this subsystem; always recorded, even on success.
    pub diagnostics: Vec<Diagnostic>,
}

impl<T> Subsystem<T> {
    /// A fully successful provider answer.
    pub fn complete(value: T) -> Self {
        Self {
            value: Some(value),
            diagnostics: Vec::new(),
        }
    }

    /// A successful answer that also reports a finding.
    pub fn with_diagnostics(value: T, diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            value: Some(value),
            diagnostics,
        }
    }

    /// A provider that could not describe its subsystem.
    pub fn unavailable(diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            value: None,
            diagnostics,
        }
    }

    /// Whether any finding reports a failure, which degrades the whole scan.
    pub fn has_failures(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.level == DiagnosticLevel::Error)
    }
}

/// Builds a diagnostic for a subsystem that could not read a required path.
pub fn unreadable(
    provider: crate::model::Provider,
    path: &str,
    error: &io::Error,
) -> Diagnostic {
    Diagnostic {
        provider,
        level: DiagnosticLevel::Error,
        code: "source_unreadable".to_string(),
        message: format!("could not read {path}: {error}"),
    }
}

/// Builds a diagnostic for a value the host publishes in a format this scan
/// does not understand yet.
pub fn unsupported_format(provider: crate::model::Provider, path: &str, detail: &str) -> Diagnostic {
    Diagnostic {
        provider,
        level: DiagnosticLevel::Warning,
        code: "unexpected_format".to_string(),
        message: format!("{path} does not contain the expected fields ({detail})"),
    }
}

/// Renders a path for diagnostics without leaking the host's environment.
impl fmt::Display for LinuxHost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("linux host")
    }
}

/// The canonical root used by [`LinuxHost`] paths.
pub fn root() -> PathBuf {
    PathBuf::from("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Provider;

    #[test]
    fn mapping_source_returns_the_mapped_content() {
        let source = MappingSource::new().with_file("/etc/os-release", "ID=ubuntu\n");
        assert_eq!(
            source.read("/etc/os-release").expect("file is mapped"),
            "ID=ubuntu\n"
        );
    }

    #[test]
    fn mapping_source_reports_unmapped_paths_as_missing() {
        let source = MappingSource::new();
        let error = source.read("/etc/os-release").expect_err("path is missing");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error.to_string().contains("/etc/os-release"));
        assert!(source.read_dir("/proc").is_err());
        assert!(!source.exists("/etc/os-release"));
    }

    #[test]
    fn mapping_source_lists_directories_sorted() {
        let source = MappingSource::new().with_dir("/sys/class/drm", ["card0-HDMI-A-1", "card0-eDP-1"]);
        assert_eq!(
            source.read_dir("/sys/class/drm").expect("directory is mapped"),
            vec!["card0-HDMI-A-1".to_string(), "card0-eDP-1".to_string()]
        );
        assert!(source.exists("/sys/class/drm"));
    }

    #[test]
    fn present_paths_can_be_declared_without_content() {
        let source = MappingSource::new().with_present_path("/lib/ld-musl-x86_64.so.1");
        assert!(source.exists("/lib/ld-musl-x86_64.so.1"));
        assert_eq!(source.read("/lib/ld-musl-x86_64.so.1").expect("empty"), "");
    }

    #[test]
    fn subsystems_report_failures_only_for_errors() {
        let unavailable = Subsystem::<u8>::unavailable(vec![unreadable(
            Provider::Cpu,
            "/proc/cpuinfo",
            &missing("/proc/cpuinfo"),
        )]);
        assert!(unavailable.has_failures());
        assert!(unavailable.value.is_none());

        let degraded = Subsystem::with_diagnostics(
            7_u8,
            vec![unsupported_format(Provider::Cpu, "/proc/cpuinfo", "no model name")],
        );
        assert!(!degraded.has_failures());
        assert_eq!(degraded.value, Some(7));
    }

    #[test]
    fn linux_host_reads_only_reachable_paths_without_privileges() {
        let host = LinuxHost;
        let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
        let content = host.read(manifest).expect("the crate manifest is readable");
        assert!(content.contains("dp-hardware-intelligence"));
        assert!(!host.exists("/definitely/not/a/real/path"));
        assert!(host.read_dir(env!("CARGO_MANIFEST_DIR")).is_ok());
    }
}
