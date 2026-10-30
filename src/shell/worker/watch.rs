//! The worker's live view of the delivery chain, read by the Deliveries
//! page: which stage last failed, which entries are stuck and how often,
//! what was delivered lately, when the next pass runs.

/// Which part of the chain a delivery failure came from, for the
/// Deliveries page: the stage it lights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The worker could not get a Google token.
    Token,
    /// Google answered with a refusal, or could not be reached.
    Google,
    /// Almanac could not build the event or find its profile.
    Local,
}

impl FailureKind {
    pub(super) fn of(error: &crate::core::error::AlmanacError) -> Self {
        match error {
            crate::core::error::AlmanacError::Auth { .. } => Self::Token,
            crate::core::error::AlmanacError::GoogleApi { .. } => Self::Google,
            _ => Self::Local,
        }
    }
}

/// How many failed tries are remembered per entry, so a long outage
/// cannot grow the watch without bound.
const TRIES_KEPT: usize = 20;
/// How many recent deliveries the Google Calendar stage lists.
const DELIVERED_KEPT: usize = 50;

/// What the worker has done with one waiting entry.
#[derive(Debug, Clone)]
pub struct EntryWatch {
    /// Unix seconds of each failed try, oldest first.
    pub failed_at: Vec<u64>,
    /// Consecutive permanent refusals; the entry is set aside at
    /// `PERMANENT_FAILURES_BEFORE_DEAD`.
    pub strikes: usize,
    pub last_kind: FailureKind,
}

/// A delivery the worker completed, for the Google Calendar stage.
#[derive(Debug, Clone)]
pub struct DeliveredWatch {
    pub entry_id: String,
    pub source_id: String,
    pub title: String,
    pub at: u64,
    pub created: bool,
}

/// A copy of the watch at one moment.
#[derive(Debug, Clone, Default)]
pub struct WatchSnapshot {
    /// Unix seconds of the next pass, once the loop has scheduled one.
    pub next_pass: Option<u64>,
    /// Stalled passes in a row (the backoff step); 0 when healthy.
    pub consecutive_failures: usize,
    /// When the current run of stalled passes began.
    pub outage_since: Option<u64>,
    /// What the latest pass failed on, `None` after a clean pass.
    pub last_failure: Option<FailureKind>,
    pub entries: std::collections::HashMap<String, EntryWatch>,
    /// The entry being delivered right now.
    pub in_flight: Option<String>,
    /// Newest first.
    pub delivered: Vec<DeliveredWatch>,
}

/// The worker's live state, read by the Deliveries page. Written only
/// by the loop below; in memory like the strike counts it mirrors, so a
/// restart starts it empty. A process-wide value because exactly one
/// worker runs.
pub struct Watch {
    inner: std::sync::Mutex<WatchSnapshot>,
    /// The failure seen during the pass in progress; it becomes
    /// `last_failure` when the pass ends, so a reader never sees a
    /// healthy chain flicker mid-pass.
    pass_failure: std::sync::Mutex<Option<FailureKind>>,
}

static WATCH: std::sync::LazyLock<Watch> = std::sync::LazyLock::new(|| Watch {
    inner: std::sync::Mutex::new(WatchSnapshot::default()),
    pass_failure: std::sync::Mutex::new(None),
});

/// The worker's live state.
pub fn watch() -> &'static Watch {
    &WATCH
}

impl Watch {
    fn with<T>(&self, f: impl FnOnce(&mut WatchSnapshot) -> T) -> T {
        f(&mut self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()))
    }

    pub(super) fn note_failure(&self, kind: FailureKind) {
        *self
            .pass_failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(kind);
    }

    /// The current state, copied out.
    pub fn snapshot(&self) -> WatchSnapshot {
        self.with(|w| w.clone())
    }

    pub(super) fn begin_pass(&self, waiting: &[String]) {
        self.with(|w| w.entries.retain(|id, _| waiting.contains(id)));
    }

    pub(super) fn deliver_started(&self, id: &str) {
        self.with(|w| w.in_flight = Some(id.to_string()));
    }

    pub(super) fn deliver_finished(&self) {
        self.with(|w| w.in_flight = None);
    }

    pub(super) fn record_failure(&self, id: &str, kind: FailureKind, strikes: usize, at: u64) {
        self.note_failure(kind);
        self.with(|w| {
            let entry = w.entries.entry(id.to_string()).or_insert(EntryWatch {
                failed_at: Vec::new(),
                strikes: 0,
                last_kind: kind,
            });
            entry.failed_at.push(at);
            if entry.failed_at.len() > TRIES_KEPT {
                entry.failed_at.remove(0);
            }
            entry.strikes = strikes;
            entry.last_kind = kind;
        });
    }

    pub(super) fn record_delivered(&self, delivered: DeliveredWatch) {
        self.with(|w| {
            w.entries.remove(&delivered.entry_id);
            w.delivered.insert(0, delivered);
            w.delivered.truncate(DELIVERED_KEPT);
        });
    }

    pub(super) fn record_set_aside(&self, id: &str) {
        self.with(|w| {
            w.entries.remove(id);
        });
    }

    /// Closes a pass: what it failed on becomes the lit stage, and a pass
    /// that delivered something or failed nothing ends the outage.
    pub(super) fn end_pass(&self, at: u64, delivered: usize, failed: usize) {
        let failure = self
            .pass_failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        self.with(|w| {
            w.last_failure = if failed > 0 { failure } else { None };
            if failed > 0 && delivered == 0 {
                w.outage_since.get_or_insert(at);
            } else {
                w.outage_since = None;
            }
        });
    }

    /// Records when the next pass runs and how deep into the backoff the
    /// loop is.
    pub(super) fn scheduled(&self, next_pass: u64, consecutive_failures: usize) {
        self.with(|w| {
            w.next_pass = Some(next_pass);
            w.consecutive_failures = consecutive_failures;
        });
    }
}

/// The title an entry will have in the calendar, or a plain stand-in
/// when its payload cannot be read.
pub fn entry_title(
    entry: &crate::core::journal::Entry,
    profile: Option<&crate::core::profile::Profile>,
) -> String {
    profile
        .and_then(|p| {
            crate::core::mapping::map_payload(
                &entry.payload,
                p,
                &format!("profile {}", p.source_id),
            )
            .map_err(|e| {
                // The stand-in title is the right answer for the page,
                // but why the payload cannot be read is worth a trace.
                tracing::debug!(entry_id = %entry.id, error = %e, "no title: payload does not map");
            })
            .ok()
        })
        .map(|event| event.summary)
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| format!("Event from {}", entry.source_id))
}
