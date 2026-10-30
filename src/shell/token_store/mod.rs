//! The encrypted app-token store (M12/AR17). One record per source:
//! which source it is, its token sealed with the encryption key, and
//! when it was issued.
//!
//! Written atomically (temp + rename, standing rule 12) so a crash
//! mid-write leaves the previous store intact rather than a truncated
//! one — losing every source's token to a half-written file would take
//! the whole hub down until each was re-issued.
//!
//! Held in memory while running so authenticating a request never
//! touches the disk; the file is the durable copy.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::core::error::AlmanacError;
use crate::core::secrets::{KEY_BYTES, open, parse_key, seal};

/// Environment variable holding the hex encryption key. Mandatory
/// whenever the bootstrap token is set (AR17): deriving this key from
/// the bootstrap token instead would work right up until that token is
/// rotated, at which point every stored app token silently becomes
/// undecryptable.
pub const SECRET_KEY_ENV: &str = "ALMANAC_SECRET_KEY";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenRecord {
    pub source_id: String,
    /// Hex of nonce + ciphertext; never the token itself.
    pub sealed_token: String,
    pub issued_at: String,
}

/// A dashboard session. Kept in the same encrypted store as the
/// tokens (AR25) so logging in survives a restart and a self-update —
/// without weakening logout, which stays a real server-side removal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionRecord {
    pub expires_at_unix: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    tokens: BTreeMap<String, TokenRecord>,
    #[serde(default)]
    sessions: BTreeMap<String, SessionRecord>,
}

/// # Lock order
///
/// `tokens` is always acquired **before** `sessions`. Both maps live in
/// one encrypted file, so every mutation has to hold both, and two
/// paths taking them in opposite orders deadlock: a login holding
/// `sessions` while waiting for `tokens`, against a token issue holding
/// `tokens` while waiting for `sessions`. Because `verify` also needs
/// `tokens` and tokio's `RwLock` is write-preferring, that deadlock
/// takes ingest down with it while `/healthz` keeps answering 200.
///
/// Any new method touching both must follow this order.
pub struct TokenStore {
    path: PathBuf,
    key: [u8; KEY_BYTES],
    tokens: RwLock<BTreeMap<String, TokenRecord>>,
    sessions: RwLock<BTreeMap<String, SessionRecord>>,
}

impl TokenStore {
    /// Reads the encryption key from the environment and loads the
    /// store. Refuses rather than starting without a key: an
    /// unencrypted fallback would put plaintext tokens on disk exactly
    /// when the operator believed the opposite.
    pub fn load(path: PathBuf) -> Result<Self, AlmanacError> {
        let hex_key = std::env::var(SECRET_KEY_ENV).map_err(|_| AlmanacError::Config {
            message: format!("{SECRET_KEY_ENV} is not set"),
            remedy: format!(
                "generate one with `openssl rand -hex {KEY_BYTES}` and supply it via `latch run \
                 --`; it is required whenever a bootstrap token is configured, and must stay \
                 stable or previously issued tokens become unreadable"
            ),
        })?;
        let key = parse_key(&hex_key)?;
        Self::with_key_loading(path, key)
    }

    /// Reads whatever is already on disk into a new store. Used by
    /// [`Self::load`] and by anything that needs a store reflecting
    /// the current file — including a process starting up after an
    /// update, which must see the sessions and tokens its predecessor
    /// wrote (AR25).
    pub fn with_key_loading(path: PathBuf, key: [u8; KEY_BYTES]) -> Result<Self, AlmanacError> {
        let (tokens, sessions) = read_store_file(&path)?;
        Ok(Self {
            path,
            key,
            tokens: RwLock::new(tokens),
            sessions: RwLock::new(sessions),
        })
    }

    /// Builds a store around an explicit key, for tests and for the
    /// dry-run paths that must not depend on the environment.
    pub fn with_key(path: PathBuf, key: [u8; KEY_BYTES]) -> Self {
        Self {
            path,
            key,
            tokens: RwLock::new(BTreeMap::new()),
            sessions: RwLock::new(BTreeMap::new()),
        }
    }

