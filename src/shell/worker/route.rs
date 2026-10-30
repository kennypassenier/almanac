//! Recording a delivery attempt on the debug ring buffer and in the
//! durable per-day history.

use crate::core::observability::{RouteOutcome, RouteRecord};
use crate::shell::state::AppState;

/// Records one delivery attempt on the K11 debug surface, so the
/// operator can see how an event was routed — including a failure with
/// its remedy, which is the case where looking at all is most likely.
pub async fn record_route(
    state: &AppState,
    entry: &crate::core::journal::Entry,
    result: &Result<crate::shell::delivery::Delivered, crate::core::error::AlmanacError>,
) {
    let outcome = match result {
        Ok(d) if d.created => RouteOutcome::Created {
            event_id: d.event_id.clone(),
        },
        Ok(d) => RouteOutcome::Updated {
            event_id: d.event_id.clone(),
        },
        Err(e) => RouteOutcome::Failed {
            message: e.to_string(),
            remedy: e.remedy().to_string(),
        },
    };

    state.routes.lock().await.push(RouteRecord {
        at: (state.now)(),
        source_id: entry.source_id.clone(),
        entry_id: entry.id.clone(),
        // Whatever the delivery actually deduplicated against. A
        // failure has no key to report, since it never got that far.
        upsert_key: match &result {
            Ok(d) => d.upsert_key.clone(),
            Err(_) => None,
        },
        outcome,
    });

    // The durable per-day record behind the History page.
    crate::shell::history_store::note(state, entry, result).await;
}
