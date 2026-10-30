//! Compaction: the log rewritten with only what is still owed, swapped
//! in atomically.

use std::io::Write;
use std::path::Path;

use crate::core::error::AlmanacError;
use crate::core::journal::Record;

use super::Journal;

impl Journal {
    /// Rewrites the log containing only still-pending entries, so a
    /// long-running process does not grow the file without bound.
    /// Atomic: writes a sibling temp file and renames over the
    /// original, so a crash mid-compaction leaves the old journal
    /// fully intact (standing rule 12).
    pub async fn compact(&self) -> Result<usize, AlmanacError> {
        let _guard = self.write_lock.lock().await;
        // feat-backup-1: see the comment in `append` — compaction rewrites
        // the same file a backup archives.
        let _ticket = chassis::shell::backup::writing().await;

        let kept = still_owed(&self.read_records()?);
        let mut body = String::new();
        for record in &kept {
            let line = serde_json::to_string(record).map_err(|e| AlmanacError::Config {
                message: format!("failed to serialize a journal record during compaction: {e}"),
                remedy: "this is a bug in almanac".to_string(),
            })?;
            body.push_str(&line);
            body.push('\n');
        }
        replace_atomically(&self.path, &body)?;
        Ok(kept.len())
    }
}

/// The pending entries, then every dead entry with its marker.
///
/// Dead entries survive compaction with their markers (T1). Dropping
/// them would silently discard a payload the source was told had been
/// accepted, and lose the reason it failed — which is the only thing
/// that makes it fixable.
fn still_owed(records: &[Record]) -> Vec<Record> {
    let mut kept: Vec<Record> = crate::core::journal::pending(records)
        .into_iter()
        .cloned()
        .map(Record::Entry)
        .collect();
    for (entry, reason, at) in crate::core::journal::dead(records) {
        kept.push(Record::Entry(entry.clone()));
        kept.push(Record::Dead {
            id: entry.id.clone(),
            reason: reason.to_string(),
            at: at.to_string(),
        });
    }
    kept
}

/// Writes `body` to a sibling temp file, fsyncs it, renames it over
/// `path` and fsyncs the directory.
fn replace_atomically(path: &Path, body: &str) -> Result<(), AlmanacError> {
    let temp = path.with_extension("compacting");
    {
        let mut file = std::fs::File::create(&temp).map_err(|e| AlmanacError::Config {
            message: format!("failed to create {}: {e}", temp.display()),
            remedy: "check free disk space and permissions".to_string(),
        })?;
        file.write_all(body.as_bytes())
            .map_err(|e| AlmanacError::Config {
                message: format!("failed to write {}: {e}", temp.display()),
                remedy: "check free disk space".to_string(),
            })?;
        file.sync_all().map_err(|e| AlmanacError::Config {
            message: format!("failed to fsync {}: {e}", temp.display()),
            remedy: "check disk health".to_string(),
        })?;
    }

    std::fs::rename(&temp, path).map_err(|e| AlmanacError::Config {
        message: format!(
            "failed to replace {} with {}: {e}",
            path.display(),
            temp.display()
        ),
        remedy: "check permissions on the journal directory".to_string(),
    })?;
    // The rename lives in the directory, not in the file: without
    // this a power cut can bring back a directory entry pointing
    // at the pre-compaction file, replaying delivered entries.
    crate::shell::durability::fsync_parent_dir(path);
    Ok(())
}
