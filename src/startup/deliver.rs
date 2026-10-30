//! What starts once the listener is up: the heartbeat, and delivery as
//! soon as Google accepts the credentials (AR21).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use almanac::shell;
use almanac::shell::auth::TokenManager;
use almanac::shell::notify::Notifier;
use almanac::shell::state::AppState;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// Where the worker's task handle lands once delivery starts, so the
/// flush hook can wait for it.
pub type WorkerSlot = Arc<Mutex<Option<JoinHandle<()>>>>;

/// Backoff between startup authentication attempts, in seconds; the
/// last value repeats. Never gives up: a wedged unit that nobody
/// restarts is worse than one that keeps trying quietly (AR21).
const STARTUP_RETRY_WAITS: [u64; 5] = [2, 5, 15, 60, 300];

/// M14: one line per interval, so a silent almanac and a wedged one are
/// distinguishable.
pub fn start_heartbeat(
    state: Arc<AppState>,
    shutdown: watch::Receiver<bool>,
    env: &BTreeMap<String, String>,
) {
    match shell::heartbeat::interval_from(|key| env.get(key).cloned()) {
        Some(every) => {
            tokio::spawn(shell::heartbeat::run(state, shutdown, every));
        }
        None => tracing::info!(
            "{} is 0 — no heartbeat line will be written",
            shell::heartbeat::INTERVAL_ENV
        ),
    }
}

/// The worker and what it needs once Google answers.
pub struct Delivery {
    pub tokens: Arc<TokenManager>,
    pub state: Arc<AppState>,
    pub shutdown: watch::Receiver<bool>,
    pub notifier: Notifier,
    pub worker: WorkerSlot,
}

/// AR21: distinguish a broken key from an unreachable Google. A
/// malformed key never fixes itself, so exit (the supervisor restarts
/// and the log says why). A transient failure — which is exactly what a
/// power cut produces — must not park the unit: keep trying, and start
/// delivering once it works. The listener is already up, so the journal
/// accepts events meanwhile.
pub fn authenticate_then_deliver(delivery: Delivery) {
    tokio::spawn(async move {
        authenticate(&delivery.tokens).await;
        let handle = tokio::spawn(shell::worker::run(
            delivery.state,
            delivery.shutdown,
            delivery.notifier,
        ));
        *delivery.worker.lock().expect("worker slot") = Some(handle);
    });
}

/// Returns once a token was issued; exits the process on a permanent
/// failure.
async fn authenticate(tokens: &TokenManager) {
    let mut attempt = 0usize;
    loop {
        match tokens.token().await {
            Ok(_) => {
                tracing::info!("authenticated against Google");
                return;
            }
            Err(e) if !e.is_transient() => {
                tracing::error!(
                    error = %e,
                    remedy = %e.remedy(),
                    "the Google credentials are unusable; exiting so the supervisor's log shows it"
                );
                eprintln!("{e}\n  remedy: {}", e.remedy());
                std::process::exit(1);
            }
            Err(e) => {
                let wait = STARTUP_RETRY_WAITS[attempt.min(STARTUP_RETRY_WAITS.len() - 1)];
                attempt += 1;
                tracing::warn!(
                    attempt,
                    wait_seconds = wait,
                    error = %e,
                    "could not reach Google yet; retrying — the service stays up and the journal \
                     accepts events meanwhile"
                );
                tokio::time::sleep(Duration::from_secs(wait)).await;
            }
        }
    }
}
