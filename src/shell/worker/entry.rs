//! Settling one pending entry: deliver it, then decide what its outcome
//! means (delivered, left pending for a retry, or set aside for good).

use super::route::record_route;
use super::watch::{DeliveredWatch, FailureKind, entry_title};
use super::{PERMANENT_FAILURES_BEFORE_DEAD, PermanentFailures};
use crate::core::error::AlmanacError;
use crate::core::journal::Entry;
use crate::core::profile::Profile;
use crate::shell::delivery::{Delivered, deliver};
use crate::shell::notify::{Event, Notifier, ops};
use crate::shell::state::AppState;

/// How one entry ended up in this pass.
pub(super) enum Settled {
    Delivered,
    /// Cancelled by a delete while it waited: nothing to write, no failure.
    Cancelled,
    /// `transient`: Google may take it later, so the rest of the pass waits.
    Failed {
        transient: bool,
    },
}

/// Delivers `entry` once and applies the consequences of the outcome.
pub(super) async fn settle_delivery(
    state: &AppState,
    entry: &Entry,
    profile: &Profile,
    permanent: &mut PermanentFailures,
    notifier: &Notifier,
) -> Settled {
    state.watch.deliver_started(&entry.id);
    let result = deliver(entry, profile, &state.client, &state.locks).await;
    state.watch.deliver_finished();
    record_route(state, entry, &result).await;

    match result {
        Ok(delivered) => {
            on_delivered(state, entry, profile, &delivered).await;
            Settled::Delivered
        }
        Err(e) if crate::shell::delivery::is_cancelled(&e) => Settled::Cancelled,
        Err(e) if e.is_transient() => {
            permanent.remove(&entry.id);
            state
                .watch
                .record_failure(&entry.id, FailureKind::of(&e), 0, (state.now_unix)());
            tracing::warn!(
                entry_id = %entry.id, error = %e, remedy = %e.remedy(),
                "delivery failed; entry stays pending for the next pass"
            );
            Settled::Failed { transient: true }
        }
        Err(e) => {
            on_permanent_failure(state, entry, &e, permanent, notifier).await;
            Settled::Failed { transient: false }
        }
    }
}

/// Marks the entry done and tells the watch. A failure to mark is only
/// a warning: the event is on the calendar and replay converges.
async fn on_delivered(state: &AppState, entry: &Entry, profile: &Profile, result: &Delivered) {
    if let Err(e) = state.journal.mark_done(&entry.id).await {
        tracing::warn!(
            entry_id = %entry.id, error = %e,
            "delivered but failed to mark done; replay will converge"
        );
    }
    // An older version still waiting would overwrite this one next pass.
    match state.journal.supersede_older(entry).await {
        Ok(0) => {}
        Ok(n) => {
            tracing::info!(entry_id = %entry.id, superseded = n, "closed older versions of this event")
        }
        Err(e) => {
            tracing::warn!(entry_id = %entry.id, error = %e, "could not close older versions")
        }
    }
    tracing::info!(
        entry_id = %entry.id,
        event_id = %result.event_id,
        created = result.created,
        "delivered"
    );
    state.watch.record_delivered(DeliveredWatch {
        entry_id: entry.id.clone(),
        source_id: entry.source_id.clone(),
        title: entry_title(entry, Some(profile)),
        at: (state.now_unix)(),
        created: result.created,
    });
}

/// A permanent failure will not fix itself by being retried. Count it,
/// and after a few passes set the entry aside so one unmappable payload
/// does not hold the whole hub in its slowest backoff forever (T1).
async fn on_permanent_failure(
    state: &AppState,
    entry: &Entry,
    e: &AlmanacError,
    permanent: &mut PermanentFailures,
    notifier: &Notifier,
) {
    let seen = permanent.entry(entry.id.clone()).or_insert(0);
    *seen += 1;
    let seen = *seen;
    state
        .watch
        .record_failure(&entry.id, FailureKind::of(e), seen, (state.now_unix)());

    if seen >= PERMANENT_FAILURES_BEFORE_DEAD {
        set_aside(state, entry, e, permanent, notifier).await;
    } else {
        tracing::warn!(
            entry_id = %entry.id, error = %e, remedy = %e.remedy(),
            permanent_failures = seen,
            "delivery failed permanently; entry stays pending for now"
        );
    }
}

/// Moves the entry to the dead list with its reason and raises the
/// notification. If the journal write fails the entry simply stays
/// pending and the next pass tries again.
async fn set_aside(
    state: &AppState,
    entry: &Entry,
    e: &AlmanacError,
    permanent: &mut PermanentFailures,
    notifier: &Notifier,
) {
    // The error's own text already ends in " — <remedy>"; adding the remedy again doubled it.
    let reason = e.to_string();
    if let Err(write_error) = state
        .journal
        .mark_dead(&entry.id, &reason, &(state.now)())
        .await
    {
        tracing::error!(
            entry_id = %entry.id, error = %write_error,
            "could not set the entry aside; it stays pending"
        );
        return;
    }

    permanent.remove(&entry.id);
    state.watch.record_set_aside(&entry.id);
    state.metrics.dead(1);
    tracing::error!(
        entry_id = %entry.id, source_id = %entry.source_id, reason = %reason,
        "set aside as undeliverable after {PERMANENT_FAILURES_BEFORE_DEAD} \
         permanent failures; it is kept in the journal and shown on the \
         debug surface, but no longer retried"
    );
    notifier
        .send(Event {
            op: ops::ENTRY_SET_ASIDE,
            ok: false,
            version: env!("CARGO_PKG_VERSION").to_string(),
            error: Some(format!(
                "an event from {} can never be delivered and was set aside: {reason}",
                entry.source_id
            )),
        })
        .await;
}
