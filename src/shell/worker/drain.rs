//! One pass over the journal: deliver every pending entry once and
//! report how it went, so the loop can decide how long to wait.

use std::collections::HashMap;

use super::entry::{Settled, settle_delivery};
use super::watch::FailureKind;
use crate::core::journal::Entry;
use crate::shell::notify::Notifier;
use crate::shell::state::AppState;

/// What the pass reports when the journal itself cannot be read: that
/// counts as exactly one failure, so the loop backs off as for any other
/// stalled pass instead of treating an unreadable journal as idle.
const UNREADABLE_JOURNAL_FAILURES: usize = 1;

/// What one drain pass did, so the loop can decide how long to wait
/// before the next one.
pub struct DrainOutcome {
    pub delivered: usize,
    pub failed: usize,
}

/// Per-entry count of consecutive permanent failures, so an entry that
/// can never succeed is eventually set aside rather than retried
/// forever. In memory: a restart gives every entry a fresh chance,
/// which is the right default because a restart is also how a fixed
/// profile gets loaded.
pub type PermanentFailures = HashMap<String, usize>;

/// Delivers every currently-pending entry once. Returns how many were
/// delivered. Never returns an error: one entry's failure must not
/// stop the others, and a failed entry stays pending for the next
/// pass.
pub async fn drain_once(state: &AppState) -> usize {
    drain_once_detailed(state).await.delivered
}

/// As [`drain_once`], but also reports failures so the caller can back
/// off while an outage lasts.
pub async fn drain_once_detailed(state: &AppState) -> DrainOutcome {
    drain_once_tracking(state, &mut PermanentFailures::new(), &Notifier::disabled()).await
}

/// As [`drain_once_detailed`], but remembering which entries keep
/// failing permanently so they can be set aside (T1).
pub async fn drain_once_tracking(
    state: &AppState,
    permanent: &mut PermanentFailures,
    notifier: &Notifier,
) -> DrainOutcome {
    let pending = match state.journal.pending() {
        Ok(pending) => pending,
        Err(e) => {
            tracing::error!(error = %e, remedy = %e.remedy(), "cannot read the journal");
            state.watch.note_failure(FailureKind::Local);
            state
                .watch
                .end_pass((state.now_unix)(), 0, UNREADABLE_JOURNAL_FAILURES);
            return DrainOutcome {
                delivered: 0,
                failed: UNREADABLE_JOURNAL_FAILURES,
            };
        }
    };

    state
        .watch
        .begin_pass(&pending.iter().map(|e| e.id.clone()).collect::<Vec<_>>());

    if pending.is_empty() {
        state.watch.end_pass((state.now_unix)(), 0, 0);
        return DrainOutcome {
            delivered: 0,
            failed: 0,
        };
    }

    tracing::info!(count = pending.len(), "delivering pending journal entries");
    let outcome = deliver_all(state, pending, permanent, notifier).await;

    state.metrics.delivered(outcome.delivered as u64);
    state.metrics.failed(outcome.failed as u64);
    state
        .watch
        .end_pass((state.now_unix)(), outcome.delivered, outcome.failed);
    outcome
}

/// Walks the pending entries, one failure never stopping the rest.
async fn deliver_all(
    state: &AppState,
    pending: Vec<Entry>,
    permanent: &mut PermanentFailures,
    notifier: &Notifier,
) -> DrainOutcome {
    let mut outcome = DrainOutcome {
        delivered: 0,
        failed: 0,
    };
    let profiles = state.profiles();
    for entry in pending {
        let settled = match profiles.get(&entry.source_id) {
            Some(profile) => settle_delivery(state, &entry, profile, permanent, notifier).await,
            None => {
                report_missing_profile(state, &entry);
                Settled::Failed
            }
        };
        match settled {
            Settled::Delivered => outcome.delivered += 1,
            Settled::Failed => outcome.failed += 1,
        }
    }
    outcome
}

/// The profile that accepted this payload is gone. Leaving it pending
/// forever would silently wedge the journal, so say so loudly on every
/// pass rather than dropping it.
fn report_missing_profile(state: &AppState, entry: &Entry) {
    tracing::error!(
        entry_id = %entry.id,
        source_id = %entry.source_id,
        "journal entry names a source with no profile — restore the profile or move the \
         journal aside; this entry cannot be delivered and will be retried indefinitely"
    );
    state
        .watch
        .record_failure(&entry.id, FailureKind::Local, 0, (state.now_unix)());
}
