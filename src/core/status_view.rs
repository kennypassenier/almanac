//! What the status page's Journal section shows, worked out from the
//! route ring and the journal (A2-2, 4.3.0). Pure: the ring's records,
//! the ids still pending or set aside and the clock all arrive as
//! parameters (AR13), so the folding is testable without a service.
//!
//! The ring holds one record per delivery ATTEMPT; the page shows one row
//! per journal ENTRY, so an entry that failed twice and then went through
//! is one row that ends as Created or Updated, not three.

use std::collections::HashSet;

use chrono::{DateTime, Timelike, Utc};

use crate::core::observability::{RouteOutcome, RouteRecord};

/// How a journal entry ended, as the Result column says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryResult {
    Created,
    Updated,
    /// The latest attempt failed and the entry is still in the journal's
    /// pending set: the worker tries it again on its own.
    Retrying,
    /// The entry was set aside as undeliverable (T1), or its failure has
    /// no entry left in the journal to retry.
    Failed,
}

impl DeliveryResult {
    pub fn as_str(self) -> &'static str {
        match self {
            DeliveryResult::Created => "created",
            DeliveryResult::Updated => "updated",
            DeliveryResult::Retrying => "retrying",
            DeliveryResult::Failed => "failed",
        }
    }
}

/// One row of the Recent deliveries table.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryView {
    /// The journal entry id: stable across attempts, so the page updates
    /// the row in place when a retry succeeds.
    pub id: String,
    /// When the latest attempt ran (RFC 3339).
    pub at: String,
    pub source_id: String,
    pub result: DeliveryResult,
    /// The latest failure, for a Retrying or Failed row.
    pub error: Option<String>,
}

/// Folds the ring (newest first) into one row per entry, newest first.
///
/// `pending` is `None` when the journal could not be read: a failed
/// attempt is then shown as Failed rather than guessed to be retrying.
pub fn fold_deliveries(
    routes: &[RouteRecord],
    pending: Option<&HashSet<String>>,
    dead: &HashSet<String>,
) -> Vec<DeliveryView> {
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    for route in routes {
        if !seen.insert(route.entry_id.as_str()) {
            continue;
        }
        let (result, error) = match &route.outcome {
            RouteOutcome::Created { .. } => (DeliveryResult::Created, None),
            RouteOutcome::Updated { .. } => (DeliveryResult::Updated, None),
            RouteOutcome::Failed { message, .. } => {
                let retrying = !dead.contains(&route.entry_id)
                    && pending.is_some_and(|p| p.contains(&route.entry_id));
                let result = if retrying {
                    DeliveryResult::Retrying
                } else {
                    DeliveryResult::Failed
                };
                (result, Some(message.clone()))
            }
        };
        rows.push(DeliveryView {
            id: route.entry_id.clone(),
            at: route.at.clone(),
            source_id: route.source_id.clone(),
            result,
            error,
        });
    }
    rows
}

/// How many rows ended each way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Outcomes {
    pub created: usize,
    pub updated: usize,
    pub failed: usize,
}

pub fn outcomes(rows: &[DeliveryView]) -> Outcomes {
    let count = |wanted: DeliveryResult| rows.iter().filter(|r| r.result == wanted).count();
    Outcomes {
        created: count(DeliveryResult::Created),
        updated: count(DeliveryResult::Updated),
        failed: count(DeliveryResult::Failed),
    }
}

/// Deliveries per hour over the last 24 hours, from the ring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trend {
    /// Start of the first bucket, Unix milliseconds.
    pub start_ms: i64,
    pub step_ms: i64,
    /// 25 buckets, oldest first; the last is the hour still running.
    pub values: Vec<u32>,
    /// Deliveries in the 60 minutes before `now`.
    pub last_hour: u32,
}

const HOUR_MS: i64 = 3_600_000;
const BUCKETS: i64 = 25;

/// Counts every successful attempt in the ring into hourly buckets. Whole
/// hours are the same instants in UTC and Brussels, so the buckets line up
/// with the clock the reader sees.
pub fn hourly_trend(routes: &[RouteRecord], now: DateTime<Utc>) -> Trend {
    let this_hour = now
        .with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(now)
        .timestamp_millis();
    let start_ms = this_hour - (BUCKETS - 1) * HOUR_MS;
    let mut values = vec![0u32; BUCKETS as usize];
    let mut last_hour = 0;
    for route in routes {
        if matches!(route.outcome, RouteOutcome::Failed { .. }) {
            continue;
        }
        let Ok(at) = DateTime::parse_from_rfc3339(&route.at) else {
            continue;
        };
        let ms = at.timestamp_millis();
        if ms >= start_ms && ms < this_hour + HOUR_MS {
            values[((ms - start_ms) / HOUR_MS) as usize] += 1;
        }
        if ms > now.timestamp_millis() - HOUR_MS && ms <= now.timestamp_millis() {
            last_hour += 1;
        }
    }
    Trend {
        start_ms,
        step_ms: HOUR_MS,
        values,
        last_hour,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(entry: &str, at: &str, outcome: RouteOutcome) -> RouteRecord {
        RouteRecord {
            at: at.to_string(),
            source_id: "kyu".to_string(),
            entry_id: entry.to_string(),
            upsert_key: None,
            outcome,
        }
    }

    fn failed() -> RouteOutcome {
        RouteOutcome::Failed {
            message: "rate limited".to_string(),
            remedy: "wait".to_string(),
        }
    }

    fn created() -> RouteOutcome {
        RouteOutcome::Created {
            event_id: "e".to_string(),
        }
    }

    #[test]
    fn an_entry_that_failed_and_then_went_through_is_one_created_row() {
        let routes = vec![
            route("a", "2026-10-19T10:05:00+00:00", created()),
            route("a", "2026-10-19T10:00:00+00:00", failed()),
        ];
        let rows = fold_deliveries(&routes, Some(&HashSet::new()), &HashSet::new());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].result, DeliveryResult::Created);
    }

    #[test]
    fn a_failure_still_pending_is_retrying_and_a_dead_one_is_failed() {
        let routes = vec![
            route("a", "2026-10-19T10:05:00+00:00", failed()),
            route("b", "2026-10-19T10:00:00+00:00", failed()),
        ];
        let pending: HashSet<String> = ["a".to_string()].into();
        let dead: HashSet<String> = ["b".to_string()].into();
        let rows = fold_deliveries(&routes, Some(&pending), &dead);
        assert_eq!(rows[0].result, DeliveryResult::Retrying);
        assert_eq!(rows[1].result, DeliveryResult::Failed);
    }

    #[test]
    fn an_unreadable_journal_never_guesses_retrying() {
        let routes = vec![route("a", "2026-10-19T10:05:00+00:00", failed())];
        let rows = fold_deliveries(&routes, None, &HashSet::new());
        assert_eq!(rows[0].result, DeliveryResult::Failed);
    }

    #[test]
    fn the_trend_buckets_by_the_clock_hour() {
        let now = DateTime::parse_from_rfc3339("2026-10-19T10:30:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let routes = vec![
            route("a", "2026-10-19T10:20:00+00:00", created()),
            route("b", "2026-10-19T09:50:00+00:00", created()),
            route("c", "2026-10-19T09:10:00+00:00", created()),
            route("d", "2026-10-19T09:15:00+00:00", failed()),
        ];
        let trend = hourly_trend(&routes, now);
        assert_eq!(trend.values.len(), 25);
        assert_eq!(trend.values[24], 1);
        assert_eq!(trend.values[23], 2);
        assert_eq!(trend.last_hour, 2);
    }
}
