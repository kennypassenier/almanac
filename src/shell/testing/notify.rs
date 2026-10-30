//! The Home Assistant webhook stand-in.

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::Value;
use tokio::sync::Mutex;

/// A stand-in for Home Assistant's homelab-ops webhook, so a test can
/// assert what Almanac actually reported rather than only that it did
/// not crash.
pub struct NotifyStub {
    pub url: String,
    pub events: Arc<Mutex<Vec<Value>>>,
}

async fn record_event(
    State(events): State<Arc<Mutex<Vec<Value>>>>,
    axum::Json(body): axum::Json<Value>,
) -> StatusCode {
    events.lock().await.push(body);
    StatusCode::OK
}

impl NotifyStub {
    pub async fn start() -> Self {
        let events = Arc::new(Mutex::new(Vec::new()));

        let router = Router::new()
            .route("/webhook", axum::routing::post(record_event))
            .with_state(Arc::clone(&events));

        let address = super::serve_loopback(router).await;

        Self {
            url: format!("http://{address}/webhook"),
            events,
        }
    }

    pub async fn count(&self) -> usize {
        self.events.lock().await.len()
    }

    /// The `op` field of every event received, in order.
    pub async fn ops(&self) -> Vec<String> {
        self.events
            .lock()
            .await
            .iter()
            .filter_map(|e| e["op"].as_str().map(str::to_string))
            .collect()
    }
}
