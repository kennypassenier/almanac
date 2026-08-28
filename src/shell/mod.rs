//! The I/O-performing shell — HTTP handlers, the Google Calendar
//! client, config/profile file loading (AR13). The only place allowed
//! to import reqwest or perform file/network I/O directly; whatever it
//! learns from the outside world reaches `core` through explicit
//! function calls and trait implementations, never the reverse.
//!
//! The ingest routes land in L3; the debug, capture and health
//! surfaces (K11, M11, M1) follow in L4.

pub mod auth;
pub mod calendar_client;
pub mod delivery;
pub mod ingest;
pub mod journal;
pub mod profiles;
pub mod worker;

use std::sync::Arc;

use axum::Router;

use crate::shell::ingest::AppState;

/// Builds the application's Axum router.
pub fn build_router(state: Arc<AppState>) -> Router {
    ingest::routes().with_state(state)
}
