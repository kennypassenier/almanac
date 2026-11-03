//! Removing set-aside entries after their 30 days (Kenny, form
//! aside-retention), one log line per entry.

use chassis::storage::rusqlite::params;

use crate::core::error::AlmanacError;
use crate::core::journal::SET_ASIDE_MAX_AGE_DAYS;

use super::{Journal, db_error};

impl Journal {
    /// Deletes set-aside entries older than [`SET_ASIDE_MAX_AGE_DAYS`] as of
    /// `now_unix` and logs each. Returns how many went.
    pub async fn drop_expired(&self, now_unix: i64) -> Result<usize, AlmanacError> {
        let cutoff = now_unix - SET_ASIDE_MAX_AGE_DAYS * 86_400;
        let gone = self
            .db()?
            .write(move |tx| {
                let mut stmt = tx.prepare(
                    "SELECT id, source_id, reason, set_aside_at FROM journal
                        WHERE state = 'dead' AND set_aside_unix > 0 AND set_aside_unix < ?1",
                )?;
                let rows = stmt
                    .query_map(params![cutoff], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, Option<String>>(2)?,
                            r.get::<_, Option<String>>(3)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                drop(stmt);
                tx.execute(
                    "DELETE FROM journal WHERE state = 'dead' AND set_aside_unix > 0
                        AND set_aside_unix < ?1",
                    params![cutoff],
                )?;
                Ok(rows)
            })
            .await
            .map_err(db_error)?;
        for (id, source, reason, at) in &gone {
            tracing::info!(
                entry_id = %id, source_id = %source,
                reason = reason.as_deref().unwrap_or(""), set_aside_at = at.as_deref().unwrap_or(""),
                "removed a set-aside event after {SET_ASIDE_MAX_AGE_DAYS} days"
            );
        }
        Ok(gone.len())
    }
}
