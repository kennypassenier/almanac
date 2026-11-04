//! JSON routes for the status page's kp-themes components (4.3.0): the
//! Journal section's glance and recent deliveries, and the Sources
//! section's list and reload. The sections themselves are
//! `kit::JournalSection` and `kit::ProfilesSection`; `web/js/status.js`
//! fills them from [`page_data`] and polls them for the live refresh.

mod journal;
mod sources;

use std::sync::Arc;

use axum::Router;
use axum::routing::post;
use chassis::PageData;

use crate::shell::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // The JSON twin of `POST /sources/reload`, which answers with a
        // redirect to the Calendars page; a flash button needs a message.
        .route("/status/sources/reload", post(sources::reload))
}

/// `GET /status/data`: the Journal and Sources sections in one read
/// (chassis-rs 4.8.0), each part on its own, so a failing part never blanks
/// the other; `GET /status/data/journal` and `/status/data/sources` read one
/// part alone for the sections' own retries.
pub fn page_data(state: Arc<AppState>) -> PageData {
    let journal_state = Arc::clone(&state);
    PageData::new("/status/data")
        .part("journal", move || {
            journal::journal(Arc::clone(&journal_state))
        })
        .part("sources", move || sources::sources(Arc::clone(&state)))
}
