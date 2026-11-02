//! M13 — the numbers Prometheus scrapes, and how they are rendered.
//!
//! Almanac already counted most of this for the debug page and threw it
//! away on every restart. A Prometheus on CT 113 scrapes the rest of
//! the fleet, so the counters live here instead: still in memory, still
//! reset by a restart (that is what a `_total` counter is allowed to
//! do), but readable by something that keeps history.
//!
//! Kept in `core` and free of I/O: the counters are plain atomics and
//! the renderer is a pure function of them plus whatever the shell
//! looks up at scrape time. That is what makes the "no secret ever
//! reaches this output" claim testable without standing up a server.
//!
//! **Nothing here is labelled by source.** A per-source breakdown would
//! be genuinely useful, and is deliberately left out: source ids are
//! chosen by whoever is integrated, and a label is one careless profile
//! away from carrying a household detail into a metrics database that
//! is scraped, stored and rendered on a dashboard for years. The debug
//! surface, which is authenticated, is where per-source detail belongs.

use std::sync::atomic::{AtomicU64, Ordering};

/// Every counter Almanac keeps. Monotonic since process start.
#[derive(Debug, Default)]
pub struct Metrics {
    /// Payloads accepted at ingest and written to the journal.
    accepted: AtomicU64,
    /// Journal entries delivered to Google and marked done.
    delivered: AtomicU64,
    /// Delivery attempts that failed, transient ones included — so this
    /// climbing while `delivered` also climbs is a retry story, not an
    /// outage.
    failed: AtomicU64,
    /// Entries given up on and set aside as dead (T1).
    dead: AtomicU64,
    /// Access tokens fetched from Google. One per hour is normal; a
    /// rate far above that means the cache is not doing its job.
    token_refreshes: AtomicU64,
    /// Deliveries that reached Google but could not be written into the
    /// history database. The delivery itself is fine; the log of it is not.
    history_write_failures: AtomicU64,
    /// Unix seconds of the first such failure this run, 0 while there is
    /// none. The first one, not the last: "since when" is what tells the
    /// reader whether the disk filled up an hour or a week ago.
    history_failing_since: AtomicU64,
}

impl Metrics {
    pub fn accepted(&self) {
        self.accepted.fetch_add(1, Ordering::Relaxed);
    }

    pub fn delivered(&self, n: u64) {
        self.delivered.fetch_add(n, Ordering::Relaxed);
    }

    pub fn failed(&self, n: u64) {
        self.failed.fetch_add(n, Ordering::Relaxed);
    }

    pub fn dead(&self, n: u64) {
        self.dead.fetch_add(n, Ordering::Relaxed);
    }

    pub fn token_refreshed(&self) {
        self.token_refreshes.fetch_add(1, Ordering::Relaxed);
    }

