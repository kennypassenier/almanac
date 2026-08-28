//! The background delivery loop (AR16). Drains the journal's pending
//! entries into Google and marks each done. Runs on startup — which is
//! what makes replay-after-a-crash automatic rather than a manual
//! recovery step — and then on an interval for everything the
//! asynchronous ingest path accepted.
//!
//! A delivery that fails is left pending deliberately: the next pass
//! retries it. That is the whole reason the journal exists, and it is
//! safe because upsert (K2/AR15) and idempotency keys (M7) make a
//! redelivery converge on the same event.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::core::observability::{RouteOutcome, RouteRecord};
use crate::shell::ingest::AppState;

/// How often to look for work the asynchronous ingest path accepted.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Compact once the log has grown past this many delivered records,
/// so a long-running process does not accumulate an unbounded file of
/// done-markers.
const COMPACT_AFTER_DELIVERIES: usize = 100;

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
        upsert_key: None,
        outcome,
    });
}

/// Delivers every currently-pending entry once. Returns how many were
/// delivered. Never returns an error: one entry's failure must not
/// stop the others, and a failed entry stays pending for the next
/// pass.
pub async fn drain_once(state: &AppState) -> usize {
    let pending = match state.journal.pending() {
        Ok(pending) => pending,
        Err(e) => {
            tracing::error!(error = %e, remedy = %e.remedy(), "cannot read the journal");
            return 0;
        }
    };

    if pending.is_empty() {
        return 0;
    }

    tracing::info!(count = pending.len(), "delivering pending journal entries");

    let mut delivered = 0;
    for entry in pending {
        let Some(profile) = state.profiles.get(&entry.source_id) else {
            // The profile that accepted this payload is gone. Leaving
            // it pending forever would silently wedge the journal, so
            // say so loudly on every pass rather than dropping it.
            tracing::error!(
                entry_id = %entry.id,
                source_id = %entry.source_id,
                "journal entry names a source with no profile — restore the profile or move the \
                 journal aside; this entry cannot be delivered and will be retried indefinitely"
            );
            continue;
        };

        let result =
            crate::shell::delivery::deliver(&entry, profile, &state.client, &state.locks).await;
        record_route(state, &entry, &result).await;

        match result {
            Ok(result) => {
                if let Err(e) = state.journal.mark_done(&entry.id).await {
                    tracing::warn!(
                        entry_id = %entry.id, error = %e,
                        "delivered but failed to mark done; replay will converge"
                    );
                }
                tracing::info!(
                    entry_id = %entry.id,
                    event_id = %result.event_id,
                    created = result.created,
                    "delivered"
                );
                delivered += 1;
            }
            Err(e) => {
                tracing::warn!(
                    entry_id = %entry.id, error = %e, remedy = %e.remedy(),
                    "delivery failed; entry stays pending for the next pass"
                );
            }
        }
    }

    delivered
}

/// Runs the loop until `shutdown` flips. On exit it drains once more,
/// so a graceful stop (M2) hands over a journal with as little
/// outstanding work as possible.
pub async fn run(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>) {
    // Startup replay: whatever a crash or power cut left behind goes
    // out before anything new is accepted for delivery.
    let replayed = drain_once(&state).await;
    if replayed > 0 {
        tracing::info!(count = replayed, "replayed entries left by a previous run");
    }

    let mut since_compaction = replayed;
    let mut ticker = tokio::time::interval(POLL_INTERVAL);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                since_compaction += drain_once(&state).await;
                if since_compaction >= COMPACT_AFTER_DELIVERIES {
                    match state.journal.compact().await {
                        Ok(kept) => {
                            tracing::info!(pending = kept, "compacted the journal");
                            since_compaction = 0;
                        }
                        Err(e) => tracing::warn!(error = %e, "journal compaction failed"),
                    }
                }
            }
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    tracing::info!("worker shutting down — draining once more");
                    drain_once(&state).await;
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::shell::calendar_client::GoogleCalendarClient;
    use crate::shell::journal::{DEFAULT_MAX_BYTES, Journal};

    fn state_with_empty_journal() -> (Arc<AppState>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "almanac-worker-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let http = reqwest::Client::new();
        let tokens = crate::shell::auth::TokenManager::new(
            http.clone(),
            crate::core::auth::ServiceAccountCredentials {
                client_email: "unused".to_string(),
                private_key: "unused".to_string(),
                token_url: "https://example.invalid/token".to_string(),
            },
        );

        let state = Arc::new(AppState::new_for_test(
            HashMap::new(),
            Journal::new(dir.join("journal.jsonl"), DEFAULT_MAX_BYTES),
            GoogleCalendarClient::new(http, tokens),
            None,
        ));

        (state, dir)
    }

    #[tokio::test]
    async fn the_worker_returns_promptly_when_shutdown_is_signalled() {
        // M2: without this the process would hang on the worker handle
        // after the HTTP server stopped, and systemd would eventually
        // SIGKILL it — losing exactly the graceful drain the shutdown
        // path exists to perform.
        let (state, dir) = state_with_empty_journal();
        let (tx, rx) = watch::channel(false);

        let worker = tokio::spawn(run(state, rx));
        tx.send(true).unwrap();

        let result = tokio::time::timeout(Duration::from_secs(5), worker).await;
        assert!(
            result.is_ok(),
            "the worker must return on the shutdown signal, not wait out its poll interval"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn draining_an_empty_journal_delivers_nothing_and_does_not_error() {
        let (state, dir) = state_with_empty_journal();
        assert_eq!(drain_once(&state).await, 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn an_entry_whose_profile_is_gone_is_left_pending_rather_than_dropped() {
        // Losing a payload because someone renamed a profile would be
        // exactly the silent data loss the journal exists to prevent.
        let (state, dir) = state_with_empty_journal();
        state
            .journal
            .accept(&crate::core::journal::Entry {
                id: "orphan".to_string(),
                source_id: "profile-that-no-longer-exists".to_string(),
                received_at: "2026-08-28T09:00:00+00:00".to_string(),
                payload: serde_json::json!({"title": "t"}),
                idempotency_key: None,
            })
            .await
            .unwrap();

        assert_eq!(drain_once(&state).await, 0, "nothing can be delivered");
        assert_eq!(
            state.journal.pending().unwrap().len(),
            1,
            "but the payload must still be there"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
