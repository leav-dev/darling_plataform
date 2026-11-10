//! Safe extraction of a `.zip` import.
//!
//! A ZIP archive is untrusted input: its entry names are attacker-controlled
//! bytes and its declared sizes are a claim, not a fact. Every entry is
//! therefore validated before a single byte is written, and the bytes that are
//! written are counted, so a declaration cannot stand in for the truth.
//!
//! # Limits
//!
//! * At most [`MAX_ENTRIES`] entries.
//! * At most [`MAX_UNCOMPRESSED_BYTES`] uncompressed bytes in total, checked
//!   both against the size the central directory declares and against the bytes
//!   actually copied.
//!
//! # Refused entries
//!
//! An absolute path, a `..` component, a NUL byte, an entry the zip reader
//! cannot enclose in the staging area, and a symbolic link entry are all
//! rejected, and the error names the archive and the entry. Symbolic links are
//! refused rather than recreated because a link target is resolved when the file
//! is used, which is after every check this installer can run.
//!
//! # Written entries
//!
//! Directories are created explicitly and the parent of every file is created
//! before the file, so an archive without directory entries still extracts.
//! Nothing is extracted outside the staging area the installer owns, and the
//! caller removes that area whether the extraction finishes or fails.

use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};

use zip::ZipArchive;
use zip::read::ZipFile;

use crate::error::{Error, Result};

/// Most entries one archive may declare.
///
/// A macOS application bundle holds thousands of files; an archive that declares
/// more than this is not one application and would exhaust the inode table of
/// the destination before the install could be validated.
pub(crate) const MAX_ENTRIES: usize = 200_000;

/// Most uncompressed bytes one archive may write: 8 GiB.
///
/// This is the order of the largest application bundles a workstation installs,
/// and it bounds what an archive can cost the disk even when the compressed
/// archive itself is small.
pub(crate) const MAX_UNCOMPRESSED_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// `S_IFMT`: the bits of a unix mode that name the file type.
const S_IFMT: u32 = 0o170_000;

/// `S_IFLNK`: the unix mode bits of a symbolic link.
const S_IFLNK: u32 = 0o120_000;

/// Extracts `archive` into `destination`, which must already exist.
pub(crate) fn extract(archive: &Path, destination: &Path) -> Result<()> {
    let file = File::open(archive).map_err(|source| Error::Io {
        path: archive.to_path_buf(),
        source,
    })?;
    let mut zip = ZipArchive::new(file).map_err(|source| Error::ArchiveUnreadable {
        archive: archive.to_path_buf(),
        source,
    })?;
    check_limits(archive, &zip)?;

    let mut written: u64 = 0;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|source| Error::ArchiveUnreadable {
                archive: archive.to_path_buf(),
                source,
            })?;
        let target = destination.join(entry_path(archive, &entry)?);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|source| Error::Io {
                path: target.clone(),
                source,
            })?;
            continue;
        }
        write_entry(archive, &mut entry, &target, &mut written)?;
    }
    Ok(())
}

/// What makes an archive entry unsafe to extract.
///
/// Kept separate from [`Error`] so the name can be inspected before the error is
/// built, which is what lets the error own the entry name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryProblem {
    /// The name holds a NUL byte, which truncates a path at the syscall.
    Nul,
    /// The name is rooted or carries a prefix, so joining it leaves the area.
    Absolute,
    /// The name holds a `..` component, which walks out of the area.
    Traversal,
    /// The name is a symbolic link entry.
    Symlink,
}

impl EntryProblem {
    /// Builds the error this problem is reported as.
    fn error(self, archive: &Path, entry: String) -> Error {
        let archive = archive.to_path_buf();
        match self {
            Self::Nul => Error::ArchiveEntryNul { archive, entry },
            Self::Absolute => Error::ArchiveEntryAbsolute { archive, entry },
            Self::Traversal => Error::ArchiveEntryTraversal { archive, entry },
            Self::Symlink => Error::ArchiveEntrySymlink { archive, entry },
        }
    }
}

