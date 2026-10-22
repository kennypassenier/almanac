//! The durable per-day delivery history behind the History page. The
//! in-memory route ring (K11) forgets on restart; this record is what the
//! month heatmap and the week agenda's "delivered" lines are built from.
//! Pure: the clock arrives as a timestamp, the database is
//! `shell::history_store`'s.
//!
//! Since 4.4.0 the history is the `deliveries` table of the kit's app
//! database (one row per final outcome of a journal entry). The [`Book`]
//! types remain as the shape of the old `history.json`, which is read once
//! by the import and never written again.
//!
//! Bounded on purpose (standing rule 12): rows older than [`MAX_DAYS`] days
//! are finished by their own definition and pruned by the kit, and at most
//! [`MAX_DELIVERIES_PER_DAY`] rows are kept per day. The per-day counters
//! are counted from the rows, so they stop at that cap.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// The zone that decides which day a delivery belongs to.
pub const ZONE: chrono_tz::Tz = chrono_tz::Europe::Brussels;
/// Days kept; older rows are finished by definition and pruned.
pub const MAX_DAYS: usize = 400;
/// Rows kept per day (the counters are not capped).
pub const MAX_DELIVERIES_PER_DAY: usize = 500;
/// Format version of the old `history.json`.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryResult {
    Created,
    Updated,
    Failed,
}

impl DeliveryResult {
    /// The word stored in the `result` column.
    pub fn as_str(self) -> &'static str {
        match self {
            DeliveryResult::Created => "created",
            DeliveryResult::Updated => "updated",
            DeliveryResult::Failed => "failed",
        }
    }

    /// The inverse of [`DeliveryResult::as_str`].
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "created" => Some(DeliveryResult::Created),
            "updated" => Some(DeliveryResult::Updated),
            "failed" => Some(DeliveryResult::Failed),
            _ => None,
        }
    }
}

/// One event Almanac finished with: delivered, or set aside as failed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeliveryRecord {
    /// When it was delivered (RFC 3339).
    pub at: String,
    /// The journal entry, so a retry replaces its earlier record.
    pub entry_id: String,
    pub source: String,
    pub calendar_id: String,
    pub title: String,
    pub result: DeliveryResult,
    /// One plain sentence, only for a failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The `almanac_source_id` the event carries on the calendar, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upsert_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct DayRecord {
    /// `YYYY-MM-DD` in [`ZONE`].
    pub date: String,
    pub accepted: u32,
    pub delivered: u32,
    pub failed: u32,
    pub deliveries: Vec<DeliveryRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Book {
    pub version: u32,
    /// Oldest first.
    pub days: Vec<DayRecord>,
}

impl Default for Book {
    fn default() -> Self {
        Self {
            version: FORMAT_VERSION,
            days: Vec::new(),
        }
    }
}

/// The day an RFC 3339 moment falls on in [`ZONE`].
pub fn day_key(rfc3339: &str) -> Option<String> {
    let at = DateTime::parse_from_rfc3339(rfc3339).ok()?;
    Some(at.with_timezone(&ZONE).format("%Y-%m-%d").to_string())
}

/// `YYYY-MM-DD` is a real date.
pub fn is_day(text: &str) -> bool {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok()
}

/// How long a row is kept, in seconds: after this it is finished by
/// definition and the kit prunes it.
pub const fn retention_secs() -> i64 {
    MAX_DAYS as i64 * 86_400
}

/// The instant an RFC 3339 moment names, as unix seconds.
pub fn epoch_of(rfc3339: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(rfc3339)
        .ok()
        .map(|at| at.timestamp())
}

/// Unix seconds as RFC 3339 (UTC), the shape `DeliveryRecord::at` has.
pub fn rfc3339_of(epoch: i64) -> String {
    DateTime::<Utc>::from_timestamp(epoch, 0)
        .unwrap_or_default()
        .to_rfc3339()
}

/// One row of the `deliveries` table.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The journal entry id; a retry replaces the row.
    pub id: String,
    pub delivered_at: i64,
    /// `YYYY-MM-DD` in [`ZONE`].
    pub day: String,
    pub source: String,
    pub calendar: String,
    pub title: String,
    pub result: DeliveryResult,
    pub reason: Option<String>,
    pub upsert_key: Option<String>,
}

impl Row {
    /// The row for a record; `None` when its timestamp is unreadable.
    pub fn from_record(rec: &DeliveryRecord) -> Option<Row> {
        Some(Row {
            id: rec.entry_id.clone(),
            delivered_at: epoch_of(&rec.at)?,
            day: day_key(&rec.at)?,
            source: rec.source.clone(),
            calendar: rec.calendar_id.clone(),
            title: rec.title.clone(),
            result: rec.result,
            reason: rec.reason.clone(),
            upsert_key: rec.upsert_key.clone(),
        })
    }

