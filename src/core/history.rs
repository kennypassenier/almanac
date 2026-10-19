//! The durable per-day delivery history behind the History page. The
//! in-memory route ring (K11) forgets on restart; this record is what the
//! month heatmap and the week agenda's "delivered" lines are built from.
//! Pure: the clock arrives as a timestamp, the disk is `shell::history_store`'s.
//!
//! Bounded on purpose: at most [`MAX_DAYS`] days are kept and at most
//! [`MAX_DELIVERIES_PER_DAY`] rows per day, so the file cannot grow without
//! limit (standing rule 12). The per-day counters keep counting past the row
//! cap, so a very busy day still reads true.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate};
use serde::{Deserialize, Serialize};

/// The zone that decides which day a delivery belongs to.
pub const ZONE: chrono_tz::Tz = chrono_tz::Europe::Brussels;
/// Days kept; the oldest are pruned on write.
pub const MAX_DAYS: usize = 400;
/// Rows kept per day (the counters are not capped).
pub const MAX_DELIVERIES_PER_DAY: usize = 500;
/// Format version of the file, for a future migration.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryResult {
    Created,
    Updated,
    Failed,
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

/// Records one finished delivery. An earlier record of the same journal
/// entry (a failure that later succeeded) is replaced, so an event counts once.
/// Returns false when the timestamp is unreadable and nothing was recorded.
pub fn record(book: &mut Book, rec: DeliveryRecord) -> bool {
    let Some(date) = day_key(&rec.at) else {
        return false;
    };
    for day in &mut book.days {
        let before = day.deliveries.len();
        let mut removed = Vec::new();
        day.deliveries.retain(|d| {
            let same = d.entry_id == rec.entry_id;
            if same {
                removed.push(d.result);
            }
            !same
        });
        if day.deliveries.len() != before {
            for result in removed {
                day.accepted = day.accepted.saturating_sub(1);
                match result {
                    DeliveryResult::Failed => day.failed = day.failed.saturating_sub(1),
                    _ => day.delivered = day.delivered.saturating_sub(1),
                }
            }
        }
    }
    book.days.retain(|d| d.accepted > 0);

    let index = match book.days.binary_search_by(|d| d.date.as_str().cmp(&date)) {
        Ok(i) => i,
        Err(i) => {
            book.days.insert(
                i,
                DayRecord {
                    date,
                    ..DayRecord::default()
                },
            );
            i
        }
    };
    let day = &mut book.days[index];
    day.accepted += 1;
    match rec.result {
        DeliveryResult::Failed => day.failed += 1,
        _ => day.delivered += 1,
    }
    day.deliveries.push(rec);
    if day.deliveries.len() > MAX_DELIVERIES_PER_DAY {
        let extra = day.deliveries.len() - MAX_DELIVERIES_PER_DAY;
        day.deliveries.drain(..extra);
    }
    prune(book);
    true
}

/// Drops the oldest days beyond [`MAX_DAYS`].
pub fn prune(book: &mut Book) {
    if book.days.len() > MAX_DAYS {
        let extra = book.days.len() - MAX_DAYS;
        book.days.drain(..extra);
    }
}

/// The first recorded day, if any.
pub fn first_day(book: &Book) -> Option<&str> {
    book.days.first().map(|d| d.date.as_str())
}

/// The newest record per upsert key, to tell the week agenda when an event was delivered.
pub fn latest_by_key(book: &Book) -> HashMap<&str, &DeliveryRecord> {
    let mut map: HashMap<&str, &DeliveryRecord> = HashMap::new();
    for rec in book.days.iter().flat_map(|d| d.deliveries.iter()) {
        if let Some(key) = rec.upsert_key.as_deref() {
            match map.get(key) {
                Some(old) if old.at.as_str() >= rec.at.as_str() => {}
                _ => {
                    map.insert(key, rec);
                }
            }
        }
    }
    map
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
            upsert_key: None,
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
    fn a_retried_entry_counts_once() {
        let mut book = Book::default();
        record(
            &mut book,
            rec("2026-10-12T10:00:00+00:00", "e1", DeliveryResult::Failed),
        );
        record(
            &mut book,
            rec("2026-10-13T10:00:00+00:00", "e1", DeliveryResult::Created),
        );
        assert_eq!(book.days.len(), 1);
        assert_eq!(book.days[0].date, "2026-10-13");
        assert_eq!((book.days[0].accepted, book.days[0].failed), (1, 0));
    }

    #[test]
    fn old_days_are_pruned() {
        let mut book = Book::default();
        for i in 0..(MAX_DAYS + 5) {
            let date = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap() + chrono::Days::new(i as u64);
            record(
                &mut book,
                rec(
                    &format!("{date}T12:00:00+00:00"),
                    &format!("e{i}"),
                    DeliveryResult::Created,
                ),
            );
        }
        assert_eq!(book.days.len(), MAX_DAYS);
        assert_eq!(first_day(&book), Some("2025-01-06"));
    }
}
