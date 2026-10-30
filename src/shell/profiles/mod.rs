//! Reads mapping profiles off disk (I/O — AR13) and hands them to
//! `core::profile` for parsing and validation. `core::profile::Profile`
//! is the actual public type; this module only ever does file I/O.
//!
//! * [`loading`] — startup and reload: every `*.toml`, never failing.
//! * [`saving`] — the dashboard adding a source (K21).
//! * [`deleting`] — the dashboard removing a source or a broken file.
//! * [`paths`] — the file naming shared by all three.

mod deleting;
mod loading;
mod paths;
mod saving;
mod types;

pub use deleting::{delete, delete_file};
pub use loading::{load_all, load_map};
pub use saving::save_new;
pub use types::{Loaded, Unusable};

// The tests reach everything through `super::*`.
#[cfg(test)]
use std::path::Path;

#[cfg(test)]
mod tests;
