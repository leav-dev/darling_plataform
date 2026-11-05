//! Resolution of the shared DPREFIX location.
//!
//! The environment is injected as [`PrefixEnv`] instead of read from the process
//! inside library code, so resolution is pure and testable without touching the
//! developer's real `$HOME`. [`PrefixEnv::from_process`] is the only function
//! that reads the process environment, and callers (the CLI) decide when.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};

/// Directory name the platform owns inside the user data directory.
const PLATFORM_DIR: &str = "darling-platform";

/// Directory name of the prefix created inside the platform data directory.
const PREFIX_DIR: &str = "prefix";

/// Name of the lock file inside the platform data directory.
const LOCK_FILE: &str = "prefix.lock";

/// Name of the state file inside the platform data directory.
const STATE_FILE: &str = "state.json";

/// Maximum prefix path length Darling accepts, in bytes.
pub const MAX_PREFIX_PATH_BYTES: usize = 255;

/// Name of the user's conventional Darling prefix, relative to `HOME`.
const CONVENTIONAL_PREFIX_DIR: &str = ".darling";

/// Environment inputs used to resolve the prefix location.
///
/// Injected rather than read from the process so that resolution has no ambient
/// dependency and tests never depend on the developer's environment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrefixEnv {
    /// Contents of `DPREFIX`, if set.
    pub dprefix: Option<OsString>,
    /// Contents of `XDG_DATA_HOME`, if set.
    pub xdg_data_home: Option<OsString>,
    /// Contents of `HOME`, if set.
    pub home: Option<OsString>,
}

impl PrefixEnv {
    /// Reads `DPREFIX`, `XDG_DATA_HOME` and `HOME` from the process environment.
    pub fn from_process() -> Self {
        Self {
            dprefix: std::env::var_os("DPREFIX"),
            xdg_data_home: std::env::var_os("XDG_DATA_HOME"),
            home: std::env::var_os("HOME"),
        }
    }
}

/// The resolved shared prefix and the platform data directory that owns its
/// lock and state files.
///
/// The data directory is always outside the prefix: the platform never stores
/// its own bookkeeping where Darling's overlay can shadow or replace it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixLocation {
    path: PathBuf,
    data_dir: PathBuf,
}

impl PrefixLocation {
    /// Resolves the prefix location from the injected environment.
    ///
    /// Rules:
    /// - the data directory is `$XDG_DATA_HOME/darling-platform` when
    ///   `XDG_DATA_HOME` is absolute, else `$HOME/.local/share/darling-platform`;
    /// - the prefix is `DPREFIX` when set and non-empty, else `<data_dir>/prefix`;
    /// - `DPREFIX` must be absolute, at most [`MAX_PREFIX_PATH_BYTES`] bytes, and
    ///   must not be the user's conventional `~/.darling` prefix.
    pub fn resolve(env: &PrefixEnv) -> Result<Self> {
        let data_dir = resolve_data_dir(env)?;
        let path = match env.dprefix.as_deref().filter(|raw| !raw.is_empty()) {
            Some(raw) => normalize(Path::new(raw)),
            None => data_dir.join(PREFIX_DIR),
        };

        if !path.is_absolute() {
            return Err(Error::PrefixNotAbsolute { path });
        }

        let length = os_str_byte_len(path.as_os_str());
        if length > MAX_PREFIX_PATH_BYTES {
            return Err(Error::PrefixPathTooLong {
                path,
                max_bytes: MAX_PREFIX_PATH_BYTES,
            });
        }

        if let Some(home) = usable_home(env) {
            let conventional = normalize(&home.join(CONVENTIONAL_PREFIX_DIR));
            if path == conventional {
                return Err(Error::ConventionalPrefixConflict { path });
            }
        }

        Ok(Self { path, data_dir })
    }

    /// The dedicated DPREFIX handed to Darling.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The platform data directory, used for the lock and state files.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Path of the lock file that serialises incompatible operations.
    pub fn lock_path(&self) -> PathBuf {
        self.data_dir.join(LOCK_FILE)
    }

    /// Path of the persisted lifecycle state file.
    pub fn state_path(&self) -> PathBuf {
        self.data_dir.join(STATE_FILE)
    }
}

