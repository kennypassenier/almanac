//! The durable ingest journal's disk side (AR16). Every accepted
//! payload is appended and fsynced *before* the source is told the
//! request succeeded, so a crash or power cut between "accepted" and
//! "delivered to Google" loses nothing: replay-on-start re-delivers,
//! and upsert (K2/AR15) plus idempotency keys (M7) make that
//! redelivery converge instead of duplicating.
//!
//! Append-only JSON lines, never mutated in place — a torn write can
//! only ever damage the final line, which replay skips (see
//! `read_records`). Compaction rewrites the file atomically (temp +
//! rename, standing rule 12), never truncates in place.

use std::io::Write;
use std::path::{Path, PathBuf};

use tokio::sync::Mutex;

use crate::core::error::AlmanacError;
use crate::core::journal::{Entry, Record};

/// Refuse to append beyond this and say so loudly rather than filling
/// the disk silently (standing rule 12: no silent caps). Compaction
/// keeps a healthy journal far below it; hitting this means delivery
/// has been failing for a long time and needs a human.
pub const DEFAULT_MAX_BYTES: u64 = 64 * 1024 * 1024;

pub struct Journal {
    path: PathBuf,
    max_bytes: u64,
    /// Serializes writers so two appends can never interleave a
    /// partial line into the file.
    write_lock: Mutex<()>,
}

