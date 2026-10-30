//! One line an hour saying almanac is still here (M14).
//!
//! Almanac wrote nothing to its log for 48 hours and looked, from the
//! outside, exactly like a service that had died. It had not: nothing
//! was posting, and a hub with no traffic has nothing to say. But a
//! silent hub and a wedged one are indistinguishable, and the homelab
//! went looking for a fault that was not there.
//!
//! It used to have a heartbeat by accident. The self-updater logged
//! `checked for a new release` every six hours, deliberately whether or
//! not there was one — that line exists because a self-updater that has
//! silently stopped and one that is working look identical otherwise
//! (0.1.3, found on real hardware). When the homelab took over updates,
//! `ALMANAC_SELF_UPDATE=off` switched that task off and took the only
//! recurring sign of life with it. Correct on its own terms, and it
//! removed something nobody had noticed was load-bearing.
//!
//! So this is deliberate rather than incidental, and it is what standing
//! rule 23 asks for in the first place: *a periodic background task logs
//! one line per cycle, at the level someone actually reads, even when
//! there was nothing to do.*
//!
//! What it does NOT do is duplicate `/metrics`. The counters answer "how
//! many"; this answers "the process is running and its background work
//! is still ticking", which no counter and no health check can say — a
//! bound port and a live counter both survive a worker loop that has
//! stopped turning.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::shell::state::AppState;

/// How often, when nothing says otherwise. An hour: often enough that a
/// silent service is noticed within one, rare enough that a day of logs
/// stays readable — 24 lines, fewer than the quietest other service on
/// the fleet writes today.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(3600);

/// The knob (standing rule 27). `0` switches the heartbeat off, which
/// is a real answer for a machine whose logs are precious; anything
/// unparseable falls back to the default rather than to silence,
/// because a typo must not quietly disable the thing that reports
/// silence.
pub const INTERVAL_ENV: &str = "ALMANAC_HEARTBEAT_INTERVAL_SECS";

/// Reads the interval from the environment. `None` means "do not run".
pub fn interval_from(get: impl Fn(&str) -> Option<String>) -> Option<Duration> {
    match get(INTERVAL_ENV) {
        None => Some(DEFAULT_INTERVAL),
        Some(raw) => match raw.trim().parse::<u64>() {
            Ok(0) => None,
            Ok(secs) => Some(Duration::from_secs(secs)),
            Err(_) => Some(DEFAULT_INTERVAL),
        },
    }
}

/// What the `pending` field says when the journal could not be read.
const PENDING_UNREADABLE: i64 = -1;

/// The journal depth for the heartbeat. Read here rather than carried: it
/// is the one number that says whether the worker is keeping up, and it is
/// I/O. An unreadable journal is reported as unreadable, not as zero:
/// "nothing is waiting" and "I cannot tell you what is waiting" are
/// opposite kinds of news, and the metrics surface already refuses to
/// conflate them.
fn pending_depth(state: &AppState) -> i64 {
    match state.journal.pending() {
        Ok(entries) => entries.len() as i64,
        Err(e) => {
            tracing::warn!(error = %e, "the heartbeat could not read the journal");
            PENDING_UNREADABLE
        }
    }
}

/// Writes the one "alive" line.
fn log_alive(state: &AppState, started: Instant) {
    let counts = state.metrics.snapshot();
    let pending = pending_depth(state);
    tracing::info!(
        accepted = counts.accepted,
        delivered = counts.delivered,
        failed = counts.failed,
        dead = counts.dead,
        pending,
        sources = state.profiles().len(),
        uptime_secs = started.elapsed().as_secs(),
        "alive"
    );
}

/// Logs one line per interval until shutdown.
///
/// The first line comes after one interval, not at startup: the startup
/// lines already say almanac is alive, and a heartbeat immediately
/// after them would be noise at exactly the moment there is least
/// doubt.
pub async fn run(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>, every: Duration) {
    let started = Instant::now();
    let mut ticker = tokio::time::interval(every);
    // The first tick of a tokio interval fires immediately; consuming
    // it here is what makes "after one interval" true. The same detail,
    // missed in the updater, put its first check six hours out while
    // every unit test passed.
    ticker.tick().await;

    loop {
        tokio::select! {
            _ = ticker.tick() => log_alive(&state, started),
            changed = shutdown.changed() => {
                // A dropped sender means nobody can ask us to stop any
                // more; looping on would spin on the closed channel.
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
