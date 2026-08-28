//! What the debug surface (K11) and the raw-capture surface (M11)
//! remember, and the retention rules that keep either from growing
//! without bound. Pure: the clock arrives as a parameter rather than
//! being read here (AR13), so retention is testable without waiting.
//!
//! Both are deliberately in-memory and lossy. They exist to answer
//! "what just happened" while debugging, not to be a record — the
//! journal and Google hold everything that matters durably (AR16).

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// One processed event's route, for K11: what arrived, which profile
/// handled it, and what became of it at Google.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouteRecord {
    pub at: String,
    pub source_id: String,
    pub entry_id: String,
    /// The upsert key the delivery resolved to, when it had one.
    pub upsert_key: Option<String>,
    pub outcome: RouteOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum RouteOutcome {
    Created {
        event_id: String,
    },
    Updated {
        event_id: String,
    },
    /// Carries the remedy as well as the message: a debug surface that
    /// shows what broke without saying what to do about it is half a
    /// tool (standing rule 11).
    Failed {
        message: String,
        remedy: String,
    },
}

/// One verbatim inbound request, for M11: exactly what a source sent,
/// before any profile has been written for it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptureRecord {
    pub at: String,
    /// Unix seconds, used for expiry. Kept alongside the human-readable
    /// `at` so retention never has to parse a timestamp back.
    pub at_unix: u64,
    pub label: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// Set when the body was longer than the per-capture limit. Named
    /// explicitly rather than silently cutting (standing rule 12).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated_from_bytes: Option<usize>,
}

/// A bounded, newest-first history. Oldest entries fall off the end
/// once `capacity` is reached.
#[derive(Debug)]
pub struct RingBuffer<T> {
    items: VecDeque<T>,
    capacity: usize,
}

impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            items: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    pub fn push(&mut self, item: T) {
        self.items.push_front(item);
        while self.items.len() > self.capacity {
            self.items.pop_back();
        }
    }

    /// Newest first.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.items.iter()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn retain<F: FnMut(&T) -> bool>(&mut self, predicate: F) {
        self.items.retain(predicate);
    }
}

/// Drops captures older than `ttl_secs`. Called before every read and
/// write of the capture store, so an abandoned capture label cannot
/// hold someone's payload in memory indefinitely.
pub fn expire_captures(buffer: &mut RingBuffer<CaptureRecord>, now_unix: u64, ttl_secs: u64) {
    buffer.retain(|record| now_unix.saturating_sub(record.at_unix) < ttl_secs);
}

/// Shortens an over-long body and reports what was cut. Never silently
/// truncates: the caller records `truncated_from_bytes` so the debug
/// surface can say so out loud.
pub fn truncate_body(body: &str, max_bytes: usize) -> (String, Option<usize>) {
    if body.len() <= max_bytes {
        return (body.to_string(), None);
    }
    let mut end = max_bytes;
    // Never split a multi-byte character.
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    (body[..end].to_string(), Some(body.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(label: &str, at_unix: u64) -> CaptureRecord {
        CaptureRecord {
            at: "2026-08-28T09:00:00+00:00".to_string(),
            at_unix,
            label: label.to_string(),
            method: "POST".to_string(),
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: "{}".to_string(),
            truncated_from_bytes: None,
        }
    }

    #[test]
    fn the_ring_buffer_returns_newest_first() {
        let mut ring = RingBuffer::new(10);
        ring.push(capture("a", 1));
        ring.push(capture("b", 2));
        let labels: Vec<_> = ring.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, vec!["b", "a"]);
    }

    #[test]
    fn the_ring_buffer_drops_the_oldest_past_capacity() {
        let mut ring = RingBuffer::new(2);
        ring.push(capture("a", 1));
        ring.push(capture("b", 2));
        ring.push(capture("c", 3));
        let labels: Vec<_> = ring.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["c", "b"],
            "the oldest must fall off, not the newest"
        );
        assert_eq!(ring.len(), 2);
    }

    #[test]
    fn a_zero_capacity_still_keeps_one_rather_than_silently_keeping_nothing() {
        let mut ring = RingBuffer::new(0);
        ring.push(capture("a", 1));
        assert_eq!(ring.len(), 1);
    }

    #[test]
    fn expiry_drops_only_what_is_older_than_the_ttl() {
        let mut ring = RingBuffer::new(10);
        ring.push(capture("old", 1_000)); // 4000s old at now=5000
        ring.push(capture("fresh", 3_000)); // 2000s old

        expire_captures(&mut ring, 5_000, 3_600);

        let labels: Vec<_> = ring.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, vec!["fresh"], "only the one past the TTL goes");
    }

    #[test]
    fn a_record_exactly_at_the_ttl_boundary_expires() {
        // Stated explicitly because "older than" versus "at least as
        // old as" is the kind of off-by-one that silently keeps data
        // an hour longer than the documented retention.
        let mut ring = RingBuffer::new(10);
        ring.push(capture("boundary", 1_000));
        expire_captures(&mut ring, 4_600, 3_600); // exactly 3600s old
        assert!(ring.is_empty());
    }

    #[test]
    fn expiry_is_safe_when_a_record_looks_newer_than_now() {
        // A clock adjustment could produce this; it must not panic on
        // the subtraction.
        let mut ring = RingBuffer::new(10);
        ring.push(capture("future", 9_000));
        expire_captures(&mut ring, 1_000, 3_600);
        assert_eq!(ring.len(), 1);
    }

    #[test]
    fn a_short_body_is_left_alone() {
        let (body, truncated) = truncate_body("hello", 100);
        assert_eq!(body, "hello");
        assert_eq!(truncated, None);
    }

    #[test]
    fn a_long_body_is_cut_and_says_so() {
        let long = "x".repeat(200);
        let (body, truncated) = truncate_body(&long, 50);
        assert_eq!(body.len(), 50);
        assert_eq!(truncated, Some(200), "the original length must be reported");
    }

    #[test]
    fn truncation_never_splits_a_multibyte_character() {
        // "é" is two bytes; cutting at 3 would land mid-character and
        // panic on a naive slice.
        let body = "aéé";
        let (cut, truncated) = truncate_body(body, 3);
        assert!(truncated.is_some());
        assert_eq!(cut, "aé", "must step back to a character boundary");
    }

    #[test]
    fn a_route_outcome_round_trips_through_json() {
        let record = RouteRecord {
            at: "2026-08-28T09:00:00+00:00".to_string(),
            source_id: "home-assistant".to_string(),
            entry_id: "j1".to_string(),
            upsert_key: Some("home-assistant:switch.wasmachine".to_string()),
            outcome: RouteOutcome::Created {
                event_id: "evt123".to_string(),
            },
        };
        let json = serde_json::to_string(&record).unwrap();
        let back: RouteRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, record);
    }

    #[test]
    fn a_failed_outcome_carries_its_remedy() {
        let outcome = RouteOutcome::Failed {
            message: "Calendar API returned HTTP 404".to_string(),
            remedy: "check the event id and calendar id".to_string(),
        };
        let json = serde_json::to_value(&outcome).unwrap();
        assert_eq!(json["result"], "failed");
        assert!(json["remedy"].as_str().unwrap().contains("check"));
    }
}
