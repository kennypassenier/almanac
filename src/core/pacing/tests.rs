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
fn projected_passes_climb_the_ladder_and_repeat_its_last_rung() {
    assert_eq!(
        projected_passes(100, 2, 4),
        vec![100, 100 + 300, 100 + 300 + 900, 100 + 300 + 900 + 1800]
    );
    assert_eq!(projected_passes(0, 9, 3), vec![0, 1800, 3600]);
}

#[test]
fn a_set_aside_reason_reads_in_plain_words() {
    let (cause, next) = plain_set_aside_reason(
        "Calendar API returned HTTP 404 Not Found: {} — check the request payload",
    );
    assert_eq!(cause, "The calendar or the event no longer exists.");
    assert_eq!(
        next,
        "Check the request payload, then send the event again from its source."
    );
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
