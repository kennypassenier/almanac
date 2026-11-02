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
        // Only a new entry counts against the cap: a Done or Dead marker is
        // what lets compaction shrink the file, so refusing it would keep a
        // full journal full and redeliver the same entries every pass.
        if matches!(record, Record::Entry(_)) {
            self.check_cap(line.len() as u64)?;
        }
        repair_torn_tail(&self.path)?;
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

/// Cuts a torn final line (a crash mid-append) off before the next append.
/// Left in place, the next record would be glued onto it and the file
/// would hold an unparseable line in the middle, which no read tolerates.
/// The torn record was never acknowledged, so cutting it loses nothing.
fn repair_torn_tail(path: &Path) -> Result<(), AlmanacError> {
    use std::io::{Read, Seek, SeekFrom};
    let fail = |e: std::io::Error| AlmanacError::Config {
        message: format!(
            "failed to repair the torn end of journal {}: {e}",
            path.display()
        ),
        remedy: format!(
            "check free disk space and permissions on {}",
            path.display()
        ),
    };
    let mut file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(fail(e)),
    };
    let len = file.metadata().map_err(fail)?.len();
    let mut last = [0u8; 1];
    if len == 0 || {
        file.seek(SeekFrom::Start(len - 1)).map_err(fail)?;
        file.read_exact(&mut last).map_err(fail)?;
        last[0] == b'\n'
    } {
        return Ok(());
    }
    let keep = last_newline_end(&mut file, len).map_err(fail)?;
    tracing::warn!(
        journal = %path.display(),
        cut = len - keep,
        "cutting a torn final journal line before appending — it was never acknowledged"
    );
    file.set_len(keep).map_err(fail)?;
    file.sync_all().map_err(fail)
}

/// The length up to and including the last newline (0 when there is none),
/// read backwards in chunks so a large journal is not loaded whole.
fn last_newline_end(file: &mut std::fs::File, len: u64) -> std::io::Result<u64> {
    use std::io::{Read, Seek, SeekFrom};
    const CHUNK: u64 = 64 * 1024;
    let mut end = len;
    let mut buf = vec![0u8; CHUNK as usize];
    while end > 0 {
        let start = end.saturating_sub(CHUNK);
        let part = &mut buf[..(end - start) as usize];
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(part)?;
        if let Some(i) = part.iter().rposition(|b| *b == b'\n') {
            return Ok(start + i as u64 + 1);
        }
        end = start;
    }
    Ok(0)
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
