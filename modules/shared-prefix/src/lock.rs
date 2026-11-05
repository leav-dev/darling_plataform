//! Advisory read/write lock over the shared prefix.
//!
//! Darling accepts one lifecycle operation at a time, so the platform serialises
//! them on a dedicated lock file that lives in the platform data directory, never
//! inside the prefix: the lock must stay meaningful while Darling replaces the
//! prefix's overlay.
//!
//! `flock` locks belong to the open file description, so two separate `File`
//! handles conflict even inside a single process: a careless caller cannot nest a
//! second exclusive operation in the same process.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use rustix::fs::FlockOperation;
use rustix::io::Errno;

use crate::error::{Error, Result};

/// An acquired prefix lock, released when dropped.
///
/// `flock` semantics are per open file description, so two separate [`File`]
/// handles conflict even inside a single process: nesting two exclusive
/// operations in one process is refused just like nesting them across processes.
/// The lock is released when this value is dropped, which unlocks and then closes
/// the lock file.
#[derive(Debug)]
pub struct PrefixLock {
    path: PathBuf,
    file: File,
}

impl PrefixLock {
    /// Acquires a shared lock, waiting until it is available.
    pub fn acquire_shared(path: &Path) -> Result<Self> {
        Self::acquire(path, LockKind::Shared)
    }

    /// Acquires an exclusive lock, waiting until it is available.
    pub fn acquire_exclusive(path: &Path) -> Result<Self> {
        Self::acquire(path, LockKind::Exclusive)
    }

    /// Tries to acquire an exclusive lock without waiting.
    ///
    /// Returns `Ok(None)` when any other handle holds a lock on the file: the call
    /// never blocks.
    pub fn try_exclusive(path: &Path) -> Result<Option<Self>> {
        let file = open_lock_file(path)?;
        match rustix::fs::flock(&file, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => Ok(Some(Self {
                path: path.to_path_buf(),
                file,
            })),
            Err(errno) if is_busy(errno) => Ok(None),
            Err(errno) => Err(io_error(path, errno)),
        }
    }

    /// Reports whether another handle currently holds a lock on `path`.
    ///
    /// Unlike [`PrefixLock::try_exclusive`], this never creates the lock file or
    /// its parents: an absent file means no operation is running. A read-only
    /// command such as `dp prefix status` therefore leaves no trace on disk.
    pub fn probe_busy(path: &Path) -> Result<bool> {
        let file = match OpenOptions::new().read(true).write(true).open(path) {
            Ok(file) => file,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(source) => {
                return Err(Error::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };

        match rustix::fs::flock(&file, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => {
                let _ = rustix::fs::flock(&file, FlockOperation::Unlock);
                Ok(false)
            }
            Err(errno) if is_busy(errno) => Ok(true),
            Err(errno) => Err(io_error(path, errno)),
        }
    }

    /// Path of the lock file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn acquire(path: &Path, kind: LockKind) -> Result<Self> {
        let file = open_lock_file(path)?;
        let operation = match kind {
            LockKind::Shared => FlockOperation::LockShared,
            LockKind::Exclusive => FlockOperation::LockExclusive,
        };
        rustix::fs::flock(&file, operation).map_err(|errno| io_error(path, errno))?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
        })
    }
}

impl Drop for PrefixLock {
    fn drop(&mut self) {
        // Closing the descriptor would release the lock anyway; unlocking first
        // keeps the release explicit and makes `Drop` the documented release
        // point. A failure here cannot change what closing the file does.
        let _ = rustix::fs::flock(&self.file, FlockOperation::Unlock);
    }
}

#[derive(Debug, Clone, Copy)]
enum LockKind {
    Shared,
    Exclusive,
}

/// Opens the lock file for read/write, creating it and its parents if needed.
///
/// The file needs both permissions because `flock` requires a write-capable
/// descriptor for an exclusive lock.
fn open_lock_file(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })
}

fn is_busy(errno: Errno) -> bool {
    errno == Errno::WOULDBLOCK || errno == Errno::AGAIN
}

