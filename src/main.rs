//! Almanac — event-to-calendar hub.
//!
//! Milestone L1 (authenticated calendar core): the app authenticates
//! against Google at startup and holds a ready `GoogleCalendarClient`.
//! HTTP routes calling into it land starting with milestone L3.

use almanac::shell;
use almanac::shell::auth::{TokenManager, load_credentials};
use almanac::shell::calendar_client::GoogleCalendarClient;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let credentials = match load_credentials() {
        Ok(credentials) => credentials,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let http = reqwest::Client::new();
    let tokens = TokenManager::new(http.clone(), credentials);

    // Fail fast: prove the credentials actually work by fetching a
    // real token now, rather than silently starting a server that can
    // only discover a broken key on its first real request.
    if let Err(e) = tokens.token().await {
        eprintln!("{e}");
        std::process::exit(1);
    }

    let _calendar_client = GoogleCalendarClient::new(http, tokens);

    tracing::info!("almanac authenticated against Google — calendar client ready");

    let router = shell::build_router();

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("failed to bind 0.0.0.0:8080 — is another process already using this port?");

    tracing::info!("almanac listening on 0.0.0.0:8080 (routes land starting with milestone L3)");

    axum::serve(listener, router)
        .await
        .expect("server terminated unexpectedly");
}
