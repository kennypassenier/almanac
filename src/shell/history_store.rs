//! The disk side of the per-day delivery history (`core::history`).
//!
//! One small JSON file next to the journal, replaced atomically (temp file,
//! fsync, rename, fsync of the directory): a power cut leaves either the
//! old file or the new one, never a torn mix, so earlier days are never
//! lost. The file is bounded by `core::history` (400 days, capped rows).

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::core::error::AlmanacError;
use crate::core::history::{self, Book, DeliveryRecord, DeliveryResult};
use crate::shell::delivery::Delivered;
use crate::shell::ingest::AppState;

const FILE_NAME: &str = "history.json";

/// Serializes the read-modify-write of the file across tasks.
static WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Where the history lives: beside the journal, inside the data directory.
pub fn path_for(state: &AppState) -> PathBuf {
    state
        .journal
        .path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
        .join(FILE_NAME)
}

fn failed(what: &str, path: &Path, e: std::io::Error) -> AlmanacError {
    AlmanacError::Config {
        message: format!(
            "failed to {what} the delivery history {}: {e}",
            path.display()
        ),
        remedy: format!(
            "check that {} is writable by the almanac user",
            path.parent().unwrap_or(Path::new(".")).display()
        ),
    }
}

/// Reads the history; a missing file is an empty history.
pub fn load(path: &Path) -> Result<Book, AlmanacError> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| AlmanacError::Config {
            message: format!("the delivery history {} is unreadable: {e}", path.display()),
            remedy: "the next delivery moves it aside and starts a new one".to_string(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Book::default()),
        Err(e) => Err(failed("read", path, e)),
    }
}

/// Temp file, fsync, rename, fsync the directory.
fn write_atomically(path: &Path, book: &Book) -> Result<(), AlmanacError> {
    let bytes = serde_json::to_vec(book).map_err(|e| AlmanacError::Config {
        message: format!("failed to serialize the delivery history: {e}"),
        remedy: "this is a bug in almanac".to_string(),
    })?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| failed("create the directory of", path, e))?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::File::create(&tmp).map_err(|e| failed("create", path, e))?;
    file.write_all(&bytes)
        .map_err(|e| failed("write", path, e))?;
    file.sync_all().map_err(|e| failed("flush", path, e))?;
    drop(file);
    std::fs::rename(&tmp, path).map_err(|e| failed("replace", path, e))?;
    if let Some(parent) = path.parent()
        && let Ok(dir) = std::fs::File::open(if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        })
    {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Adds one record and prunes, under the write lock.
fn append(path: &Path, rec: DeliveryRecord) -> Result<(), AlmanacError> {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut book = match load(path) {
        Ok(book) => book,
        Err(e) => {
            // Keep the unreadable file for a human, then start again.
            tracing::error!(error = %e, "the delivery history was unreadable; starting a new one");
            let _ = std::fs::rename(path, path.with_extension("json.bad"));
            Book::default()
        }
    };
    if history::record(&mut book, rec) {
        write_atomically(path, &book)?;
    }
    Ok(())
}

/// A plain sentence for a failure, never the raw error.
fn plain_reason(e: &AlmanacError) -> &'static str {
    match e {
        AlmanacError::GoogleApi { .. } | AlmanacError::Auth { .. } => {
            "Google Calendar would not take this event."
        }
        _ => "The event could not be built from what the source sent.",
    }
}

/// The hook `worker::record_route` calls after every delivery attempt.
/// Only final outcomes are kept: a delivery, or a failure that will not
/// fix itself. A transient failure stays pending and is retried, so it is
/// not an outcome yet. Never fails the delivery: a history problem is logged.
pub async fn note(
    state: &AppState,
    entry: &crate::core::journal::Entry,
    result: &Result<Delivered, AlmanacError>,
) {
    let (outcome, reason, upsert_key) = match result {
        Ok(d) if d.created => (DeliveryResult::Created, None, d.upsert_key.clone()),
        Ok(d) => (DeliveryResult::Updated, None, d.upsert_key.clone()),
        Err(e) if e.is_transient() => return,
        Err(e) => (
            DeliveryResult::Failed,
            Some(plain_reason(e).to_string()),
            None,
        ),
    };
    let profile = state.profiles().get(&entry.source_id).cloned();
    let title = profile
        .as_ref()
        .and_then(|p| crate::core::mapping::map_payload(&entry.payload, p, "history").ok())
        .map(|event| event.summary)
        .unwrap_or_else(|| "Untitled event".to_string());
    let rec = DeliveryRecord {
        at: (state.now)(),
        entry_id: entry.id.clone(),
        source: entry.source_id.clone(),
        calendar_id: profile.map(|p| p.target_calendar_id).unwrap_or_default(),
        title,
        result: outcome,
        reason,
        upsert_key,
    };
    let path = path_for(state);
    // Hold off while a backup copies the state root, like the journal does.
    let _ticket = chassis::shell::backup::writing().await;
    match tokio::task::spawn_blocking(move || append(&path, rec)).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            tracing::warn!(error = %e, remedy = %e.remedy(), "could not record the delivery history")
        }
        Err(e) => tracing::warn!(error = %e, "the delivery history task failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_an_empty_history_and_a_write_round_trips() {
        let dir = std::env::temp_dir().join(format!("almanac-history-{}", uuid::Uuid::new_v4()));
        let path = dir.join(FILE_NAME);
        assert!(load(&path).unwrap().days.is_empty());
        let rec = DeliveryRecord {
            at: "2026-10-19T08:16:00+00:00".to_string(),
            entry_id: "e".to_string(),
            source: "s".to_string(),
            calendar_id: "c".to_string(),
            title: "t".to_string(),
            result: DeliveryResult::Created,
            reason: None,
            upsert_key: None,
        };
        append(&path, rec).unwrap();
        assert_eq!(load(&path).unwrap().days.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
