//! Dashboard sessions, kept in the same sealed store as the tokens.

use crate::core::error::AlmanacError;

use super::{SessionRecord, TokenStore};

impl TokenStore {
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
}
