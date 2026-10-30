//! The kit hooks Almanac wires at start: update events forwarded to Home
//! Assistant, the calendar cache filled off the start path, and the
//! worker drained when the listener stops.
//!
//! AR25 ("never restart under an investigation") retired in 4.0.1 with
//! Almanac's own capture store: the kit's per-client captures (K13) are
//! in memory and lost on restart by the kit's own design, and nothing
//! durable is at stake between two ticks of the update loop.

use std::sync::Arc;

use almanac::shell::notify::Notifier;
use almanac::shell::state::AppState;
use chassis::App;
use tokio::sync::watch;

use super::deliver::WorkerSlot;

/// D-A1 (kit 1.3.0): the update events keep reaching Home Assistant in
/// Almanac's own vocabulary (K22 / AR23 / AR24). The kit still logs each
/// event; this hook runs alongside, on the updater's task, so the send
/// is spawned rather than awaited. `update.ok` and `update.held` are
/// routine and stay out of the phone.
pub fn forward_update_events(app: &mut App, notifier: Notifier) {
    app.on_update_event(move |event| {
        use almanac::shell::notify::{Event as HaEvent, ops};
        let (op, ok) = match event.kind {
            "update.installed" => (ops::UPDATE_APPLIED, true),
            "update.rolled_back" => (ops::UPDATE_REVERTED, false),
            // The kit's `failed` covers a refused signature or checksum as
            // well as an unreachable host; AR24's "three strikes before
            // notifying" is not reproduced here — the kit says it once.
            "update.failed" => (ops::UPDATE_UNVERIFIED, false),
            _ => return,
        };
        let ha = HaEvent {
            op,
            ok,
            version: event.version.clone(),
            error: (!ok).then(|| event.detail.clone()),
        };
        let notifier = notifier.clone();
        tokio::spawn(async move { notifier.send(ha).await });
    });
}

/// 4.0.2: the Sources page's calendar dropdown and column render
/// synchronously from a cache; fill it once now, the Calendars page
/// refreshes it on every visit. Off the start path: an unreachable
/// Google must not delay the listener (N1: the port is bound within
/// seconds, then retries happen).
pub fn prime_calendar_cache(state: Arc<AppState>) {
    tokio::spawn(async move {
        match state.client.list_calendars().await {
            Ok(calendars) => {
                let calendars =
                    state.without_deleted_calendars(state.with_created_calendars(calendars));
                state.remember_calendars(&calendars);
            }
            Err(e) => tracing::warn!(
                error = %e,
                "could not list the calendars at start; the Sources page shows ids until the Calendars page is opened"
            ),
        }
    });
}

/// M2: the listener has stopped accepting and in-flight requests have
/// finished. Tell the worker to drain what they journalled before the
/// process exits; the kit bounds this by its shutdown budget.
pub fn drain_worker_on_flush(app: &mut App, shutdown: watch::Sender<bool>, worker: WorkerSlot) {
    app.on_flush(move || {
        tracing::info!("http server stopped; draining the worker");
        if shutdown.send(true).is_err() {
            // Every receiver is gone: the worker and the heartbeat have
            // already ended, so there is nothing left to tell.
            tracing::debug!("no task was listening for the shutdown signal");
        }
        let handle = worker.lock().expect("worker slot").take();
        if let Some(handle) = handle {
            tokio::runtime::Handle::current().block_on(async {
                if let Err(e) = handle.await {
                    tracing::warn!(error = %e, "worker did not shut down cleanly");
                }
            });
        }
        tracing::info!("almanac stopped");
    });
}
