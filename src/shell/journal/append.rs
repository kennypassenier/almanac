//! Appending one record, fsynced before the caller may acknowledge it.

use std::io::Write;
use std::path::Path;

use crate::core::error::AlmanacError;
use crate::core::journal::Record;

use super::Journal;

impl Journal {
    /// Appends one record and fsyncs it. Returns only once the bytes
    /// are durably on disk — the caller may not acknowledge the source
    /// before this resolves.
    pub(super) async fn append(&self, record: &Record) -> Result<(), AlmanacError> {
        let mut line = serde_json::to_string(record).map_err(|e| AlmanacError::Config {
            message: format!("failed to serialize a journal record: {e}"),
            remedy: "this is a bug in almanac; the payload could not be re-encoded".to_string(),
        })?;
        line.push('\n');

        let _guard = self.write_lock.lock().await;
        // feat-backup-1 (chassis-rs 3.1.0): waits out a backup pause before
        // touching the file, so a nightly `tar` of the state root never
        // races this append. Held until the fsync below returns.
        let _ticket = chassis::shell::backup::writing().await;

        ensure_parent_dir(&self.path)?;
        self.check_cap(line.len() as u64)?;
        append_synced(&self.path, line.as_bytes())
    }

    /// Refuses an append that would take the file past its cap.
    fn check_cap(&self, adding: u64) -> Result<(), AlmanacError> {
        let existing = match std::fs::metadata(&self.path) {
            Ok(meta) => meta.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
            // Before, any failure read as an empty file and the cap was
            // never checked; a journal that cannot be measured is one
            // that cannot be trusted to stay under it.
            Err(e) => {
                return Err(AlmanacError::Config {
                    message: format!("failed to measure journal {}: {e}", self.path.display()),
                    remedy: format!("check permissions on {}", self.path.display()),
                });
            }
        };
        if existing + adding > self.max_bytes {
            return Err(AlmanacError::Config {
                message: format!(
                    "journal {} would exceed its {} byte cap",
                    self.path.display(),
                    self.max_bytes
                ),
                remedy:
                    "delivery to Google has been failing long enough to fill the journal — check \
                     the logs for the underlying error before it is safe to drain or clear it"
                        .to_string(),
            });
        }
        Ok(())
    }
}

fn ensure_parent_dir(path: &Path) -> Result<(), AlmanacError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| AlmanacError::Config {
            message: format!(
                "failed to create journal directory {}: {e}",
                parent.display()
            ),
            remedy: format!("check permissions on {}", parent.display()),
        })?;
    }
    Ok(())
}

/// Appends `bytes` and fsyncs the file.
fn append_synced(path: &Path, bytes: &[u8]) -> Result<(), AlmanacError> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| AlmanacError::Config {
            message: format!("failed to open journal {}: {e}", path.display()),
            remedy: format!("check permissions on {}", path.display()),
        })?;

    file.write_all(bytes).map_err(|e| AlmanacError::Config {
        message: format!("failed to write to journal {}: {e}", path.display()),
        remedy: "check free disk space and permissions".to_string(),
    })?;

    // The whole point: without this the OS may still be holding
    // the bytes in cache when the power goes, and the source has
    // already been told the request succeeded.
    file.sync_all().map_err(|e| AlmanacError::Config {
        message: format!("failed to fsync journal {}: {e}", path.display()),
        remedy: "the filesystem rejected a durability barrier; check disk health".to_string(),
    })
}