fn resolve_data_dir(env: &PrefixEnv) -> Result<PathBuf> {
    if let Some(xdg) = env.xdg_data_home.as_deref().filter(|raw| !raw.is_empty()) {
        let xdp = Path::new(xdg);
        if xdg.is_absolute() {
            return Ok(normalize(xdg).join(PLATFORM_DIR));
        }
    }

    if let Some(home) = usable_home(env) {
        return Ok(normalize(&home)
            .join(".local")
            .join("share")
            .join(PLATFORM_DIR));
    }

    Err(Error::LocationUnresolved {
        reason: String::from(
            "DPREFIX is unset, XDG_DATA_HOME is unset or not absolute, and HOME is unset or not absolute",
        ),
    })
}

fn usable_home(env: &PrefixEnv) -> Option<PathBuf> {
    let home = env.home.as_deref().filter(|raw| !raw.is_empty())?;
    let home = Path::new(home);
    home.is_absolute().then(|| home.to_path_buf())
}

/// Collapses the lexical shape of a path without touching the filesystem:
/// `//` collapses, `.` disappears, `..` pops a real ancestor, trailing
/// separators are dropped and the root stays `/`.
fn normalize(path: &Path) -> PathBuf {
    let mut rooted = false;
    let mut segments: Vec<OsString> = Vec::new();
    let mut escaping: Vec<OsString> = Vec::new();

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                segments.clear();
                escaping.clear();
                rooted = false;
                segments.push(prefix.as_os_str().to_os_string());
            }
            Component::RootDir => {
                segments.clear();
                escaping.clear();
                rooted = true;
            }
            Component::CurDir => {}
            Component::ParentDir => match segments.pop() {
                Some(_) => {}
                None if !rooted => escaping.push(OsString::from("..")),
                None => {}
            },
            Component::Normal(name) => segments.push(name.to_os_string()),
        }
    }

    let mut normalized = PathBuf::new();
    if rooted {
        normalized.push(Path::new("/"));
    }
    for segment in escaping.iter().chain(segments.iter()) {
        normalized.push(segment);
    }
    normalized
}

