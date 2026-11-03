//! The journal table's SQL: the schema, the insert and the row readers.

use chassis::storage::rusqlite::{Connection, Row, Transaction, params};

use crate::core::journal::{Entry, entry_key};

/// One row per event: `pending` until delivered (then deleted) or `dead`
/// when set aside. `seq` keeps acceptance order; `upsert_key` lets a
/// delete, "newer version wins" and Retry find the versions of one event.
pub(super) const SCHEMA: &str = "CREATE TABLE journal (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        id TEXT NOT NULL UNIQUE,
        source_id TEXT NOT NULL,
        received_at TEXT NOT NULL,
        payload TEXT NOT NULL,
        idempotency_key TEXT,
        upsert_key TEXT,
        state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'dead')),
        reason TEXT,
        set_aside_at TEXT,
        set_aside_unix INTEGER
    ) STRICT;
    CREATE INDEX journal_state ON journal (state, seq);
    CREATE INDEX journal_upsert_key ON journal (upsert_key);";

const COLUMNS: &str = "id, source_id, received_at, payload, idempotency_key";

/// Inserts one pending entry.
pub(super) fn insert(tx: &Transaction, entry: &Entry) -> Result<(), chassis::Error> {
    tx.execute(
        "INSERT INTO journal (id, source_id, received_at, payload, idempotency_key, upsert_key)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            entry.id,
            entry.source_id,
            entry.received_at,
            entry.payload.to_string(),
            entry.idempotency_key,
            entry_key(entry),
        ],
    )?;
    Ok(())
}

/// The entry in a row selected with [`COLUMNS`] first.
pub(super) fn entry_of(r: &Row) -> chassis::storage::rusqlite::Result<Entry> {
    let payload: String = r.get(3)?;
    Ok(Entry {
        id: r.get(0)?,
        source_id: r.get(1)?,
        received_at: r.get(2)?,
        // Written by `insert` from a parsed value, so it parses back; a
        // hand-edited row reads as null rather than failing the journal.
        payload: serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null),
        idempotency_key: r.get(4)?,
    })
}

/// Pending entries in acceptance order.
pub(super) fn pending(conn: &Connection) -> Result<Vec<Entry>, chassis::Error> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM journal WHERE state = 'pending' ORDER BY seq"
    ))?;
    let rows = stmt.query_map([], entry_of)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Set-aside entries with their reason and time, oldest first.
pub(super) fn dead(conn: &Connection) -> Result<Vec<(Entry, String, String)>, chassis::Error> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS}, reason, set_aside_at FROM journal WHERE state = 'dead' ORDER BY seq"
    ))?;
    let rows = stmt.query_map([], |r| {
        let reason: Option<String> = r.get(5)?;
        let at: Option<String> = r.get(6)?;
        Ok((
            entry_of(r)?,
            reason.unwrap_or_default(),
            at.unwrap_or_default(),
        ))
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}
