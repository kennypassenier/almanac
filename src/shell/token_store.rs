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
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::core::error::AlmanacError;
use crate::core::secrets::{KEY_BYTES, open, parse_key, seal};
use crate::shell::durability::fsync_parent_dir;

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

    async fn persist_with(
        &self,
        tokens: &BTreeMap<String, TokenRecord>,
        sessions: &BTreeMap<String, SessionRecord>,
    ) -> Result<(), AlmanacError> {
        let file = StoreFile {
            tokens: tokens.clone(),
            sessions: sessions.clone(),
        };
        let body = serde_json::to_string_pretty(&file).map_err(|e| AlmanacError::Config {
            message: format!("failed to serialize the token store: {e}"),
            remedy: "this is a bug in almanac".to_string(),
        })?;

        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| AlmanacError::Config {
                message: format!("failed to create {}: {e}", parent.display()),
                remedy: format!("check permissions on {}", parent.display()),
            })?;
        }

        let temp = self.path.with_extension("writing");
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

        std::fs::rename(&temp, &self.path).map_err(|e| AlmanacError::Config {
            message: format!("failed to replace {}: {e}", self.path.display()),
            remedy: "check permissions on the store's directory".to_string(),
        })?;

        // Without this the rename itself can be lost in a real power
        // cut: the file contents are durable but the directory entry
        // pointing at them is not (critic's finding, 2026-08-28).
        fsync_parent_dir(&self.path);
        Ok(())
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

    /// Records a live dashboard session, dropping any already expired
    /// so the store does not accumulate them.
    pub async fn start_session(
        &self,
        id: &str,
        expires_at_unix: u64,
        now_unix: u64,
    ) -> Result<(), AlmanacError> {
        // tokens first, then sessions — the same order as `issue`
        // and `revoke`. Taking them the other way round here is what
        // made a login racing a token issue able to deadlock the whole
        // service, ingest included.
        let tokens = self.tokens.read().await;
        let mut sessions = self.sessions.write().await;
        sessions.retain(|_, s| s.expires_at_unix > now_unix);
        sessions.insert(id.to_string(), SessionRecord { expires_at_unix });
        self.persist_with(&tokens, &sessions).await
    }

    /// Whether this cookie names a live session. Compared in constant
    /// time: a session id is as good as a password.
    pub async fn session_is_live(&self, presented: &str, now_unix: u64) -> bool {
        self.sessions.read().await.iter().any(|(id, s)| {
            s.expires_at_unix > now_unix && crate::core::token::constant_time_eq(id, presented)
        })
    }

    /// Ends a session server-side, so a copied cookie stops working —
    /// the property a self-validating cookie could not offer.
    pub async fn end_session(&self, id: &str) -> Result<(), AlmanacError> {
        let tokens = self.tokens.read().await;
        let mut sessions = self.sessions.write().await;
        if sessions.remove(id).is_none() {
            return Ok(());
        }
        self.persist_with(&tokens, &sessions).await
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

type StoreContents = (
    BTreeMap<String, TokenRecord>,
    BTreeMap<String, SessionRecord>,
);

/// Reads and parses the store file, treating a missing file as empty.
fn read_store_file(path: &Path) -> Result<StoreContents, AlmanacError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> (TokenStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "almanac-tokenstore-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tokens.json");
        (TokenStore::with_key(path, [3u8; KEY_BYTES]), dir)
    }

    #[tokio::test]
    async fn an_issued_token_verifies_and_a_wrong_one_does_not() {
        let (store, dir) = temp_store("verify");
        store
            .issue("home-assistant", "tok-abc", "now")
            .await
            .unwrap();

        assert!(store.verify("home-assistant", "tok-abc").await);
        assert!(!store.verify("home-assistant", "tok-xyz").await);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn one_sources_token_does_not_verify_for_another() {
        let (store, dir) = temp_store("cross");
        store
            .issue("home-assistant", "ha-tok", "now")
            .await
            .unwrap();
        store.issue("uptime-kuma", "kuma-tok", "now").await.unwrap();

        assert!(!store.verify("uptime-kuma", "ha-tok").await);
        assert!(store.verify("uptime-kuma", "kuma-tok").await);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_revoked_token_stops_working_immediately() {
        // M12's sharpest exit criterion: revocation must take effect on
        // the next request, not on the next restart.
        let (store, dir) = temp_store("revoke");
        store
            .issue("home-assistant", "tok-abc", "now")
            .await
            .unwrap();
        assert!(store.verify("home-assistant", "tok-abc").await);

        assert!(store.revoke("home-assistant").await.unwrap());
        assert!(!store.verify("home-assistant", "tok-abc").await);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn revoking_something_that_was_never_issued_says_so() {
        let (store, dir) = temp_store("revoke-missing");
        assert!(!store.revoke("never-existed").await.unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn reissuing_replaces_the_previous_token() {
        let (store, dir) = temp_store("reissue");
        store.issue("home-assistant", "old", "t1").await.unwrap();
        store.issue("home-assistant", "new", "t2").await.unwrap();

        assert!(
            !store.verify("home-assistant", "old").await,
            "the old one must stop working"
        );
        assert!(store.verify("home-assistant", "new").await);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn the_file_on_disk_never_contains_the_plaintext_token() {
        // Standing rule 10, asserted rather than assumed.
        let (store, dir) = temp_store("plaintext");
        store
            .issue("home-assistant", "PLAINTEXT-MARKER-9f3", "now")
            .await
            .unwrap();

        let contents = std::fs::read_to_string(store.path()).unwrap();
        assert!(
            !contents.contains("PLAINTEXT-MARKER-9f3"),
            "the token leaked into the store file:\n{contents}"
        );
        assert!(
            contents.contains("home-assistant"),
            "but the source id is fine to store"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn reveal_returns_a_working_token_and_none_for_an_unknown_source() {
        let (store, dir) = temp_store("reveal");
        store
            .issue("home-assistant", "tok-abc", "now")
            .await
            .unwrap();

        assert_eq!(
            store.reveal("home-assistant").await.unwrap().as_deref(),
            Some("tok-abc")
        );
        assert_eq!(store.reveal("nobody").await.unwrap(), None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn listing_reports_sources_without_their_tokens() {
        let (store, dir) = temp_store("list");
        store
            .issue("home-assistant", "tok-abc", "2026-08-28")
            .await
            .unwrap();

        let listed = store.list().await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, "home-assistant");
        assert_eq!(listed[0].1, "2026-08-28");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn an_empty_store_passes_the_startup_key_check() {
        let (store, dir) = temp_store("keycheck-empty");
        assert!(store.verify_key_opens_store().await.is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn logging_in_while_a_token_is_issued_does_not_deadlock() {
        // `issue` takes tokens-then-sessions; `start_session` used to
        // take sessions-then-tokens. Under real concurrency that is a
        // deadlock, and because `verify` also wants `tokens` and
        // tokio's RwLock is write-preferring, it takes ingest down with
        // it — while /healthz keeps answering 200. Every other test in
        // this file is sequential, which is exactly why none of them
        // saw it.
        let (store, dir) = temp_store("deadlock");
        let store = std::sync::Arc::new(store);

        let mut work = Vec::new();
        for i in 0..20 {
            let issuing = std::sync::Arc::clone(&store);
            work.push(tokio::spawn(async move {
                issuing
                    .issue(&format!("source-{i}"), &format!("token-{i}"), "2026-08-28")
                    .await
                    .unwrap();
            }));
            let logging_in = std::sync::Arc::clone(&store);
            work.push(tokio::spawn(async move {
                logging_in
                    .start_session(&format!("session-{i}"), 2_000_000_000, 1_787_000_000)
                    .await
                    .unwrap();
            }));
            let verifying = std::sync::Arc::clone(&store);
            work.push(tokio::spawn(async move {
                verifying.verify(&format!("source-{i}"), "whatever").await;
            }));
        }

        let all = futures_join(work);
        let finished = tokio::time::timeout(std::time::Duration::from_secs(20), all).await;
        assert!(
            finished.is_ok(),
            "logins, token issues and verifies must not deadlock each other"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Awaits every handle in order. A plain helper rather than a new
    /// dependency on `futures` for one test.
    async fn futures_join(handles: Vec<tokio::task::JoinHandle<()>>) {
        for handle in handles {
            handle.await.unwrap();
        }
    }

    #[tokio::test]
    async fn a_session_that_has_passed_its_expiry_is_no_longer_live() {
        // The 7-day TTL is enforced in code and was asserted nowhere,
        // even though the clock is injectable and this is two lines.
        let (store, dir) = temp_store("session-ttl");
        store.start_session("cookie", 1_000, 500).await.unwrap();

        assert!(store.session_is_live("cookie", 999).await, "still valid");
        assert!(
            !store.session_is_live("cookie", 1_000).await,
            "a session must not survive its own expiry timestamp"
        );
        assert!(
            !store.session_is_live("cookie", 5_000).await,
            "nor anything after it"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_forged_session_cookie_is_not_live() {
        let (store, dir) = temp_store("session-forged");
        store
            .start_session("real-cookie", 2_000_000_000, 1_000)
            .await
            .unwrap();

        assert!(!store.session_is_live("", 1_000).await);
        assert!(!store.session_is_live("real-cookie-x", 1_000).await);
        assert!(!store.session_is_live("real-cooki", 1_000).await);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn starting_a_session_does_not_lose_a_token_issued_at_the_same_time() {
        // Both maps live in one file. `start_session` used to clone the
        // token map before mutating sessions, so a token issued in
        // between was written and then overwritten away again.
        let (store, dir) = temp_store("no-lost-update");
        let store = std::sync::Arc::new(store);

        store
            .issue("first", "token-first", "2026-08-28")
            .await
            .unwrap();
        store
            .start_session("cookie", 2_000_000_000, 1_000)
            .await
            .unwrap();

        let reopened =
            TokenStore::with_key_loading(store.path().to_path_buf(), [3u8; KEY_BYTES]).unwrap();
        assert!(
            reopened.verify("first", "token-first").await,
            "the token must survive a session being written after it"
        );
        assert!(reopened.session_is_live("cookie", 1_000).await);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn the_startup_key_check_passes_with_the_right_key() {
        let (store, dir) = temp_store("keycheck-ok");
        store.issue("home-assistant", "tok", "now").await.unwrap();
        assert!(store.verify_key_opens_store().await.is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn the_startup_key_check_catches_a_wrong_key_instead_of_401ing_later() {
        // The failure this exists to prevent: without the check the
        // process starts happily and every source gets a 401 against a
        // store that looks perfectly intact.
        let (store, dir) = temp_store("keycheck-wrong");
        store.issue("home-assistant", "tok", "now").await.unwrap();

        let wrong = TokenStore::with_key_loading(store.path().to_path_buf(), [9u8; KEY_BYTES])
            .expect("the file itself parses fine — that is the whole problem");
        let err = wrong.verify_key_opens_store().await.unwrap_err();

        assert!(err.to_string().contains("does not open"));
        assert!(
            err.remedy().contains("escrow"),
            "the remedy must point at the way out"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_store_written_with_one_key_does_not_verify_under_another() {
        // Why the encryption key must stay stable, demonstrated: a
        // changed key does not silently accept or reject at random, it
        // consistently fails to match.
        let (store, dir) = temp_store("keychange");
        store
            .issue("home-assistant", "tok-abc", "now")
            .await
            .unwrap();

        let reopened = TokenStore::with_key(store.path().to_path_buf(), [4u8; KEY_BYTES]);
        {
            let mut tokens = reopened.tokens.write().await;
            *tokens = store.tokens.read().await.clone();
        }
        assert!(!reopened.verify("home-assistant", "tok-abc").await);

        std::fs::remove_dir_all(&dir).ok();
    }
}
