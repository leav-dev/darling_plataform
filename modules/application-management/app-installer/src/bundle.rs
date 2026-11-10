//! Reading a source `.app` bundle into the staging area and finding the bundle
//! inside a staged import.
//!
//! # Symlink policy
//!
//! A source bundle is copied as a tree: directories are created, regular files
//! are copied and symbolic links are recreated verbatim, because real bundles
//! rely on them — `Versions/Current`, `Resources/Current` and the framework
//! aliases that hold an install together — and dereferencing them would silently
//! duplicate megabytes of a framework.
//!
//! A link is only recreated after its canonical target is proven to stay inside
//! the source bundle. A link that resolves outside would let a bundle reference
//! any file on the host, so it is rejected and the error names the link and its
//! target. On a host that cannot represent a symbolic link the copy is rejected
//! instead of installing a bundle whose links became files.
//!
//! ZIP archives take the opposite policy on purpose: an entry that is a link is
//! refused outright, because the archive is the untrusted input and there is no
//! reason for an archive of a bundle to carry one. See [`crate::archive`].

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Suffix of a macOS application bundle.
const BUNDLE_SUFFIX: &str = ".app";

/// Directory macOS writes next to the bundle when it archives one by hand. It
/// holds AppleDouble resource forks, never a second bundle, so the search for
/// the staged bundle does not descend into it.
const APPLEDOUBLE: &str = "__MACOSX";

/// Copies the `.app` bundle at `bundle` into `staging`, keeping its own name.
pub(crate) fn copy(bundle: &Path, staging: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(bundle).map_err(|source| Error::Io {
        path: bundle.to_path_buf(),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(Error::BundleNotDirectory {
            path: bundle.to_path_buf(),
        });
    }
    let Some(name) = bundle.file_name() else {
        return Err(Error::InvalidBundleName {
            declared: bundle.display().to_string(),
        });
    };
    let root = bundle.canonicalize().map_err(|source| Error::Io {
        path: bundle.to_path_buf(),
        source,
    })?;
    copy_entry(bundle, &staging.join(name), &root)
}

/// Finds the single `.app` bundle staged under `staging`.
///
/// The search is recursive, because a ZIP may hold the bundle inside a folder,
/// but it never descends into a bundle it found: a framework or a helper bundle
/// nested inside an application is part of that application, not a second
/// candidate. Zero bundles and several bundles are both errors, and the error
/// names what was staged.
pub(crate) fn single(staging: &Path) -> Result<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    collect(staging, &mut found)?;
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(Error::NoBundleInStaging {
            staging: staging.to_path_buf(),
        }),
        _ => Err(Error::SeveralBundlesInStaging {
            staging: staging.to_path_buf(),
            found: found
                .iter()
                .map(|path| {
                    path.strip_prefix(staging)
                        .unwrap_or(path)
                        .display()
                        .to_string()
                })
                .collect(),
        }),
    }
}

/// Collects the bundles below `directory` into `found`.
fn collect(directory: &Path, found: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries: Vec<(PathBuf, std::fs::FileType)> = Vec::new();
    for entry in std::fs::read_dir(directory).map_err(|source| Error::Io {
        path: directory.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| Error::Io {
            path: directory.to_path_buf(),
            source,
        })?;
        let file_type = entry.file_type().map_err(|source| Error::Io {
            path: entry.path(),
            source,
        })?;
        entries.push((entry.path(), file_type));
    }
    // Sorted so a source holding several bundles is always reported the same
    // way, whatever order the file system hands them over in.
    entries.sort_by(|left, right| left.0.cmp(&right.0));

    for (path, file_type) in entries {
        let name = path.file_name().unwrap_or_else(|| OsStr::new(""));
        if name == OsStr::new(APPLEDOUBLE) {
            continue;
        }
        // A link is not followed here: the copy recreates links, so a link to a
        // directory never becomes a candidate bundle of its own.
        if !file_type.is_dir() {
            continue;
        }
        if name.to_string_lossy().ends_with(BUNDLE_SUFFIX) {
            found.push(path);
            continue;
        }
        collect(&path, found)?;
    }
    Ok(())
}

/// Copies one directory entry of a source bundle, validating links as it goes.
fn copy_entry(source: &Path, target: &Path, root: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(source).map_err(|error| Error::Io {
        path: source.to_path_buf(),
        source: error,
    })?;
    if metadata.file_type().is_symlink() {
        return copy_symlink(source, target, root);
    }
    if metadata.is_dir() {
        std::fs::create_dir_all(target).map_err(|error| Error::Io {
            path: target.to_path_buf(),
            source: error,
        })?;
        let mut entries: Vec<PathBuf> = Vec::new();
        for entry in std::fs::read_dir(source).map_err(|error| Error::Io {
            path: source.to_path_buf(),
            source: error,
        })? {
            let entry = entry.map_err(|error| Error::Io {
                path: source.to_path_buf(),
                source: error,
            })?;
            entries.push(entry.path());
        }
        // Sorted so a failure always names the same entry first.
        entries.sort();
        for path in entries {
            let name = path
                .file_name()
                .ok_or_else(|| Error::UnsupportedEntry { path: path.clone() })?;
            copy_entry(&path, &target.join(name), root)?;
        }
        return Ok(());
    }
    // A socket, a FIFO or a device node is not part of a bundle, and copying one
    // could block forever instead of failing.
    if !metadata.is_file() {
        return Err(Error::UnsupportedEntry {
            path: source.to_path_buf(),
        });
    }
    std::fs::copy(source, target)
        .map(|_| ())
        .map_err(|error| Error::Io {
            path: target.to_path_buf(),
            source: error,
        })
}

/// Copies one symbolic link after proving its target stays inside the bundle.
fn copy_symlink(source: &Path, target: &Path, root: &Path) -> Result<()> {
    let resolved = source.canonicalize().map_err(|error| Error::Io {
        path: source.to_path_buf(),
        source: error,
    })?;
    if !resolved.starts_with(root) {
        return Err(Error::SymlinkEscapes {
            bundle: root.to_path_buf(),
            path: source.to_path_buf(),
            target: resolved,
        });
    }
    recreate_symlink(source, target)
}

/// Recreates the link at `target` with the same target text it had at `source`.
#[cfg(unix)]
fn recreate_symlink(source: &Path, target: &Path) -> Result<()> {
    let link_target = std::fs::read_link(source).map_err(|error| Error::Io {
        path: source.to_path_buf(),
        source: error,
    })?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|error| Error::Io {
            path: parent.to_path_buf(),
            source: error,
        })?;
    }
    std::os::unix::fs::symlink(&link_target, target).map_err(|error| Error::Io {
        path: target.to_path_buf(),
        source: error,
    })
}

/// Rejects the copy on a host that cannot represent a symbolic link, rather than
/// installing a bundle whose links silently became copies or were dropped.
#[cfg(not(unix))]
fn recreate_symlink(source: &Path, _target: &Path) -> Result<()> {
    Err(Error::SymlinkUnsupported {
        path: source.to_path_buf(),
    })
}