impl Journal {
    pub fn new(path: PathBuf, max_bytes: u64) -> Self {
        Self {
            path,
            max_bytes,
            write_lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    /// Appends one record and fsyncs it. Returns only once the bytes
    /// are durably on disk — the caller may not acknowledge the source
    /// before this resolves.
    async fn append(&self, record: &Record) -> Result<(), AlmanacError> {
        let mut line = serde_json::to_string(record).map_err(|e| AlmanacError::Config {
            message: format!("failed to serialize a journal record: {e}"),
            remedy: "this is a bug in almanac; the payload could not be re-encoded".to_string(),
        })?;
        line.push('\n');

        let _guard = self.write_lock.lock().await;

        if let Some(parent) = self.path.parent()
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

        let existing = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        if existing + line.len() as u64 > self.max_bytes {
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

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| AlmanacError::Config {
                message: format!("failed to open journal {}: {e}", self.path.display()),
                remedy: format!("check permissions on {}", self.path.display()),
            })?;

        file.write_all(line.as_bytes())
            .map_err(|e| AlmanacError::Config {
                message: format!("failed to write to journal {}: {e}", self.path.display()),
                remedy: "check free disk space and permissions".to_string(),
            })?;

        // The whole point: without this the OS may still be holding
        // the bytes in cache when the power goes, and the source has
        // already been told the request succeeded.
        file.sync_all().map_err(|e| AlmanacError::Config {
            message: format!("failed to fsync journal {}: {e}", self.path.display()),
            remedy: "the filesystem rejected a durability barrier; check disk health".to_string(),
        })?;

        Ok(())
    }

    /// Durably records an accepted payload. Must complete before the
    /// source is acknowledged.
    pub async fn accept(&self, entry: &Entry) -> Result<(), AlmanacError> {
        self.append(&Record::Entry(entry.clone())).await
    }

    /// Durably records that an entry has been delivered.
    pub async fn mark_done(&self, id: &str) -> Result<(), AlmanacError> {
        self.append(&Record::Done { id: id.to_string() }).await
    }

    /// Sets an entry aside as permanently undeliverable (T1).
    ///
    /// A marker, like `mark_done` — the payload itself stays in the
    /// log, because the source was told 202 and because the reason is
    /// what you need in order to fix the profile.
    pub async fn mark_dead(&self, id: &str, reason: &str, at: &str) -> Result<(), AlmanacError> {
        self.append(&Record::Dead {
            id: id.to_string(),
            reason: reason.to_string(),
            at: at.to_string(),
        })
        .await
    }

    /// Entries that were set aside, with the reason each was.
    pub fn dead(&self) -> Result<Vec<(crate::core::journal::Entry, String, String)>, AlmanacError> {
        let records = self.read_records()?;
        Ok(crate::core::journal::dead(&records)
            .into_iter()
            .map(|(entry, reason, at)| (entry.clone(), reason.to_string(), at.to_string()))
            .collect())
    }

    /// Reads every intact record from the log.
    ///
    /// A trailing partial line — the signature of a crash mid-append —
    /// is skipped rather than treated as corruption: the record it
    /// would have become was never acknowledged to its source, so
    /// dropping it loses nothing that was ever promised. Any *earlier*
    /// unparseable line is a real integrity problem and is surfaced.
    fn read_records(&self) -> Result<Vec<Record>, AlmanacError> {
        let contents = match std::fs::read_to_string(&self.path) {
            Ok(contents) => contents,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => {
                return Err(AlmanacError::Config {
                    message: format!("failed to read journal {}: {e}", self.path.display()),
                    remedy: format!("check permissions on {}", self.path.display()),
                });
            }
        };

        let ends_cleanly = contents.is_empty() || contents.ends_with('\n');
        let lines: Vec<&str> = contents.lines().collect();

        let mut records = Vec::with_capacity(lines.len());
        for (i, line) in lines.iter().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Record>(line) {
                Ok(record) => records.push(record),
                Err(e) => {
                    let is_trailing_partial = !ends_cleanly && i + 1 == lines.len();
                    if is_trailing_partial {
                        tracing::warn!(
                            journal = %self.path.display(),
                            "discarding a torn final journal line — it was never acknowledged to its source"
                        );
                        continue;
                    }
                    return Err(AlmanacError::Config {
                        message: format!(
                            "journal {} line {} is unparseable: {e}",
                            self.path.display(),
                            i + 1
                        ),
                        remedy: "the journal is damaged beyond the usual torn-tail case; move it \
                                 aside to inspect before restarting, since doing so abandons any \
                                 undelivered events it holds"
                            .to_string(),
                    });
                }
            }
        }

        Ok(records)
    }

    /// The entries accepted but not yet delivered, in acceptance
    /// order. Called on startup (replay) and by the worker loop.
    pub fn pending(&self) -> Result<Vec<Entry>, AlmanacError> {
        let records = self.read_records()?;
        Ok(crate::core::journal::pending(&records)
            .into_iter()
            .cloned()
            .collect())
    }

    /// Rewrites the log containing only still-pending entries, so a
    /// long-running process does not grow the file without bound.
    /// Atomic: writes a sibling temp file and renames over the
    /// original, so a crash mid-compaction leaves the old journal
    /// fully intact (standing rule 12).
    pub async fn compact(&self) -> Result<usize, AlmanacError> {
        let _guard = self.write_lock.lock().await;

        let records = self.read_records()?;
        let mut pending: Vec<Record> = crate::core::journal::pending(&records)
            .into_iter()
            .cloned()
            .map(Record::Entry)
            .collect();

        // Dead entries survive compaction with their markers (T1).
        // Dropping them would silently discard a payload the source
        // was told had been accepted, and lose the reason it failed —
        // which is the only thing that makes it fixable.
        for (entry, reason, at) in crate::core::journal::dead(&records) {
            pending.push(Record::Entry(entry.clone()));
            pending.push(Record::Dead {
                id: entry.id.clone(),
                reason: reason.to_string(),
                at: at.to_string(),
            });
        }

        let mut body = String::new();
        for record in &pending {
            let line = serde_json::to_string(record).map_err(|e| AlmanacError::Config {
                message: format!("failed to serialize a journal record during compaction: {e}"),
                remedy: "this is a bug in almanac".to_string(),
            })?;
            body.push_str(&line);
            body.push('\n');
        }

        let temp = self.path.with_extension("compacting");
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

        std::fs::rename(&temp, &self.path).map_err(|e| AlmanacError::Config {
            message: format!(
                "failed to replace {} with {}: {e}",
                self.path.display(),
                temp.display()
            ),
            remedy: "check permissions on the journal directory".to_string(),
        })?;
        // The rename lives in the directory, not in the file: without
        // this a power cut can bring back a directory entry pointing
        // at the pre-compaction file, replaying delivered entries.
        crate::shell::durability::fsync_parent_dir(&self.path);

        Ok(pending.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_journal(name: &str) -> (Journal, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "almanac-journal-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("journal.jsonl");
        (Journal::new(path.clone(), DEFAULT_MAX_BYTES), dir)
    }

    fn entry(id: &str) -> Entry {
        Entry {
            id: id.to_string(),
            source_id: "home-assistant".to_string(),
            received_at: "2026-08-28T09:00:00+00:00".to_string(),
            payload: json!({"title": "t"}),
            idempotency_key: None,
        }
    }

    #[tokio::test]
    async fn an_accepted_entry_is_pending_until_marked_done() {
        let (journal, dir) = temp_journal("accept");

        journal.accept(&entry("a")).await.unwrap();
        assert_eq!(journal.pending().unwrap().len(), 1);

        journal.mark_done("a").await.unwrap();
        assert!(journal.pending().unwrap().is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_journal_that_does_not_exist_yet_has_nothing_pending() {
        let (journal, dir) = temp_journal("missing");
        assert!(journal.pending().unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn replay_survives_a_torn_final_line() {
        // Exactly what a power cut mid-append leaves behind.
        let (journal, dir) = temp_journal("torn");
        journal.accept(&entry("a")).await.unwrap();

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(journal.path())
            .unwrap();
        file.write_all(b"{\"kind\":\"entry\",\"id\":\"b\",\"sou")
            .unwrap();
        drop(file);

        let pending = journal.pending().unwrap();
        assert_eq!(pending.len(), 1, "the intact entry must survive");
        assert_eq!(pending[0].id, "a");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_damaged_line_that_is_not_the_tail_is_reported_not_silently_skipped() {
        let (journal, dir) = temp_journal("damaged");
        journal.accept(&entry("a")).await.unwrap();

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(journal.path())
            .unwrap();
        file.write_all(b"garbage that is not json\n").unwrap();
        file.write_all(b"{\"kind\":\"done\",\"id\":\"a\"}\n")
            .unwrap();
        drop(file);

        let err = journal.pending().unwrap_err();
        assert!(err.to_string().contains("unparseable"));
        assert!(err.remedy().contains("move it aside"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn compaction_keeps_pending_entries_and_drops_delivered_ones() {
        let (journal, dir) = temp_journal("compact");

        journal.accept(&entry("a")).await.unwrap();
        journal.accept(&entry("b")).await.unwrap();
        journal.mark_done("a").await.unwrap();

        let before = std::fs::metadata(journal.path()).unwrap().len();
        let kept = journal.compact().await.unwrap();
        let after = std::fs::metadata(journal.path()).unwrap().len();

        assert_eq!(kept, 1);
        assert!(after < before, "compaction should shrink the file");

        let pending = journal.pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "b");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn the_size_cap_refuses_loudly_instead_of_filling_the_disk() {
        let (_, dir) = temp_journal("cap");
        let journal = Journal::new(dir.join("journal.jsonl"), 200);

        journal.accept(&entry("a")).await.unwrap();
        let err = loop {
            match journal.accept(&entry("filler")).await {
                Ok(()) => continue,
                Err(e) => break e,
            }
        };

        assert!(err.to_string().contains("cap"));
        assert!(err.remedy().contains("failing"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn entries_replay_in_acceptance_order() {
        let (journal, dir) = temp_journal("order");
        for id in ["a", "b", "c"] {
            journal.accept(&entry(id)).await.unwrap();
        }
        let pending = journal.pending().unwrap();
        let ids: Vec<_> = pending.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