    /// Stores a source's token, replacing any it already had. Returns
    /// once the new store is durably on disk.
    pub async fn issue(&self, source_id: &str, token: &str, now: &str) -> Result<(), AlmanacError> {
        let record = TokenRecord {
            source_id: source_id.to_string(),
            sealed_token: seal(&self.key, token)?,
            issued_at: now.to_string(),
        };

        // LOCK ORDER: tokens before sessions, always. Both maps are
        // written to one file, so every mutation needs both — and two
        // call paths taking them in opposite orders is a deadlock that
        // only shows up under real concurrency. See the module note.
        let mut tokens = self.tokens.write().await;
        tokens.insert(source_id.to_string(), record);
        let sessions = self.sessions.read().await;
        self.persist_with(&tokens, &sessions).await
    }

    /// Removes a source's token. Returns whether there was one — the
    /// dashboard says "revoked" or "there was nothing to revoke"
    /// rather than implying it did something it did not.
    pub async fn revoke(&self, source_id: &str) -> Result<bool, AlmanacError> {
        let mut tokens = self.tokens.write().await;
        let existed = tokens.remove(source_id).is_some();
        if existed {
            let sessions = self.sessions.read().await;
            self.persist_with(&tokens, &sessions).await?;
        }
        Ok(existed)
    }

    /// Whether `presented` is the token issued to `source_id`.
    ///
    /// Compares the decrypted token rather than re-encrypting the
    /// candidate: a fresh nonce per record makes two sealings of the
    /// same value differ, so ciphertext comparison would never match.
    pub async fn verify(&self, source_id: &str, presented: &str) -> bool {
        let tokens = self.tokens.read().await;
        let Some(record) = tokens.get(source_id) else {
            return false;
        };
        match open(&self.key, &record.sealed_token) {
            Ok(actual) => crate::core::token::constant_time_eq(&actual, presented),
            Err(e) => {
                tracing::error!(
                    source_id = %source_id, error = %e, remedy = %e.remedy(),
                    "a stored token could not be decrypted; treating it as no match"
                );
                false
            }
        }
    }

    /// The plaintext token for a source, so the dashboard can render a
    /// working command. The one place that decrypts for display.
    pub async fn reveal(&self, source_id: &str) -> Result<Option<String>, AlmanacError> {
        let tokens = self.tokens.read().await;
        match tokens.get(source_id) {
            Some(record) => open(&self.key, &record.sealed_token).map(Some),
            None => Ok(None),
        }
    }

    /// Proves at startup that the configured key actually opens this
    /// store, by decrypting one record.
    ///
    /// Without this the failure is silent in exactly the way that
    /// costs an hour: the file parses fine under a wrong key, startup
    /// succeeds, and every source then gets a 401 while the store
    /// looks intact. Latch itself fails loudly on a missing or wrong
    /// key — this closes the equivalent gap on Almanac's side (Kenny,
    /// after consulting the Latch project, 2026-08-28).
    pub async fn verify_key_opens_store(&self) -> Result<(), AlmanacError> {
        let tokens = self.tokens.read().await;
        let Some((source_id, record)) = tokens.iter().next() else {
            // An empty store proves nothing, and there is nothing to
            // get wrong yet — the first issue() will write under the
            // current key.
            return Ok(());
        };

        open(&self.key, &record.sealed_token).map(|_| ()).map_err(|_| AlmanacError::Config {
            message: format!(
                "the encryption key does not open the existing token store ({} holds {} record(s), \
                 starting with \"{source_id}\")",
                self.path.display(),
                tokens.len()
            ),
            remedy: format!(
                "{SECRET_KEY_ENV} is not the key this store was written with. Restore the original \
                 key (latch has an escrow mechanism for exactly this), or delete {} and re-issue \
                 every source's token from the dashboard — starting with the wrong key would leave \
                 every source failing authentication against an intact-looking store",
                self.path.display()
            ),
        })
    }

    /// Which sources have a token, and when it was issued. Never
    /// returns the tokens themselves.
    pub async fn list(&self) -> Vec<(String, String)> {
        self.tokens
            .read()
            .await
            .values()
            .map(|r| (r.source_id.clone(), r.issued_at.clone()))
            .collect()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

mod persist;

use persist::read_store_file;
mod sessions;

#[cfg(test)]
mod tests;
