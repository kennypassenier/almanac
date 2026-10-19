//! The Deliveries page (/deliveries) and its JSON routes.

use std::sync::Arc;

use axum::Extension;
use axum::Router;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chassis::Dashboard;
use serde_json::json;

use crate::shell::ingest::AppState;

const PAGE_HTML: &str = include_str!("../../../templates/deliveries.html");

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/deliveries", get(page))
}

async fn page(Extension(dash): Extension<Dashboard>) -> Response {
    match dash.render_project("/deliveries", PAGE_HTML, json!({})) {
        Ok(html) => html.into_response(),
        Err(e) => e.into_response(),
    }
}
