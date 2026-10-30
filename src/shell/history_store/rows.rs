//! The SQL shared by the import and the live inserts, and the row reader.

use chassis::storage::rusqlite::{Transaction, params};

use crate::core::history::{DeliveryResult, MAX_DELIVERIES_PER_DAY, Row};

pub(super) const INSERT: &str = "INSERT OR REPLACE INTO deliveries
    (id, delivered_at, day, source, calendar, title, result, reason, upsert_key)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)";

pub(super) const SELECT_ROW: &str = "SELECT id, delivered_at, day, source, calendar, title, result, reason, upsert_key FROM deliveries";

/// The bound parameters of a [`Row`], in the order of [`INSERT`].
macro_rules! row_params {
    ($r:expr) => {
        chassis::storage::rusqlite::params![
            $r.id,
            $r.delivered_at,
            $r.day,
            $r.source,
            $r.calendar,
            $r.title,
            $r.result.as_str(),
            $r.reason,
            $r.upsert_key,
        ]
    };
}
pub(super) use row_params;

/// Inserts or replaces one delivery (a retried entry replaces its earlier
/// record), then keeps the day within [`MAX_DELIVERIES_PER_DAY`] rows.
pub fn insert(tx: &Transaction, row: &Row) -> Result<(), chassis::Error> {
    tx.execute(INSERT, row_params!(row))?;
    tx.execute(
        "DELETE FROM deliveries WHERE day = ?1 AND rowid NOT IN
            (SELECT rowid FROM deliveries WHERE day = ?1 ORDER BY rowid DESC LIMIT ?2)",
        params![row.day, MAX_DELIVERIES_PER_DAY as i64],
    )?;
    Ok(())
}

/// A row of [`SELECT_ROW`]; `None` for a result word this build does not
/// know (logged, so a table written by a newer build is not skipped silently).
pub(super) fn read_row(
    r: &chassis::storage::rusqlite::Row,
) -> chassis::storage::rusqlite::Result<Option<Row>> {
    let word: String = r.get(6)?;
    let Some(result) = DeliveryResult::parse(&word) else {
        tracing::warn!(result = %word, "a delivery row has a result this build does not know; skipped");
        return Ok(None);
    };
    Ok(Some(Row {
        id: r.get(0)?,
        delivered_at: r.get(1)?,
        day: r.get(2)?,
        source: r.get(3)?,
        calendar: r.get(4)?,
        title: r.get(5)?,
        result,
        reason: r.get(7)?,
        upsert_key: r.get(8)?,
    }))
}
