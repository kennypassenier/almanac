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

/// One day's rows in the order they were recorded.
pub async fn day_rows(state: &AppState, date: &str) -> Result<Vec<DeliveryRecord>, chassis::Error> {
    let date = date.to_string();
    state
        .app_db()?
        .read(move |c| {
            let mut stmt = c.prepare(&format!("{SELECT_ROW} WHERE day = ?1 ORDER BY rowid"))?;
            let mut out = Vec::new();
            for row in stmt.query_map([&date], read_row)? {
                if let Some(row) = row? {
                    out.push(row.to_record());
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
) -> Result<HashMap<String, DeliveryRecord>, chassis::Error> {
    state
        .app_db()?
        .read(move |c| {
            let mut stmt = c.prepare(&format!(
                "{SELECT_ROW} WHERE upsert_key = ?1 ORDER BY delivered_at DESC, rowid DESC LIMIT 1"
            ))?;
            let mut out = HashMap::new();
            for key in keys {
                // The query returns at most one row (LIMIT 1); a row this
                // build cannot read is `None` and the key simply has no record.
                for row in stmt.query_map([&key], read_row)? {
                    if let Some(row) = row? {
                        out.insert(key.clone(), row.to_record());
                    }
                }
            }
            Ok(out)
        })
        .await
}
