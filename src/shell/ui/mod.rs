//! The JSON routes and pages behind Almanac's kp-themes 11.0.0 screens
//! (4.3.0), one module per page, merged into the dashboard router behind the
//! admin login. Each page's script lives in `web/js/<page>.js`.

use std::sync::Arc;

use axum::Router;

use crate::shell::ingest::AppState;

pub mod calendars;
pub mod deliveries;
pub mod history;
pub mod status;
pub mod try_source;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .merge(calendars::routes())
        .merge(status::routes())
        .merge(try_source::routes())
        .merge(deliveries::routes())
        .merge(history::routes())
}
