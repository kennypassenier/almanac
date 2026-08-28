//! How fast the delivery worker comes back after a pass (AR26).
//!
//! Pure, because the interesting mistakes here are logic errors that a
//! real outage would take half an hour to reveal: never leaving the
//! slow lane once a blip has passed, or reporting the same backlog
//! every fifteen seconds until the alert means nothing. Both are
//! silent — one shows up as every household event arriving up to
//! thirty minutes late, forever; the other trains you to ignore the
//! notification that matters.

/// How long to wait after a pass where nothing got through. The last
/// value repeats, so a multi-hour outage settles at half-hourly rather
/// than growing without bound.
pub const FAILURE_BACKOFF_SECS: [u64; 5] = [15, 60, 300, 900, 1800];

/// The interval while deliveries are succeeding.
pub const POLL_INTERVAL_SECS: u64 = 5;

/// Warn once the journal passes this share of its cap, so there is
/// room to act before ingest starts refusing events and the sources'
/// own retries give up.
pub const JOURNAL_WARN_FRACTION: f64 = 0.5;

/// What one drain pass achieved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainSummary {
    pub delivered: usize,
    pub failed: usize,
}

/// What the loop should do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pacing {
    /// Seconds to wait before the next pass.
    pub wait_secs: u64,
    /// Whether this is the moment to report a filling journal — true
    /// at most once per outage.
    pub report_backlog: bool,
    /// Whether deliveries have just recovered, worth saying once.
    pub recovered: bool,
}

/// Tracks how an outage is going across passes.
#[derive(Debug, Default)]
pub struct Worker {
    consecutive_failures: usize,
    backlog_reported: bool,
}

impl Worker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Folds one pass into the pacing state.
    ///
    /// `journal_is_filling` is passed in rather than measured here
    /// because measuring it is I/O; the decision about what to do with
    /// it is not.
    pub fn after(&mut self, summary: DrainSummary, journal_is_filling: bool) -> Pacing {
        // Only a pass that achieved nothing counts as an outage. A
        // mixed pass — one dead entry among nine good ones — must not
        // slow the whole hub down, which is the difference between one
        // bad payload being an annoyance and being an outage (T1).
        let stalled = summary.failed > 0 && summary.delivered == 0;

        if !stalled {
            let recovered = self.consecutive_failures > 0;
            self.consecutive_failures = 0;
            // Reset, so a second outage later is reported again rather
            // than being swallowed by the first one's flag.
            self.backlog_reported = false;
            return Pacing {
                wait_secs: POLL_INTERVAL_SECS,
                report_backlog: false,
                recovered,
            };
        }

        let wait_secs = FAILURE_BACKOFF_SECS[self
            .consecutive_failures
            .min(FAILURE_BACKOFF_SECS.len() - 1)];
        self.consecutive_failures += 1;

        let report_backlog = journal_is_filling && !self.backlog_reported;
        if report_backlog {
            self.backlog_reported = true;
        }

        Pacing {
            wait_secs,
            report_backlog,
            recovered: false,
        }
    }

    pub fn consecutive_failures(&self) -> usize {
        self.consecutive_failures
    }
}

/// Whether a journal of `size` bytes against `cap` is filling up.
pub fn journal_is_filling(size: u64, cap: u64) -> bool {
    (size as f64) >= cap as f64 * JOURNAL_WARN_FRACTION
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed() -> DrainSummary {
        DrainSummary {
            delivered: 0,
            failed: 1,
        }
    }

    fn delivered() -> DrainSummary {
        DrainSummary {
            delivered: 1,
            failed: 0,
        }
    }

    fn idle() -> DrainSummary {
        DrainSummary {
            delivered: 0,
            failed: 0,
        }
    }

    #[test]
    fn a_healthy_pass_stays_on_the_fast_interval() {
        let mut worker = Worker::new();
        assert_eq!(
            worker.after(delivered(), false).wait_secs,
            POLL_INTERVAL_SECS
        );
        assert_eq!(worker.after(idle(), false).wait_secs, POLL_INTERVAL_SECS);
    }

    #[test]
    fn an_idle_hub_is_not_an_outage() {
        // Nothing to deliver must not look like everything failing, or
        // a quiet night would back the hub off to half-hourly polling.
        let mut worker = Worker::new();
        worker.after(idle(), false);
        assert_eq!(worker.consecutive_failures(), 0);
    }

    #[test]
    fn the_backoff_ladder_climbs_and_then_holds() {
        let mut worker = Worker::new();
        let waits: Vec<u64> = (0..7)
            .map(|_| worker.after(failed(), false).wait_secs)
            .collect();
        assert_eq!(waits, vec![15, 60, 300, 900, 1800, 1800, 1800]);
    }

    #[test]
    fn one_success_returns_the_hub_to_the_fast_interval() {
        // The regression that would otherwise be invisible: after a
        // single blip the hub stays on half-hourly polling forever,
        // and every household event arrives up to thirty minutes late.
        let mut worker = Worker::new();
        for _ in 0..5 {
            worker.after(failed(), false);
        }
        assert_eq!(worker.after(failed(), false).wait_secs, 1800);

        let pacing = worker.after(delivered(), false);
        assert_eq!(pacing.wait_secs, POLL_INTERVAL_SECS);
        assert!(pacing.recovered, "recovery is worth one log line");
        assert_eq!(worker.consecutive_failures(), 0);
    }

    #[test]
    fn a_partial_pass_does_not_count_as_an_outage() {
        // One permanently-undeliverable payload among nine good ones
        // must not slow every other source down (T1).
        let mut worker = Worker::new();
        let pacing = worker.after(
            DrainSummary {
                delivered: 9,
                failed: 1,
            },
            false,
        );
        assert_eq!(pacing.wait_secs, POLL_INTERVAL_SECS);
        assert_eq!(worker.consecutive_failures(), 0);
    }

    #[test]
    fn the_backlog_is_reported_once_per_outage_not_once_per_pass() {
        // At the 15-second end of the ladder, reporting every pass
        // would mean four notifications a minute. An alert that
        // frequent is one you turn off.
        let mut worker = Worker::new();

        assert!(worker.after(failed(), true).report_backlog, "the first one");
        for _ in 0..10 {
            assert!(
                !worker.after(failed(), true).report_backlog,
                "and no more while the same outage lasts"
            );
        }
    }

    #[test]
    fn a_second_outage_is_reported_again() {
        // The flag has to reset on recovery, or the only backlog you
        // ever hear about is the first one since the process started.
        let mut worker = Worker::new();
        assert!(worker.after(failed(), true).report_backlog);

        worker.after(delivered(), false);

        assert!(
            worker.after(failed(), true).report_backlog,
            "a later outage must be reported too"
        );
    }

    #[test]
    fn a_journal_with_room_is_not_reported() {
        let mut worker = Worker::new();
        assert!(!worker.after(failed(), false).report_backlog);
    }

    #[test]
    fn the_warning_threshold_is_half_the_cap() {
        assert!(!journal_is_filling(0, 100));
        assert!(!journal_is_filling(49, 100));
        assert!(journal_is_filling(50, 100), "exactly half already counts");
        assert!(journal_is_filling(99, 100));
    }

    #[test]
    fn a_zero_cap_does_not_divide_by_anything_silly() {
        assert!(journal_is_filling(0, 0));
    }
}
