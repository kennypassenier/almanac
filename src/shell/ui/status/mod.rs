//! JSON routes for the status page's kp-themes components (4.3.0): the
//! Journal section's glance and recent deliveries, and the Sources
//! section's list and reload. The sections themselves are
//! `kit::JournalSection` and `kit::ProfilesSection`; `web/js/status.js`
//! fills them from these answers and polls them for the live refresh.

mod journal;
mod sources;

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};

use crate::shell::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/status/journal", get(journal::journal))
        .route("/status/sources", get(sources::sources))
        // The JSON twin of `POST /sources/reload`, which answers with a
        // redirect to the Calendars page; a flash button needs a message.
        .route("/status/sources/reload", post(sources::reload))
}