fn io_error(path: &Path, errno: Errno) -> Error {
    Error::Io {
        path: path.to_path_buf(),
        source: std::io::Error::from(errno),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, mpsc};
    use std::time::Duration;

    use super::*;

    fn lock_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("darling-platform").join("prefix.lock")
    }

    #[test]
    fn probe_busy_does_not_create_the_lock_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = lock_path(&dir);

        assert!(
            !PrefixLock::probe_busy(&path).expect("probe on an absent file"),
            "an absent lock file means no operation is running"
        );
        assert!(!path.exists(), "the probe must not create the lock file");
        assert!(
            !dir.path().join("darling-platform").exists(),
            "the probe must not create the lock directory either"
        );
    }

    #[test]
    fn probe_busy_reports_a_held_lock_and_clears_after_release() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = lock_path(&dir);

        let held = PrefixLock::acquire_shared(&path).expect("shared lock");
        assert!(
            PrefixLock::probe_busy(&path).expect("probe while held"),
            "a shared holder makes the exclusive probe busy"
        );

        drop(held);
        assert!(
            !PrefixLock::probe_busy(&path).expect("probe after release"),
            "a released lock stops being busy"
        );
    }

    #[test]
    fn shared_and_shared_do_not_conflict() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = lock_path(&dir);

        let first = PrefixLock::acquire_shared(&path).expect("first shared lock");
        let second = PrefixLock::acquire_shared(&path).expect("second shared lock");

        assert_eq!(first.path(), path);
        assert_eq!(second.path(), path);
    }

    #[test]
    fn exclusive_blocks_a_blocking_shared_acquire() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = lock_path(&dir);
        let held = PrefixLock::acquire_exclusive(&path).expect("exclusive lock");

        let (sender, receiver) = mpsc::channel();
        let waiter_path = path.clone();
        let waiter = std::thread::spawn(move || {
            let acquired = PrefixLock::acquire_shared(&waiter_path);
            let _ = sender.send(acquired.is_ok());
            acquired
        });

        std::thread::sleep(Duration::from_millis(150));
        assert!(
            matches!(receiver.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "a shared acquire must wait while an exclusive lock is held"
        );

        drop(held);
        assert!(
            receiver
                .recv_timeout(Duration::from_secs(10))
                .expect("sent"),
            "the shared acquire must succeed once the exclusive lock is released"
        );
        waiter
            .join()
            .expect("waiter thread")
            .expect("shared lock must be held");
    }

    #[test]
    fn exclusive_blocks_a_blocking_exclusive_acquire() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = lock_path(&dir);
        let held = PrefixLock::acquire_exclusive(&path).expect("exclusive lock");

        assert!(
            PrefixLock::try_exclusive(&path)
                .expect("try must not fail")
                .is_none(),
            "a second exclusive lock must not be granted"
        );

        let (sender, receiver) = mpsc::channel();
        let waiter_path = path.clone();
        let waiter = std::thread::spawn(move || {
            let acquired = PrefixLock::acquire_exclusive(&waiter_path);
            let _ = sender.send(acquired.is_ok());
            acquired
        });

        std::thread::sleep(Duration::from_millis(150));
        assert!(
            matches!(receiver.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "a second exclusive acquire must wait"
        );

        drop(held);
        assert!(
            receiver
                .recv_timeout(Duration::from_secs(10))
                .expect("sent"),
            "the exclusive acquire must succeed once the first is released"
        );
        waiter
            .join()
            .expect("waiter thread")
            .expect("exclusive lock must be held");
    }

    #[test]
    fn try_exclusive_reports_busy_then_succeeds_after_release() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = lock_path(&dir);

        let shared = PrefixLock::acquire_shared(&path).expect("shared lock");
        assert!(
            PrefixLock::try_exclusive(&path)
                .expect("try must not fail")
                .is_none(),
            "try_exclusive must not steal a lock held in shared mode"
        );

        drop(shared);
        let exclusive = PrefixLock::try_exclusive(&path)
            .expect("try must not fail")
            .expect("the lock must be free after the shared lock is dropped");
        assert_eq!(exclusive.path(), path);
    }

    #[test]
    fn a_released_lock_can_be_acquired_again() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = lock_path(&dir);

        let first = PrefixLock::acquire_exclusive(&path).expect("first lock");
        drop(first);
        let second = PrefixLock::acquire_exclusive(&path).expect("lock after release");
        drop(second);
        let third = PrefixLock::acquire_shared(&path).expect("shared lock after release");
        assert_eq!(third.path(), path);
    }

    #[test]
    fn two_threads_never_both_get_the_exclusive_lock() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = lock_path(&dir);
        let start = Arc::new(Barrier::new(3));
        let overlap = Arc::new(Barrier::new(2));
        let winners = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..2 {
            let path = path.clone();
            let start = Arc::clone(&start);
            let overlap = Arc::clone(&overlap);
            let winners = Arc::clone(&winners);
            handles.push(std::thread::spawn(move || {
                start.wait();
                let held = PrefixLock::try_exclusive(&path).expect("try must not fail");
                if held.is_some() {
                    winners.fetch_add(1, Ordering::SeqCst);
                }
                // Hold the lock until the other thread has made its attempt.
                overlap.wait();
                drop(held);
            }));
        }

        start.wait();
        for handle in handles {
            handle.join().expect("thread");
        }

        assert_eq!(
            winners.load(Ordering::SeqCst),
            1,
            "exactly one thread may hold the exclusive lock"
        );
    }

    #[test]
    fn creates_missing_parent_directories() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("a").join("b").join("c").join("prefix.lock");
        assert!(!path.parent().expect("parent").exists());

        let lock = PrefixLock::acquire_shared(&path).expect("shared lock");

        assert!(path.exists(), "the lock file must be created");
        assert!(lock.path().parent().expect("parent").is_dir());
    }
}
