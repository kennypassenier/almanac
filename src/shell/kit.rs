//! Almanac's glue to chassis (3.0.0): what `/healthz` and `/metrics` say,
//! in the kit's shape, with Almanac's own words and metric names. The kit
//! answers the routes; these types answer the kit.

use std::sync::Arc;

use chassis::{ScrapeSource, Subsystem, SubsystemStatus};

use crate::shell::ingest::AppState;

/// `journal`: readable, with the number of undelivered entries. This is
/// deliberately NOT Google's reachability (M1): the health check going red
/// during an outage Almanac rides out via the journal would be a lie about
/// Almanac's own state. The only failing answer is a journal that cannot
/// be read — the one thing that stops deliveries for good.
pub struct JournalSubsystem(pub Arc<AppState>);

impl Subsystem for JournalSubsystem {
    fn name(&self) -> &str {
        "journal"
    }

    fn check(&self) -> SubsystemStatus {
        match self.0.journal.pending() {
            Ok(pending) if pending.is_empty() => SubsystemStatus::ok("readable; nothing pending"),
            Ok(pending) => SubsystemStatus::ok(format!(
                "readable; {} undelivered entr{} waiting for the worker",
                pending.len(),
                if pending.len() == 1 { "y" } else { "ies" }
            )),
            Err(e) => {
                SubsystemStatus::failing(format!("cannot be read: {e}. What now: {}", e.remedy()))
            }
        }
    }
}

/// The `almanac_*` series (M13), appended verbatim to the kit's `/metrics`
/// so every Grafana panel keeps its query. `almanac_build_info` is the
/// kit's since 3.0.0 (same name, same label).
pub struct AlmanacMetrics(pub Arc<AppState>);

impl ScrapeSource for AlmanacMetrics {
    fn scrape(&self) -> String {
        // The journal depth is read per scrape rather than tracked, because
        // a counter of "how many are pending" drifts from the file the
        // moment a replay, a compaction or a restart touches it.
        let pending = match self.0.journal.pending() {
            Ok(entries) => Some(entries.len() as u64),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "could not read the journal for a metrics scrape; reporting it as unreadable rather than as empty"
                );
                None
            }
        };
        self.0.metrics.render(pending, env!("CARGO_PKG_VERSION"))
    }
}