fn os_str_byte_len(raw: &std::ffi::OsStr) -> usize {
    use std::os::unix::ffi::OsStrExt;

    raw.as_bytes().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(dprefix: Option<&str>, xdg: Option<&str>, home: Option<&str>) -> PrefixEnv {
        PrefixEnv {
            dprefix: dprefix.map(OsString::from),
            xdg_data_home: xdg.map(OsString::from),
            home: home.map(OsString::from),
        }
    }

    #[test]
    fn uses_an_absolute_dprefix() {
        let location = PrefixLocation::resolve(&env(
            Some("/srv/darling-prefix"),
            Some("/data"),
            Some("/home/dev"),
        ))
        .expect("an absolute DPREFIX must resolve");

        assert_eq!(location.path(), Path::new("/srv/darling-prefix"));
        assert_eq!(location.data_dir(), Path::new("/data/darling-platform"));
        assert_eq!(
            location.lock_path(),
            Path::new("/data/darling-platform/prefix.lock")
        );
        assert_eq!(
            location.state_path(),
            Path::new("/data/darling-platform/state.json")
        );
    }

    #[test]
    fn rejects_a_relative_dprefix() {
        let error = PrefixLocation::resolve(&env(Some("prefix"), None, Some("/home/dev")))
            .expect_err("a relative DPREFIX must be rejected");

        match error {
            Error::PrefixNotAbsolute { path } => assert_eq!(path, PathBuf::from("prefix")),
            other => panic!("expected PrefixNotAbsolute, got {other:?}"),
        }
    }

    #[test]
    fn rejects_the_conventional_darling_prefix() {
        let error = PrefixLocation::resolve(&env(
            Some("/home/dev/.darling"),
            Some("/data"),
            Some("/home/dev"),
        ))
        .expect_err("the conventional prefix must be rejected");

        match error {
            Error::ConventionalPrefixConflict { path } => {
                assert_eq!(path, PathBuf::from("/home/dev/.darling"));
            }
            other => panic!("expected ConventionalPrefixConflict, got {other:?}"),
        }
    }

    #[test]
    fn compares_the_conventional_prefix_after_normalization() {
        let error = PrefixLocation::resolve(&env(
            Some("//home//dev/./.darling/"),
            Some("/data"),
            Some("/home/dev/"),
        ))
        .expect_err("a lexically normalized conventional prefix must be rejected");

        assert!(
            matches!(error, Error::ConventionalPrefixConflict { .. }),
            "expected ConventionalPrefixConflict, got {error:?}"
        );
    }

    #[test]
    fn normalizes_redundant_separators_dots_and_trailing_slashes() {
        let location = PrefixLocation::resolve(&env(
            Some("//srv//darling///prefix/"),
            Some("/data"),
            Some("/home/dev"),
        ))
        .expect("a redundant but absolute DPREFIX must resolve");
        assert_eq!(location.path(), Path::new("/srv/darling/prefix"));

        let location = PrefixLocation::resolve(&env(
            Some("/srv/./darling/../prefix/./"),
            Some("/data"),
            Some("/home/dev"),
        ))
        .expect("a DPREFIX with dots must resolve");
        assert_eq!(location.path(), Path::new("/srv/prefix"));

        let location = PrefixLocation::resolve(&env(Some("/"), Some("/data"), Some("/home/dev")))
            .expect("the root must resolve");
        assert_eq!(location.path(), Path::new("/"));
    }

    #[test]
    fn accepts_255_bytes_and_rejects_256_bytes() {
        let at_limit = format!("/{}", "a".repeat(MAX_PREFIX_PATH_BYTES - 1));
        assert_eq!(at_limit.len(), MAX_PREFIX_PATH_BYTES);
        PrefixLocation::resolve(&env(Some(&at_limit), Some("/data"), Some("/home/dev")))
            .expect("a prefix of exactly the limit must resolve");

        let over_limit = format!("/{}", "a".repeat(MAX_PREFIX_PATH_BYTES));
        assert_eq!(over_limit.len(), MAX_PREFIX_PATH_BYTES + 1);
        let error =
            PrefixLocation::resolve(&env(Some(&over_limit), Some("/data"), Some("/home/dev")))
                .expect_err("a prefix over the limit must be rejected");

        match error {
            Error::PrefixPathTooLong { path, max_bytes } => {
                assert_eq!(path, PathBuf::from(over_limit));
                assert_eq!(max_bytes, MAX_PREFIX_PATH_BYTES);
            }
            other => panic!("expected PrefixPathTooLong, got {other:?}"),
        }
    }

    #[test]
    fn without_dprefix_uses_xdg_data_home() {
        let location = PrefixLocation::resolve(&env(None, Some("/xdg"), Some("/home/dev")))
            .expect("XDG_DATA_HOME must be enough to resolve");

        assert_eq!(location.path(), Path::new("/xdg/darling-platform/prefix"));
        assert_eq!(location.data_dir(), Path::new("/xdg/darling-platform"));
    }

    #[test]
    fn without_dprefix_uses_home_when_xdg_is_unusable() {
        for xdp in [None, Some("relative/xdg"), Some("")] {
            let location = PrefixLocation::resolve(&env(None, xdg, Some("/home/dev")))
                .expect("HOME must be enough to resolve");
            assert_eq!(
                location.path(),
                Path::new("/home/dev/.local/share/darling-platform/prefix")
            );
            assert_eq!(
                location.data_dir(),
                Path::new("/home/dev/.local/share/darling-platform")
            );
        }
    }

    #[test]
    fn without_home_or_xdg_fails_with_location_unresolved() {
        for (xdg, home) in [(None, None), (Some("relative"), None), (None, Some(""))] {
            let error = PrefixLocation::resolve(&env(None, xdg, home))
                .expect_err("an unresolvable environment must fail");
            assert!(
                matches!(error, Error::LocationUnresolved { .. }),
                "expected LocationUnresolved for xdg={xdg:?} home={home:?}, got {error:?}"
            );
        }
    }

    #[test]
    fn data_dir_never_nests_inside_the_prefix_path() {
        let default = PrefixLocation::resolve(&env(None, Some("/xdg"), Some("/home/dev"))).unwrap();
        assert!(
            !default.data_dir().starts_with(default.path()),
            "data dir {} must stay outside the prefix {}",
            default.data_dir().display(),
            default.path().display()
        );

        let explicit =
            PrefixLocation::resolve(&env(Some("/srv/dp"), Some("/xdg"), Some("/home/dev")))
                .unwrap();
        assert!(!explicit.data_dir().starts_with(explicit.path()));
    }
}
