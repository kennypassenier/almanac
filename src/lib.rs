//! Almanac as a library — split out from `main.rs` so integration
//! tests under `tests/` (standing rule 9: E2E against real
//! dependencies) can exercise `shell`'s Google Calendar client and
//! auth directly, the same way `main` does.

pub mod core;
pub mod shell;
