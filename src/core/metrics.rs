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

    /// Renders the Prometheus text exposition format.
    ///
    /// `pending` is the journal depth, looked up by the caller at
    /// scrape time because reading it is I/O. `None` means the journal
    /// could not be read — reported as its own gauge rather than as a
    /// zero, since "nothing is waiting" and "I cannot tell you what is
    /// waiting" are opposite kinds of news and a dashboard must not
    /// show the alarming one as the reassuring one.
    pub fn render(&self, pending: Option<u64>, version: &str) -> String {
        let mut out = String::new();

        for (name, help, value) in [
            (
                "almanac_events_accepted_total",
                "Payloads accepted at ingest and written to the journal.",
                self.accepted.load(Ordering::Relaxed),
            ),
            (
                "almanac_events_delivered_total",
                "Journal entries delivered to Google Calendar.",
                self.delivered.load(Ordering::Relaxed),
            ),
            (
                "almanac_deliveries_failed_total",
                "Delivery attempts that failed, including ones later retried successfully.",
                self.failed.load(Ordering::Relaxed),
            ),
            (
                "almanac_entries_dead_total",
                "Entries given up on after repeated permanent failures.",
                self.dead.load(Ordering::Relaxed),
            ),
            (
                "almanac_token_refreshes_total",
                "Access tokens fetched from Google.",
                self.token_refreshes.load(Ordering::Relaxed),
            ),
        ] {
            out.push_str(&format!(
                "# HELP {name} {help}\n# TYPE {name} counter\n{name} {value}\n"
            ));
        }

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

        // The conventional way to expose a version: a constant 1 with
        // the version as a label, so a dashboard can group by it and an
        // alert can fire on a fleet running mixed versions.
        out.push_str(
            "# HELP almanac_build_info The running version, as a label on a constant 1.\n\
             # TYPE almanac_build_info gauge\n",
        );
        out.push_str(&format!("almanac_build_info{{version=\"{version}\"}} 1\n"));

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_set_of_counters_renders_all_zeroes_rather_than_nothing() {
        // An absent series and a zero series are different to
        // Prometheus: a rate() over a series that only appears on the
        // first delivery has no baseline to compare against.
        let rendered = Metrics::default().render(Some(0), "1.2.3");
        for name in [
            "almanac_events_accepted_total",
            "almanac_events_delivered_total",
            "almanac_deliveries_failed_total",
            "almanac_entries_dead_total",
            "almanac_token_refreshes_total",
        ] {
            assert!(
                rendered.contains(&format!("{name} 0\n")),
                "{name} is missing from a fresh scrape"
            );
        }
    }

    #[test]
    fn every_counter_climbs_independently() {
        let m = Metrics::default();
        m.accepted();
        m.accepted();
        m.delivered(3);
        m.failed(1);
        m.dead(4);
        m.token_refreshed();

        let rendered = m.render(Some(7), "0.1.0");
        assert!(rendered.contains("almanac_events_accepted_total 2\n"));
        assert!(rendered.contains("almanac_events_delivered_total 3\n"));
        assert!(rendered.contains("almanac_deliveries_failed_total 1\n"));
        assert!(rendered.contains("almanac_entries_dead_total 4\n"));
        assert!(rendered.contains("almanac_token_refreshes_total 1\n"));
        assert!(rendered.contains("almanac_journal_pending 7\n"));
    }

    #[test]
    fn an_unreadable_journal_is_not_reported_as_an_empty_one() {
        // The failure this guards against is quiet and expensive: a
        // journal that cannot be read renders as "0 pending", the
        // backlog dashboard goes flat and green, and the alert that
        // should fire never does.
        let rendered = Metrics::default().render(None, "0.1.0");
        assert!(
            !rendered.contains("almanac_journal_pending 0"),
            "an unreadable journal must not render as an empty one"
        );
        assert!(rendered.contains("almanac_journal_readable 0\n"));
    }

    #[test]
    fn a_readable_journal_says_so() {
        let rendered = Metrics::default().render(Some(0), "0.1.0");
        assert!(rendered.contains("almanac_journal_pending 0\n"));
        assert!(rendered.contains("almanac_journal_readable 1\n"));
    }

    #[test]
    fn the_version_is_exposed_the_conventional_way() {
        let rendered = Metrics::default().render(Some(0), "0.1.3");
        assert!(rendered.contains("almanac_build_info{version=\"0.1.3\"} 1\n"));
    }

    #[test]
    fn every_series_is_preceded_by_its_help_and_type() {
        // Not decoration: a scrape missing TYPE makes Prometheus guess
        // untyped, and rate() on an untyped series silently does the
        // wrong thing.
        let rendered = Metrics::default().render(Some(0), "0.1.0");
        for line in rendered.lines().filter(|l| !l.starts_with('#')) {
            let name = line.split([' ', '{']).next().unwrap();
            assert!(
                rendered.contains(&format!("# HELP {name} ")),
                "{name} has no HELP line"
            );
            assert!(
                rendered.contains(&format!("# TYPE {name} ")),
                "{name} has no TYPE line"
            );
        }
    }

    #[test]
    fn the_output_carries_no_identifiers_only_numbers() {
        // The M13 acceptance criterion. Deliberately checked against
        // the *shape* of the output rather than a list of forbidden
        // strings: the only text that may appear is metric names, help
        // text and the version, so anything with a value that is not a
        // number would fail this.
        let m = Metrics::default();
        m.accepted();
        let rendered = m.render(Some(1), "0.1.0");
        for line in rendered.lines().filter(|l| !l.starts_with('#')) {
            let (_, value) = line.rsplit_once(' ').expect("every series has a value");
            assert!(
                value.parse::<f64>().is_ok(),
                "a series carries something that is not a number: {line}"
            );
        }
        // And the only label anywhere is the version.
        assert_eq!(rendered.matches('{').count(), 1);
    }
}
