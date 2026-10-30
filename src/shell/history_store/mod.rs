//! The database side of the per-day delivery history (`core::history`).
//!
//! The history is the `deliveries` table of the kit's app database
//! (`<state_dir>/almanac.db`, chassis-rs 3.11.0): one row per final outcome
//! of a journal entry. The kit's Storage page lists it as "Delivery log"
//! (size, what is stored, clean up) and prunes the rows older than
//! [`history::MAX_DAYS`] days on its own. Before 4.4.0 this was `history.json`
//! beside the journal; [`import_from`] moves that file in once.
//!
//! * [`spec`] — the schema, the app database spec and the storage kind.
//! * [`import`] — the one-time `history.json` import.
//! * [`rows`] — the shared SQL, the insert and the row reader.
//! * [`record`] — the hook the worker calls after a delivery attempt.
//! * [`queries`] — the reads behind the History page and the agenda.

mod import;
mod queries;
mod record;
mod rows;
mod spec;

pub use import::import_from;
pub use queries::{Partial, day_counts, day_rows, latest_by_key};
pub use record::note;
pub use rows::insert;
pub use spec::{app_db_spec, kind, migrations};

// The tests reach everything through `super::*`.
#[cfg(test)]
use crate::core::history::{
    self, Book, DeliveryRecord, DeliveryResult, MAX_DELIVERIES_PER_DAY, Row,
};
#[cfg(test)]
use chassis::storage::AppDbSpec;
#[cfg(test)]
use import::OLD_FILE;
#[cfg(test)]
use std::path::Path;

#[cfg(test)]
mod tests;
