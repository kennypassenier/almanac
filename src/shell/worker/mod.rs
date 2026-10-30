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
//!
//! Layout: `drain` walks one pass over the journal, `entry` settles one
//! entry's outcome, `run` is the paced loop around it, `watch` is the
//! live state the Deliveries page reads and `route` records attempts on
//! the debug surface and in the history.

mod drain;
mod entry;
mod route;
mod run;
#[cfg(test)]
pub(crate) mod tests;
mod watch;

pub use drain::{
    DrainOutcome, PermanentFailures, drain_once, drain_once_detailed, drain_once_tracking,
};
pub use route::record_route;
pub use run::run;
pub use watch::{DeliveredWatch, EntryWatch, FailureKind, Watch, WatchSnapshot, entry_title};

/// How many times an entry may fail *permanently* before it is set
/// aside (T1).
///
/// Not one: a permanent classification can be wrong at the edges — a
/// calendar whose permissions were being changed at that moment, a
/// 403 whose reason string Google has not documented. Three passes
/// spread over the backoff ladder is long enough that a genuinely
/// transient misclassification has recovered, and short enough that a
/// dead payload does not hold the hub back for hours.
const PERMANENT_FAILURES_BEFORE_DEAD: usize = 3;
