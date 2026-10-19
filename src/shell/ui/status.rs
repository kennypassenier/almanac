//! JSON routes for the status page's kp-themes components.

use std::sync::Arc;

use axum::Router;

use crate::shell::ingest::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
}
