//! Almanac — event-to-calendar hub.
//!
//! Milestone L0 (walking skeleton): project structure, the AR13
//! core/shell split with empty modules, CI green from day one.
//! Application behavior lands starting with milestone L1.

mod core;
mod shell;

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let router = shell::build_router();

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("failed to bind 0.0.0.0:8080 — is another process already using this port?");

    tracing::info!("almanac listening on 0.0.0.0:8080 (walking skeleton — no routes yet)");

    axum::serve(listener, router)
        .await
        .expect("server terminated unexpectedly");
}
