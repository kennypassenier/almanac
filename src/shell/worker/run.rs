//! The paced loop around [`drain_once_tracking`]: poll on an interval,
//! back off while an outage lasts, compact the journal now and then, and
//! drain once more on shutdown.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Interval;

use super::drain::{
    DrainOutcome, PermanentFailures, drain_for_shutdown, drain_once, drain_once_tracking,
};
use crate::core::pacing::{self, DrainSummary, Pacing};
use crate::shell::notify::{Event, Notifier, ops};
use crate::shell::state::AppState;

/// AR26's backlog warning fires once this many entries are waiting: a
/// journal that big means deliveries have been failing for a long time.
const BACKLOG_WARN_PENDING: usize = 1_000;

/// Whether the journal holds a backlog. The I/O half of AR26's warning;
/// what to do about it lives in `core::pacing`.
fn journal_is_filling(state: &AppState) -> bool {
    let pending = match state.journal.pending() {
        Ok(pending) => pending.len(),
        Err(e) => {
            // Without a count the warning cannot fire; say why instead of
            // quietly reporting a healthy journal.
            tracing::warn!(error = %e, "cannot count the journal; skipping the backlog check");
            return false;
        }
    };
    let filling = pacing::journal_is_filling(pending as u64, BACKLOG_WARN_PENDING as u64 * 2);
    if filling {
        tracing::warn!(
            pending,
            "the journal holds a backlog: deliveries have been failing long enough to pile up; \
             check the delivery errors"
        );
    }
    filling
}

/// Runs the loop until `shutdown` flips. On exit it drains once more,
/// so a graceful stop (M2) hands over a journal with as little
/// outstanding work as possible, within `SHUTDOWN_DRAIN_BUDGET`.
pub async fn run(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>, notifier: Notifier) {
    // Startup replay: whatever a crash or power cut left behind goes
    // out before anything new is accepted for delivery.
    let replayed = drain_once(&state).await;
    if replayed > 0 {
        tracing::info!(count = replayed, "replayed entries left by a previous run");
    }

    let mut permanent = PermanentFailures::new();
    let mut pace = pacing::Worker::new();
    let mut ticker = tokio::time::interval(Duration::from_secs(pacing::POLL_INTERVAL_SECS));

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let outcome = drain_once_tracking(&state, &mut permanent, &notifier).await;
                let next = pace_next(&state, &mut pace, &outcome, &notifier).await;
                reschedule(&state, &mut ticker, &next, pace.consecutive_failures()).await;
                expire_set_aside(&state).await;
            }
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    tracing::info!("worker shutting down — draining once more");
                    drain_for_shutdown(&state).await;
                    return;
                }
            }
        }
    }
}

/// AR26: slow down while an outage lasts, speed back up the moment
/// anything gets through. The decision is in `core::pacing`, which is
/// where its tests are; this reports it.
async fn pace_next(
    state: &AppState,
    pace: &mut pacing::Worker,
    outcome: &DrainOutcome,
    notifier: &Notifier,
) -> Pacing {
    let summary = DrainSummary {
        delivered: outcome.delivered,
        failed: outcome.failed,
    };
    let stalled = summary.failed > 0 && summary.delivered == 0;
    let next = pace.after(summary, stalled && journal_is_filling(state));

    if next.report_backlog {
        notifier
            .send(Event {
                op: ops::JOURNAL_BACKLOG,
                ok: false,
                version: env!("CARGO_PKG_VERSION").to_string(),
                error: Some(
                    "deliveries keep failing and the journal is over half its cap; \
                     once it fills, ingest starts refusing events and the sources' \
                     own retries will eventually give up"
                        .to_string(),
                ),
            })
            .await;
    }
    if next.recovered {
        tracing::info!("deliveries recovered; returning to the normal poll interval");
    }
    if stalled {
        tracing::warn!(
            consecutive_failures = pace.consecutive_failures(),
            wait_seconds = next.wait_secs,
            "deliveries are failing; backing off before the next attempt"
        );
    }
    next
}

/// Publishes the next pass to the watch and re-arms the ticker when the
/// wanted wait changed.
async fn reschedule(
    state: &AppState,
    ticker: &mut Interval,
    next: &Pacing,
    consecutive_failures: usize,
) {
    state
        .watch
        .scheduled((state.now_unix)() + next.wait_secs, consecutive_failures);
    let wanted = Duration::from_secs(next.wait_secs);
    if ticker.period() != wanted {
        *ticker = tokio::time::interval(wanted);
        ticker.tick().await; // the first tick fires immediately
    }
}

/// Removes set-aside entries past their 30 days (Kenny, form
/// aside-retention). Cheap on an indexed table, so every pass.
async fn expire_set_aside(state: &AppState) {
    if let Err(e) = state.journal.drop_expired((state.now_unix)() as i64).await {
        tracing::warn!(error = %e, "could not remove expired set-aside events");
    }
}
