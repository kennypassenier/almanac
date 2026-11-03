//! The shared application state every handler, the worker and the
//! dashboard read: the loaded profiles, the journal, the Google client,
//! the clocks, the debug ring buffer and the counters.
//!
//! Split from the ingest module because it is not ingest's: the worker,
//! the heartbeat and the UI all hold an `AppState` and none of them
//! should have to reach through the HTTP handlers to get it. The
//! calendar-lag memory (K24) lives next door in [`calendars`].

mod calendars;

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::core::metrics::Metrics;
use crate::core::observability::{RingBuffer, RouteRecord};
use crate::core::profile::Profile;
use crate::shell::calendar_client::GoogleCalendarClient;
use crate::shell::delivery::KeyLocks;
use crate::shell::journal::Journal;

pub struct AppState {
    /// The loaded mapping profiles, swappable as a whole.
    ///
    /// Behind a lock since K21: the dashboard can add a profile while
    /// the service runs, and the alternative — telling Kenny to restart
    /// almanac after every change — is the friction that made him ask
    /// for the feature. Readers take the `Arc` and drop the guard
    /// immediately (`profiles()`), so a reload never waits on a request
    /// and no guard is ever held across an await.
    profiles: std::sync::RwLock<Arc<HashMap<String, Profile>>>,
    /// Where those profiles live, so a reload reads the same directory
    /// startup did (K20's resolved path, not a second guess at it).
    pub profiles_dir: std::path::PathBuf,
    /// Calendars deleted from the dashboard, until Google stops
    /// listing them (K24).
    ///
    /// Google's calendar list is eventually consistent: a calendar
    /// deleted a second ago can still come back in the very next list
    /// call, so the page that renders straight after the delete shows
    /// the thing that was just removed. Measured on 2026-09-03 —
    /// Kenny deleted one, it stayed on the page, and asking Google
    /// minutes later showed it genuinely gone.
    ///
    /// Almanac knows what it deleted, so it can say so rather than
    /// re-asking a source that has not caught up. Self-clearing: an id
    /// is dropped from here as soon as Google's own list no longer
    /// carries it, so this never grows and never outlives the truth.
    pub deleted_calendars: std::sync::Mutex<std::collections::HashSet<String>>,
    /// Calendars almanac made, by name, until Google lists them (K24).
    ///
    /// The mirror image of `deleted_calendars`, and it exists for the
    /// same lag. `ensure_calendar` refuses to make a second calendar
    /// with a name it can already see — but it looks in the very list
    /// that has not caught up, so a second click within those seconds
    /// finds nothing and creates a duplicate. Measured on CT 112: two
    /// `deleted a calendar` lines at 19:56 for one request.
    ///
    /// A duplicate calendar is close to invisible — events land,
    /// nothing errors, and half of them are on a calendar nobody has
    /// open — so the guard against it has to hold when Google's answer
    /// is still on its way.
    ///
    /// Self-clearing on the same rule as the tombstones: an entry is
    /// dropped as soon as Google's own list carries that id.
    pub created_calendars: std::sync::Mutex<std::collections::HashMap<String, String>>,
    /// Who a calendar created from the dashboard is shared with (K21).
    /// `None` disables creating calendars rather than creating one
    /// nobody can see: a calendar the service account makes is owned by
    /// the service account and invisible to every human until it is
    /// shared, and that mistake has already been made here twice.
    pub calendar_owner: Option<String>,
    /// The calendars Google last listed (id, name), for the kit's issue form
    /// and the Target-calendar column, which render synchronously (4.0.2).
    /// Primed at start, refreshed whenever the Calendars page lists them.
    pub calendar_names: std::sync::Mutex<Vec<(String, String)>>,
    pub journal: Journal,
    pub client: GoogleCalendarClient,
    pub locks: KeyLocks,
    /// Supplies the acceptance timestamp; injected rather than read
    /// ambiently so tests can pin it.
    pub now: Box<dyn Fn() -> String + Send + Sync>,
    /// Unix seconds, next to the formatted `now`, so nothing that needs
    /// arithmetic has to parse a timestamp back out of a string.
    pub now_unix: Box<dyn Fn() -> u64 + Send + Sync>,
    /// Recent delivery routes, for the K11 debug surface.
    pub routes: Mutex<RingBuffer<RouteRecord>>,
    /// M13 counters. Shared with the token manager, which is built
    /// before this state exists, so it is an `Arc` rather than owned.
    pub metrics: Arc<Metrics>,
    /// The kit's app database (`almanac.db`), home of the delivery history.
    /// Set once by `kit::mount`; the handle opens at start, so a state built
    /// by a test without the kit simply has none and the history is skipped.
    app_db: std::sync::OnceLock<chassis::storage::AppDb>,
    /// The worker's live view of the delivery chain, read by the
    /// Deliveries page. One per state, so tests do not share it.
    pub watch: crate::shell::worker::Watch,
}

