//! Pure business logic — mapping-profile transformation, upsert
//! decisions, validation (AR13). Zero ambient I/O: every dependency on
//! the outside world (HTTP, the filesystem, the clock) reaches this
//! module only through a trait or parameter injected by `shell`, never
//! a direct call to reqwest, axum, or `std::fs`.
//!
//! Enforced mechanically, not just by convention: `.claude/hooks/gates.sh`
//! and the CI `gates` job fail the build if this module imports an I/O
//! crate — see AR13 in `docs/ARCHITECTURE_DECISIONS.md`.
//!
pub mod auth;
pub mod calendar;
pub mod error;
pub mod html;
pub mod journal;
pub mod mapping;
pub mod observability;
pub mod pacing;
pub mod profile;
pub mod retry;
pub mod secrets;
pub mod token;
pub mod update;
pub mod upsert;
