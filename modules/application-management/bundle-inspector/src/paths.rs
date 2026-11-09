//! Containment of the paths a bundle declares.
//!
//! `Info.plist` and the bundle's own file tree are untrusted input. Every path
//! the bundle declares is proven to stay inside the bundle twice: lexically, by
//! rejecting absolute values and `..` components, and physically, by resolving
//! symlinks and comparing the canonical target against the canonical bundle
//! root. A bundle that points outside itself is rejected before anything is
//! copied or executed.

use std::path::{Component, Path, PathBuf};

use crate::{Error, Result};

/// Joins a declared relative value onto the `base` components below `bundle`,
/// after proving it cannot leave the bundle lexically.
///
/// `rooted` covers both a leading separator (`/usr/bin/true`) and a Windows
/// prefix (`C:\Windows`): either one would replace the base when joined.
pub(crate) fn declared(
    bundle: &Path,
    base: &[&str],
    role: &'static str,
    value: &str,
) -> Result<PathBuf> {
    let relative = Path::new(value);
    if relative.is_absolute()
        || relative.has_root()
        || relative
            .components()
            .any(|component| matches!(component, Component::Prefix(_)))
    {
        return Err(Error::AbsolutePath {
            bundle: bundle.to_path_buf(),
            role,
            value: value.to_string(),
        });
    }
    if relative
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(Error::ParentTraversal {
            bundle: bundle.to_path_buf(),
            role,
            value: value.to_string(),
        });
    }
    let mut path = bundle.to_path_buf();
    for component in base {
        path.push(component);
    }
    Ok(path.join(relative))
}

/// Proves that the existing `path` still sits inside `root`, the canonical
/// bundle directory, once every symlink along the way is resolved.
///
/// `role` and `value` are the declaration the path came from, so the error names
/// what the bundle asked for rather than an internal path.
pub(crate) fn verify(
    root: &Path,
    bundle: &Path,
    role: &'static str,
    value: &str,
    path: &Path,
) -> Result<()> {
    let target = path.canonicalize().map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if target.starts_with(root) {
        return Ok(());
    }
    Err(Error::EscapingSymlink {
        bundle: bundle.to_path_buf(),
        role,
        value: value.to_string(),
        target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTENTS: &str = "Contents";
    const MACOS: &str = "MacOS";

    fn bundle() -> PathBuf {
        PathBuf::from("/Applications/Example.app")
    }

    #[test]
    fn a_relative_value_is_joined_onto_the_declared_base() {
        let joined = declared(
            &bundle(),
            &[CONTENTS, MACOS],
            "CFBundleExecutable",
            "Example",
        )
        .expect("relative value");

        assert_eq!(joined, bundle().join(CONTENTS).join(MACOS).join("Example"));
    }

    #[test]
    fn a_rooted_value_is_rejected_on_every_platform() {
        let error = declared(
            &bundle(),
            &[CONTENTS, MACOS],
            "CFBundleExecutable",
            "/usr/bin/true",
        )
        .expect_err("a rooted path leaves the bundle");

        assert!(
            matches!(error, Error::AbsolutePath { .. }),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn a_windows_prefixed_value_is_rejected() {
        let error = declared(
            &bundle(),
            &[CONTENTS, MACOS],
            "CFBundleExecutable",
            r"C:\Windows\System32\calc.exe",
        )
        .expect_err("a prefixed path leaves the bundle");

        assert!(
            matches!(
                error,
                Error::AbsolutePath { .. } | Error::ParentTraversal { .. }
            ),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn a_parent_component_is_rejected() {
        let error = declared(
            &bundle(),
            &[CONTENTS, "Resources"],
            "CFBundleIconFile",
            "Resources/../../escape.icns",
        )
        .expect_err("'..' leaves the bundle");

        assert!(
            matches!(error, Error::ParentTraversal { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            error.to_string().contains("Resources/../../escape.icns"),
            "the message must name the offending value: {error}"
        );
    }

    #[test]
    fn a_single_dot_component_is_allowed() {
        let joined = declared(
            &bundle(),
            &[CONTENTS, MACOS],
            "CFBundleExecutable",
            "./Example",
        )
        .expect("harmless");

        assert_eq!(
            joined,
            bundle()
                .join(CONTENTS)
                .join(MACOS)
                .join(".")
                .join("Example")
        );
    }

    #[test]
    fn verify_accepts_a_path_inside_the_bundle() {
        let root = tempfile::tempdir().expect("create the temporary directory");
        let path = root.path().join("Contents").join("MacOS").join("Example");
        std::fs::create_dir_all(path.parent().expect("the file lives in a directory"))
            .expect("create the directory");
        std::fs::write(&path, b"binary").expect("write the file");
        let root = root.path().canonicalize().expect("canonicalize the root");

        verify(&root, &root, "CFBundleExecutable", "Example", &path).expect("inside the bundle");
    }

    #[test]
    fn verify_rejects_a_missing_path_as_an_io_error() {
        let root = tempfile::tempdir().expect("create the temporary directory");
        let path = root.path().join("Contents").join("MacOS").join("Absent");
        let root = root.path().canonicalize().expect("canonicalize the root");

        let error = verify(&root, &root, "CFBundleExecutable", "Absent", &path)
            .expect_err("a missing path cannot be proven contained");

        assert!(
            matches!(error, Error::Io { .. }),
            "unexpected error: {error:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn verify_rejects_a_symlink_that_escapes_the_bundle() {
        let root = tempfile::tempdir().expect("create the temporary directory");
        let outside = tempfile::tempdir().expect("create the outside directory");
        let target = outside.path().join("Escaped");
        std::fs::write(&target, b"binary").expect("write the outside file");
        let link = root.path().join("Contents").join("MacOS").join("Example");
        std::fs::create_dir_all(link.parent().expect("the link lives in a directory"))
            .expect("create the directory");
        std::os::unix::fs::symlink(&target, &link).expect("create the symlink");
        let root = root.path().canonicalize().expect("canonicalize the root");

        let error = verify(&root, &root, "CFBundleExecutable", "Example", &link)
            .expect_err("the link leaves the bundle");

        assert!(
            matches!(error, Error::EscapingSymlink { .. }),
            "unexpected error: {error:?}"
        );
    }
}