/// How many recent routes to keep. Enough to debug what just happened;
/// small enough that it cannot grow into a memory problem on a
/// long-running process.
pub const HISTORY_CAPACITY: usize = 100;

/// Seconds since the Unix epoch. A clock set before 1970 cannot be
/// represented; that is logged (once per call, which is rare enough) and
/// answered with 0 rather than panicking inside a request.
fn unix_now() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(e) => {
            tracing::warn!(error = %e, "the system clock is before the Unix epoch; using 0");
            0
        }
    }
}

impl AppState {
    /// Assembles the shared state with real clocks and empty history.
    /// A constructor rather than a struct literal at each call site:
    /// the observability fields are the same everywhere, and every
    /// future field would otherwise have to be added to five places.
    pub fn new(
        profiles: HashMap<String, Profile>,
        journal: Journal,
        client: GoogleCalendarClient,
    ) -> Self {
        Self {
            profiles: std::sync::RwLock::new(Arc::new(profiles)),
            profiles_dir: std::path::PathBuf::from("profiles"),
            calendar_owner: None,
            calendar_names: std::sync::Mutex::new(Vec::new()),
            deleted_calendars: std::sync::Mutex::new(std::collections::HashSet::new()),
            created_calendars: std::sync::Mutex::new(std::collections::HashMap::new()),
            journal,
            client,
            locks: KeyLocks::new(),
            now: Box::new(|| chrono::Utc::now().to_rfc3339()),
            now_unix: Box::new(unix_now),
            routes: Mutex::new(RingBuffer::new(HISTORY_CAPACITY)),
            metrics: Arc::new(Metrics::default()),
            app_db: std::sync::OnceLock::new(),
            watch: crate::shell::worker::Watch::default(),
        }
    }

    /// Hands the state the kit's app database (once; a second call is ignored).
    pub fn set_app_db(&self, db: chassis::storage::AppDb) {
        self.journal.attach(db.clone());
        // A second attach would silently keep the first database, which
        // hides a wiring mistake; the first one stays, but say so.
        if self.app_db.set(db).is_err() {
            tracing::warn!("the app database was already attached; keeping the first one");
        }
    }

    /// The app database, or an error when the kit has not provided one.
    pub fn app_db(&self) -> Result<&chassis::storage::AppDb, chassis::Error> {
        self.app_db.get().ok_or_else(|| {
            chassis::Error::internal(
                "the app database is not attached",
                "it is attached when the service is mounted on the kit",
            )
        })
    }

    /// The current profile set. Cheap: an `Arc` clone under a read
    /// lock held for the length of that clone.
    pub fn profiles(&self) -> Arc<HashMap<String, Profile>> {
        self.profiles
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Replaces the whole set (K21). Whole-set rather than per-entry
    /// because `validate_unique_source_ids` is a property of the set —
    /// inserting one profile at a time cannot check it.
    pub fn set_profiles(&self, profiles: HashMap<String, Profile>) {
        *self
            .profiles
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Arc::new(profiles);
    }

    /// Where profiles are read from and written to. A builder rather
    /// than a constructor argument: only `main` knows the resolved
    /// path, and every test would otherwise have to invent one.
    pub fn with_profiles_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.profiles_dir = dir;
        self
    }

    /// Sets who a dashboard-created calendar is shared with (K21).
    pub fn with_calendar_owner(mut self, owner: Option<String>) -> Self {
        self.calendar_owner = owner;
        self
    }

    /// Shares one set of counters with the token manager, which is
    /// constructed earlier in startup. Without this the two would each
    /// count into their own instance and `almanac_token_refreshes_total`
    /// would sit at zero forever.
    pub fn with_metrics(mut self, metrics: Arc<Metrics>) -> Self {
        self.metrics = metrics;
        self
    }

    /// Same, but with both clocks pinned — for tests that assert on
    /// timestamps or drive expiry without waiting.
    #[cfg(test)]
    pub fn new_for_test(
        profiles: HashMap<String, Profile>,
        journal: Journal,
        client: GoogleCalendarClient,
    ) -> Self {
        Self {
            now: Box::new(|| "2026-08-28T09:00:00+00:00".to_string()),
            now_unix: Box::new(|| 1_787_000_000),
            ..Self::new(profiles, journal, client)
        }
    }
}