/// Reports what makes `entry` unsafe, or nothing when it is extractable.
fn entry_problem(entry: &ZipFile<'_, File>) -> Option<EntryProblem> {
    let name = entry_name(entry);
    if name.contains('\0') {
        return Some(EntryProblem::Nul);
    }
    let path = Path::new(&name);
    if path.is_absolute()
        || path.has_root()
        || path
            .components()
            .any(|component| matches!(component, Component::Prefix(_)))
    {
        return Some(EntryProblem::Absolute);
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Some(EntryProblem::Traversal);
    }
    // The unix mode is the only place a ZIP can say "symlink"; the reader's own
    // predicate reads the same bits, and both are checked so neither a mode
    // written by a DOS packer nor a mode read by a stricter reader is missed.
    let mode_says_link = entry
        .unix_mode()
        .is_some_and(|mode| mode & S_IFMT == S_IFLNK);
    if entry.is_symlink() || mode_says_link {
        return Some(EntryProblem::Symlink);
    }
    None
}

/// Returns the path `entry` may be written to, or the error that names it.
///
/// The path is always relative: it is joined onto the staging directory by the
/// caller, never used on its own.
fn entry_path(archive: &Path, entry: &ZipFile<'_, File>) -> Result<PathBuf> {
    let name = entry_name(entry);
    if let Some(problem) = entry_problem(entry) {
        return Err(problem.error(archive, name));
    }
    entry
        .enclosed_name()
        .ok_or_else(|| Error::ArchiveEntryUnsafe {
            archive: archive.to_path_buf(),
            entry: name,
        })
}

/// Renders an entry name for a message. Names are not required to be UTF-8, so a
/// name that is not is shown lossily instead of being replaced by a placeholder.
fn entry_name(entry: &ZipFile<'_, File>) -> String {
    String::from_utf8_lossy(entry.name_raw()).into_owned()
}

/// Reports the entry count and the declared size as unsatisfiable, if they are.
fn check_limits(archive: &Path, zip: &ZipArchive<File>) -> Result<()> {
    let count = zip.len();
    if count > MAX_ENTRIES {
        return Err(Error::ArchiveTooManyEntries {
            archive: archive.to_path_buf(),
            found: count,
            limit: MAX_ENTRIES,
        });
    }
    // `decompressed_size` is `None` when an entry hides its size behind a data
    // descriptor; the copied bytes are counted as well, so that case is still
    // bounded.
    if let Some(declared) = zip.decompressed_size()
        && declared > u128::from(MAX_UNCOMPRESSED_BYTES)
    {
        return Err(Error::ArchiveTooLarge {
            archive: archive.to_path_buf(),
            found: u64::try_from(declared).unwrap_or(u64::MAX),
            limit: MAX_UNCOMPRESSED_BYTES,
        });
    }
    Ok(())
}

/// Copies one entry to `target`, counting the bytes it writes.
fn write_entry(
    archive: &Path,
    entry: &mut ZipFile<'_, File>,
    target: &Path,
    written: &mut u64,
) -> Result<()> {
    if let Some(parent) = target.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut file = File::create(target).map_err(|source| Error::Io {
        path: target.to_path_buf(),
        source,
    })?;
    let copied = io::copy(entry, &mut file).map_err(|source| Error::Io {
        path: target.to_path_buf(),
        source,
    })?;
    *written = written.saturating_add(copied);
    if *written > MAX_UNCOMPRESSED_BYTES {
        return Err(Error::ArchiveTooLarge {
            archive: archive.to_path_buf(),
            found: *written,
            limit: MAX_UNCOMPRESSED_BYTES,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::{CPU_TYPE_X86_64, entry, thin_macho, write_zip};

    fn staging() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    #[test]
    fn a_stored_archive_is_extracted_with_its_directories() {
        let directory = staging();
        let archive = directory.path().join("Example.zip");
        let destination = directory.path().join("staging");
        std::fs::create_dir(&destination).expect("create the staging directory");
        write_zip(
            &archive,
            &[
                entry("Example.app/Contents/Info.plist", b"<plist/>"),
                entry(
                    "Example.app/Contents/MacOS/Example",
                    &thin_macho(CPU_TYPE_X86_64),
                ),
            ],
        );

        extract(&archive, &destination).expect("the archive is extracted");

        assert!(
            destination
                .join("Example.app")
                .join("Contents")
                .join("Info.plist")
                .is_file()
        );
        assert!(
            destination
                .join("Example.app")
                .join("Contents")
                .join("MacOS")
                .join("Example")
                .is_file()
        );
    }

    #[test]
    fn the_declared_limits_are_the_documented_ones() {
        assert_eq!(MAX_ENTRIES, 200_000);
        assert_eq!(MAX_UNCOMPRESSED_BYTES, 8 * 1024 * 1024 * 1024);
    }
}
