//! An exclusive lock on the data directory (AR22).
//!
//! The journal's per-key serialization (AR16) lives in one process's
//! memory. During a self-update handover two processes exist at once,
//! and a second one draining the same journal would produce exactly
//! the duplicate events the journal exists to prevent — worse, a
//! concurrent compaction can rename a rewritten file over the other's
//! appends and lose the done-markers, replaying already-delivered
//! entries.
//!
//! An `flock` on a file in the data directory makes that impossible:
//! the second process finds the lock held and refuses to touch the
//! journal until the first has exited.

use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::Path;

use crate::core::error::AlmanacError;

/// Held for the life of the process. Dropping it (including on a
/// crash, since the kernel releases the lock when the fd closes)
/// releases the directory for the next process.
#[derive(Debug)]
pub struct DataDirLock {
    _file: File,
}

impl DataDirLock {
    /// Takes the lock, or fails naming what already holds it.
    ///
    /// Deliberately non-blocking: a self-update handover should report
    /// "the old process is still running" immediately rather than hang
    /// waiting for a process that may never exit.
    pub fn acquire(dir: &Path) -> Result<Self, AlmanacError> {
        std::fs::create_dir_all(dir).map_err(|e| AlmanacError::Config {
            message: format!("failed to create the data directory {}: {e}", dir.display()),
            remedy: format!("check permissions on {}", dir.display()),
        })?;

        let path = dir.join(".lock");
        let file = File::create(&path).map_err(|e| AlmanacError::Config {
            message: format!("failed to open the lock file {}: {e}", path.display()),
            remedy: format!("check permissions on {}", dir.display()),
        })?;

        // SAFETY: a plain libc call on a file descriptor this function
        // owns; flock has no preconditions beyond a valid fd.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            return Err(AlmanacError::Config {
                message: format!(
                    "another almanac process already holds {}: {err}",
                    path.display()
                ),
                remedy: "stop the running instance first — two processes sharing one journal \
                         would deliver the same event twice and can lose delivery records \
                         (AR22)"
                    .to_string(),
            });
        }

        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "almanac-lock-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_first_holder_gets_the_lock() {
        let dir = scratch("first");
        assert!(DataDirLock::acquire(&dir).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_second_holder_is_refused_and_told_why() {
        // The whole point: this is what stops a self-update handover
        // from running two workers over one journal.
        let dir = scratch("second");
        let _first = DataDirLock::acquire(&dir).unwrap();

        let err = DataDirLock::acquire(&dir).unwrap_err();
        assert!(err.to_string().contains("already holds"));
        assert!(err.remedy().contains("twice"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn releasing_the_lock_lets_the_next_process_in() {
        let dir = scratch("release");
        {
            let _held = DataDirLock::acquire(&dir).unwrap();
        }
        assert!(
            DataDirLock::acquire(&dir).is_ok(),
            "the lock must not outlive its holder"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_directory_is_created_rather_than_refused() {
        let dir = scratch("missing").join("nested").join("data");
        assert!(DataDirLock::acquire(&dir).is_ok());
        std::fs::remove_dir_all(dir.parent().unwrap().parent().unwrap()).ok();
    }
}
