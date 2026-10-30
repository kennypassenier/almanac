//! The store file on disk: read at load, rewritten atomically (temp +
//! rename + directory fsync) on every change.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use crate::core::error::AlmanacError;
use crate::shell::durability::fsync_parent_dir;

use super::{SessionRecord, StoreFile, TokenRecord, TokenStore};

impl TokenStore {
    pub(super) async fn persist_with(
        &self,
        tokens: &BTreeMap<String, TokenRecord>,
        sessions: &BTreeMap<String, SessionRecord>,
    ) -> Result<(), AlmanacError> {
        // feat-backup-1 (chassis-rs 3.1.0): waits out a backup pause before
        // the temp-file-plus-rename below, so a nightly `tar` of the state
        // root never races this write.
        let _ticket = chassis::shell::backup::writing().await;

        let file = StoreFile {
            tokens: tokens.clone(),
            sessions: sessions.clone(),
        };
        let body = serde_json::to_string_pretty(&file).map_err(|e| AlmanacError::Config {
            message: format!("failed to serialize the token store: {e}"),
            remedy: "this is a bug in almanac".to_string(),
        })?;

        write_atomically(&self.path, &body)?;

        // Without this the rename itself can be lost in a real power
        // cut: the file contents are durable but the directory entry
        // pointing at them is not (critic's finding, 2026-08-28).
        fsync_parent_dir(&self.path);
        Ok(())
    }
}

/// Writes `body` to `path` through a temp file, so a crash mid-write
/// leaves the previous file whole.
fn write_atomically(path: &Path, body: &str) -> Result<(), AlmanacError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| AlmanacError::Config {
            message: format!("failed to create {}: {e}", parent.display()),
            remedy: format!("check permissions on {}", parent.display()),
        })?;
    }

    let temp = path.with_extension("writing");
    {
        let mut handle = File::create(&temp).map_err(|e| AlmanacError::Config {
            message: format!("failed to create {}: {e}", temp.display()),
            remedy: "check free disk space and permissions".to_string(),
        })?;
        handle
            .write_all(body.as_bytes())
            .map_err(|e| AlmanacError::Config {
                message: format!("failed to write {}: {e}", temp.display()),
                remedy: "check free disk space".to_string(),
            })?;
        handle.sync_all().map_err(|e| AlmanacError::Config {
            message: format!("failed to fsync {}: {e}", temp.display()),
            remedy: "check disk health".to_string(),
        })?;
    }

    std::fs::rename(&temp, path).map_err(|e| AlmanacError::Config {
        message: format!("failed to replace {}: {e}", path.display()),
        remedy: "check permissions on the store's directory".to_string(),
    })?;

    Ok(())
}

pub(super) type StoreContents = (
    BTreeMap<String, TokenRecord>,
    BTreeMap<String, SessionRecord>,
);

/// Reads and parses the store file, treating a missing file as empty.
pub(super) fn read_store_file(path: &Path) -> Result<StoreContents, AlmanacError> {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let parsed: StoreFile =
                serde_json::from_str(&contents).map_err(|e| AlmanacError::Config {
                    message: format!("failed to parse the token store {}: {e}", path.display()),
                    remedy: "the store is damaged; restore it from backup, or delete it and \
                             re-issue every source's token"
                        .to_string(),
                })?;
            Ok((parsed.tokens, parsed.sessions))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok((BTreeMap::new(), BTreeMap::new()))
        }
        Err(e) => Err(AlmanacError::Config {
            message: format!("failed to read the token store {}: {e}", path.display()),
            remedy: format!("check permissions on {}", path.display()),
        }),
    }
}
