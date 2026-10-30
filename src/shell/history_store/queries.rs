//! The reads behind the History page and the week agenda.

use std::collections::{BTreeMap, HashMap};

use super::rows::{SELECT_ROW, read_row};
use crate::core::history::{self, DayCount, DeliveryRecord, DeliveryResult};
use crate::shell::state::AppState;

/// Every day's counters, oldest first: the heatmap reads them in one go.
pub async fn day_counts(state: &AppState) -> Result<Vec<DayCount>, chassis::Error> {
    state
        .app_db()?
        .read(|c| {
            let mut stmt = c.prepare(
                "SELECT day, result, COUNT(*) FROM deliveries GROUP BY day, result ORDER BY day",
            )?;
            let mut by_day: BTreeMap<String, Vec<(DeliveryResult, u32)>> = BTreeMap::new();
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u32>(2)?,
                ))
            })?;
            for row in rows {
                let (day, word, n) = row?;
                match DeliveryResult::parse(&word) {
                    Some(result) => by_day.entry(day).or_default().push((result, n)),
                    None => tracing::warn!(
                        result = %word,
                        day = %day,
                        "a delivery row has a result this build does not know; not counted"
                    ),
                }
            }
            Ok(by_day
                .into_iter()
                .map(|(day, groups)| history::count_day(&day, groups))
                .collect())
        })
        .await
}

/// A read that kept going past rows it could not read: what it read,
/// and how many rows it left out (each is in the log). The page says
/// how many are missing rather than failing whole or hiding them.
pub struct Partial<T> {
    pub rows: T,
    pub unreadable: usize,
}

/// One row of a query, or `None` after logging why it was left out.
fn readable<T>(row: chassis::storage::rusqlite::Result<Option<T>>, what: &str) -> Option<T> {
    match row {
        Ok(Some(row)) => Some(row),
        // `read_row` has already logged the result word it did not know.
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(error = %e, what, "a delivery row could not be read; it is left out");
            None
        }
    }
}

/// One day's rows in the order they were recorded.
pub async fn day_rows(
    state: &AppState,
    date: &str,
) -> Result<Partial<Vec<DeliveryRecord>>, chassis::Error> {
    let date = date.to_string();
    state
        .app_db()?
        .read(move |c| {
            let mut stmt = c.prepare(&format!("{SELECT_ROW} WHERE day = ?1 ORDER BY rowid"))?;
            let mut out = Partial {
                rows: Vec::new(),
                unreadable: 0,
            };
            for row in stmt.query_map([&date], read_row)? {
                match readable(row, "day") {
                    Some(row) => out.rows.push(row.to_record()),
                    None => out.unreadable += 1,
                }
            }
            Ok(out)
        })
        .await
}

/// The newest record per upsert key, for the week agenda's "delivered" line.
pub async fn latest_by_key(
    state: &AppState,
    keys: Vec<String>,
) -> Result<Partial<HashMap<String, DeliveryRecord>>, chassis::Error> {
    state
        .app_db()?
        .read(move |c| {
            let mut stmt = c.prepare(&format!(
                "{SELECT_ROW} WHERE upsert_key = ?1 ORDER BY delivered_at DESC, rowid DESC LIMIT 1"
            ))?;
            let mut out = Partial {
                rows: HashMap::new(),
                unreadable: 0,
            };
            for key in keys {
                // The query returns at most one row (LIMIT 1).
                for row in stmt.query_map([&key], read_row)? {
                    match readable(row, "latest") {
                        Some(row) => {
                            out.rows.insert(key.clone(), row.to_record());
                        }
                        None => out.unreadable += 1,
                    }
                }
            }
            Ok(out)
        })
        .await
}