    /// Notes that a delivery could not be written into the history database.
    pub fn history_write_failed(&self, now_unix: u64) {
        self.history_write_failures.fetch_add(1, Ordering::Relaxed);
        // Only the first failure sets the time; later ones leave it be.
        let _ = self.history_failing_since.compare_exchange(
            0,
            now_unix.max(1),
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
    }

    /// Renders the Prometheus text exposition format.
    ///
    /// `pending` is the journal depth, looked up by the caller at
    /// scrape time because reading it is I/O. `None` means the journal
    /// could not be read — reported as its own gauge rather than as a
    /// zero, since "nothing is waiting" and "I cannot tell you what is
    /// waiting" are opposite kinds of news and a dashboard must not
    /// show the alarming one as the reassuring one.
    /// `_version` stays in the signature for callers; the kit emits
    /// `almanac_build_info{version}` itself since 3.0.0.
    pub fn render(&self, pending: Option<u64>, _version: &str) -> String {
        let counts = self.snapshot();
        let mut out = String::new();
        for (name, help, value) in [
            (
                "almanac_events_accepted_total",
                "Payloads accepted at ingest and written to the journal.",
                counts.accepted,
            ),
            (
                "almanac_events_delivered_total",
                "Journal entries delivered to Google Calendar.",
                counts.delivered,
            ),
            (
                "almanac_deliveries_failed_total",
                "Delivery attempts that failed, including ones later retried successfully.",
                counts.failed,
            ),
            (
                "almanac_entries_dead_total",
                "Entries given up on after repeated permanent failures.",
                counts.dead,
            ),
            (
                "almanac_token_refreshes_total",
                "Access tokens fetched from Google.",
                counts.token_refreshes,
            ),
            (
                "almanac_history_write_failures_total",
                "Deliveries that could not be written into the delivery history database.",
                counts.history_write_failures,
            ),
        ] {
            push_counter(&mut out, name, help, value);
        }
        push_journal_gauges(&mut out, pending);
        // The conventional way to expose a version: a constant 1 with
        // the version as a label, so a dashboard can group by it and an
        // alert can fire on a fleet running mixed versions.
        // almanac_build_info is emitted by the kit since 3.0.0 (same name, same label).
        out
    }
}

/// Appends one counter family: help line, type line, value line.
fn push_counter(out: &mut String, name: &str, help: &str, value: u64) {
    out.push_str(&format!(
        "# HELP {name} {help}\n# TYPE {name} counter\n{name} {value}\n"
    ));
}

/// Appends the two journal gauges: the depth (omitted when unreadable) and
/// whether the journal could be read at all.
fn push_journal_gauges(out: &mut String, pending: Option<u64>) {
    out.push_str(
        "# HELP almanac_journal_pending Entries accepted but not yet delivered.\n\
         # TYPE almanac_journal_pending gauge\n",
    );
    match pending {
        Some(n) => out.push_str(&format!("almanac_journal_pending {n}\n")),
        None => out.push_str("# journal unreadable at scrape time; gauge omitted\n"),
    }
    out.push_str(
        "# HELP almanac_journal_readable Whether the journal could be read for this scrape.\n\
         # TYPE almanac_journal_readable gauge\n",
    );
    out.push_str(&format!(
        "almanac_journal_readable {}\n",
        u8::from(pending.is_some())
    ));
}

/// The counters at one moment, for something that wants to report them
/// rather than serve them (M14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub accepted: u64,
    pub delivered: u64,
    pub failed: u64,
    pub dead: u64,
    pub token_refreshes: u64,
    pub history_write_failures: u64,
    /// Unix seconds of the first history write failure, 0 when none.
    pub history_failing_since: u64,
}

/// The Status page sentence for failed history writes, or `None` when there
/// were none since the process started. It stays until a restart rather
/// than clearing on the next good write: one success does not mean the
/// disk problem is gone, and a line that vanishes on its own hides that
/// deliveries were left out of the log in the meantime.
pub fn history_failure_line(counts: &Counts) -> Option<String> {
    if counts.history_write_failures == 0 {
        return None;
    }
    let since = i64::try_from(counts.history_failing_since)
        .ok()
        .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
        .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "an unknown time".to_string());
    Some(format!(
        "The delivery log could not be written: {} time(s) since {since}. \
         Deliveries to the calendar are not affected, but they are missing from the history. \
         Free up disk space on the state folder and check the permissions of the database file (almanac.db).",
        counts.history_write_failures
    ))
}

impl Metrics {
    /// Reads all the counters at once.
    ///
    /// Not atomic across the five — they are separate counters and an
    /// event can land between two reads. That is right for a heartbeat,
    /// which answers "alive, and roughly how busy"; paying for a lock
    /// to make a log line consistent to the microsecond would be the
    /// wrong trade.
    pub fn snapshot(&self) -> Counts {
        Counts {
            accepted: self.accepted.load(Ordering::Relaxed),
            delivered: self.delivered.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            dead: self.dead.load(Ordering::Relaxed),
            token_refreshes: self.token_refreshes.load(Ordering::Relaxed),
            history_write_failures: self.history_write_failures.load(Ordering::Relaxed),
            history_failing_since: self.history_failing_since.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests;
