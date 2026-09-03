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

use crate::shell::ingest::AppState;

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
            _ = ticker.tick() => {
                let counts = state.metrics.snapshot();
                // Read here rather than carried: the journal depth is
                // the one number that says whether the worker is
                // keeping up, and it is I/O.
                // An unreadable journal is reported as unreadable, not
                // as zero: "nothing is waiting" and "I cannot tell you
                // what is waiting" are opposite kinds of news, and the
                // metrics surface already refuses to conflate them.
                let pending: i64 = match state.journal.pending() {
                    Ok(entries) => entries.len() as i64,
                    Err(_) => -1,
                };
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
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Counts how many "alive" lines the task produced, by capturing
    /// the tracing output rather than by trusting the code's own idea
    /// of what it emitted.
    #[derive(Clone, Default)]
    struct Lines(Arc<std::sync::Mutex<Vec<String>>>);

    impl std::io::Write for Lines {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(buf).to_string());
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Lines {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_first_line_comes_after_one_interval_not_at_startup() {
        // The same shape of bug the updater had: a tokio interval fires
        // its first tick immediately, so a loop that does not consume
        // it logs at once — noise at exactly the moment the startup
        // lines have just said the same thing — and then drifts by one
        // period. Pinned here because it is invisible to every test
        // that calls the body directly.
        let lines = Lines::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(lines.clone())
            .with_ansi(false)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let (state, dir) = crate::shell::worker::tests::state_for_update_loop().await;
        let (tx, rx) = watch::channel(false);
        let every = Duration::from_secs(60);
        let task = tokio::spawn(run(state, rx, every));

        // Just short of the first interval: still nothing said.
        tokio::time::sleep(every - Duration::from_secs(1)).await;
        assert_eq!(
            lines
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|l| l.contains("alive"))
                .count(),
            0,
            "a heartbeat at startup would repeat what the startup lines just said"
        );

        // Past it: exactly one.
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(
            lines
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|l| l.contains("alive"))
                .count(),
            1
        );

        // And it keeps going rather than firing once.
        tokio::time::sleep(every).await;
        assert_eq!(
            lines
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|l| l.contains("alive"))
                .count(),
            2
        );

        tx.send(true).unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(
            task.is_finished(),
            "it must stop when shutdown is signalled"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test(start_paused = true)]
    async fn the_line_carries_the_numbers_someone_would_look_for() {
        let lines = Lines::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(lines.clone())
            .with_ansi(false)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let (state, dir) = crate::shell::worker::tests::state_for_update_loop().await;
        state.metrics.accepted();
        state.metrics.delivered(1);

        let (_tx, rx) = watch::channel(false);
        let every = Duration::from_secs(60);
        tokio::spawn(run(Arc::clone(&state), rx, every));
        tokio::time::sleep(every + Duration::from_secs(1)).await;

        let captured = lines.0.lock().unwrap().join("");
        assert!(captured.contains("alive"), "got: {captured}");
        for field in ["accepted=1", "delivered=1", "pending=", "uptime_secs="] {
            assert!(captured.contains(field), "{field} missing from: {captured}");
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unset_interval_is_an_hour() {
        assert_eq!(interval_from(|_| None), Some(DEFAULT_INTERVAL));
    }

    #[test]
    fn zero_switches_it_off() {
        // A real answer, not a mistake: some machines want their logs
        // untouched.
        assert_eq!(interval_from(|_| Some("0".to_string())), None);
    }

    #[test]
    fn a_number_is_taken_as_seconds() {
        assert_eq!(
            interval_from(|_| Some("90".to_string())),
            Some(Duration::from_secs(90))
        );
        assert_eq!(
            interval_from(|_| Some("  120 ".to_string())),
            Some(Duration::from_secs(120))
        );
    }

    #[test]
    fn a_typo_falls_back_to_the_default_rather_than_to_silence() {
        // The failure mode to avoid: a mistyped value quietly disabling
        // the very thing whose job is to report silence.
        assert_eq!(
            interval_from(|_| Some("hourly".to_string())),
            Some(DEFAULT_INTERVAL)
        );
        assert_eq!(
            interval_from(|_| Some(String::new())),
            Some(DEFAULT_INTERVAL)
        );
    }
}