    /// Back to the record shape the History page renders.
    pub fn to_record(&self) -> DeliveryRecord {
        DeliveryRecord {
            at: rfc3339_of(self.delivered_at),
            entry_id: self.id.clone(),
            source: self.source.clone(),
            calendar_id: self.calendar.clone(),
            title: self.title.clone(),
            result: self.result,
            reason: self.reason.clone(),
            upsert_key: self.upsert_key.clone(),
        }
    }
}

/// Every row an old `history.json` holds, oldest day first. An unreadable
/// timestamp is an error, never a silently dropped row: the import would
/// otherwise lose a delivery without anyone being told.
pub fn rows_from_book(book: &Book) -> Result<Vec<Row>, String> {
    let mut rows = Vec::new();
    for rec in book.days.iter().flat_map(|d| d.deliveries.iter()) {
        match Row::from_record(rec) {
            Some(row) => rows.push(row),
            None => {
                return Err(format!(
                    "delivery {} has an unreadable timestamp {:?}",
                    rec.entry_id, rec.at
                ));
            }
        }
    }
    Ok(rows)
}

/// One day's counters, as the heatmap reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayCount {
    pub date: String,
    pub accepted: u32,
    pub delivered: u32,
    pub failed: u32,
}

/// One day's counters from `(result, how many rows)` pairs.
pub fn count_day(date: &str, groups: impl IntoIterator<Item = (DeliveryResult, u32)>) -> DayCount {
    let mut count = DayCount {
        date: date.to_string(),
        accepted: 0,
        delivered: 0,
        failed: 0,
    };
    for (result, n) in groups {
        count.accepted += n;
        match result {
            DeliveryResult::Failed => count.failed += n,
            _ => count.delivered += n,
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(at: &str, entry: &str, result: DeliveryResult) -> DeliveryRecord {
        DeliveryRecord {
            at: at.to_string(),
            entry_id: entry.to_string(),
            source: "s".to_string(),
            calendar_id: "c".to_string(),
            title: "t".to_string(),
            result,
            reason: None,
            upsert_key: Some("s:k".to_string()),
        }
    }

    #[test]
    fn the_day_is_the_brussels_day() {
        assert_eq!(
            day_key("2026-10-18T22:30:00+00:00").as_deref(),
            Some("2026-10-19")
        );
    }

    #[test]
    fn a_record_survives_the_trip_through_a_row() {
        let original = rec("2026-10-19T08:16:00+00:00", "e1", DeliveryResult::Updated);
        let row = Row::from_record(&original).unwrap();
        assert_eq!(row.day, "2026-10-19");
        assert_eq!(row.to_record(), original);
    }

    #[test]
    fn an_unreadable_timestamp_is_an_error_not_a_dropped_row() {
        let book = Book {
            version: FORMAT_VERSION,
            days: vec![DayRecord {
                date: "2026-10-19".to_string(),
                accepted: 1,
                delivered: 1,
                failed: 0,
                deliveries: vec![rec("yesterday", "e1", DeliveryResult::Created)],
            }],
        };
        assert!(rows_from_book(&book).unwrap_err().contains("e1"));
    }

    #[test]
    fn rows_from_a_book_keep_every_delivery_in_order() {
        let book = Book {
            version: FORMAT_VERSION,
            days: vec![
                DayRecord {
                    date: "2026-10-18".to_string(),
                    accepted: 1,
                    delivered: 1,
                    failed: 0,
                    deliveries: vec![rec(
                        "2026-10-18T10:00:00+00:00",
                        "a",
                        DeliveryResult::Created,
                    )],
                },
                DayRecord {
                    date: "2026-10-19".to_string(),
                    accepted: 2,
                    delivered: 1,
                    failed: 1,
                    deliveries: vec![
                        rec("2026-10-19T10:00:00+00:00", "b", DeliveryResult::Failed),
                        rec("2026-10-19T11:00:00+00:00", "c", DeliveryResult::Updated),
                    ],
                },
            ],
        };
        let ids: Vec<_> = rows_from_book(&book)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }

    #[test]
    fn the_counters_split_failures_from_deliveries() {
        let count = count_day(
            "2026-10-19",
            [
                (DeliveryResult::Created, 1),
                (DeliveryResult::Updated, 1),
                (DeliveryResult::Failed, 1),
            ],
        );
        assert_eq!((count.accepted, count.delivered, count.failed), (3, 2, 1));
    }

    #[test]
    fn results_round_trip_through_their_word() {
        for r in [
            DeliveryResult::Created,
            DeliveryResult::Updated,
            DeliveryResult::Failed,
        ] {
            assert_eq!(DeliveryResult::parse(r.as_str()), Some(r));
        }
        assert_eq!(DeliveryResult::parse("other"), None);
    }
}
