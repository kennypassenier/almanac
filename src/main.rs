//! Almanac — event-to-calendar hub.
//!
//! Startup order matters: everything that can be checked without side
//! effects is checked before the listener binds, so a misconfigured
//! process fails immediately and visibly rather than accepting traffic
//! it cannot serve.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use almanac::core::token::hash_token;
use almanac::shell;
use almanac::shell::admin::BOOTSTRAP_TOKEN_ENV;
use almanac::shell::auth::{TokenManager, load_credentials};
use almanac::shell::calendar_client::GoogleCalendarClient;
use almanac::shell::ingest::AppState;
use almanac::shell::journal::{DEFAULT_MAX_BYTES, Journal};
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;

const DEFAULT_PROFILES_DIR: &str = "profiles";
const DEFAULT_JOURNAL_PATH: &str = "data/journal.jsonl";
const BIND_ADDRESS: &str = "0.0.0.0:8080";

fn die(e: impl std::fmt::Display) -> ! {
    eprintln!("{e}");
    std::process::exit(1);
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Profiles first: a typo in a profile should stop the process
    // before it has authenticated against anything (M4).
    let profiles_dir = PathBuf::from(
        std::env::var("ALMANAC_PROFILES_DIR").unwrap_or_else(|_| DEFAULT_PROFILES_DIR.to_string()),
    );
    let profiles = match shell::profiles::load_all(&profiles_dir) {
        Ok(profiles) => profiles,
        Err(e) => die(e),
    };
    if profiles.is_empty() {
        die(format!(
            "no mapping profiles found in {} — create at least one *.toml profile there, or set \
             ALMANAC_PROFILES_DIR to where they live",
            profiles_dir.display()
        ));
    }
    tracing::info!(
        count = profiles.len(),
        sources = ?profiles.iter().map(|p| p.source_id.as_str()).collect::<Vec<_>>(),
        "loaded mapping profiles"
    );
    let profiles: HashMap<String, _> = profiles
        .into_iter()
        .map(|p| (p.source_id.clone(), p))
        .collect();

    let credentials = match load_credentials() {
        Ok(credentials) => credentials,
        Err(e) => die(e),
    };

    let http = reqwest::Client::new();
    let tokens = TokenManager::new(http.clone(), credentials);

    // Fail fast: prove the credentials actually work now, rather than
    // starting a server that can only discover a broken key on its
    // first real request.
    if let Err(e) = tokens.token().await {
        die(e);
    }
    tracing::info!("authenticated against Google");

    let journal_path = PathBuf::from(
        std::env::var("ALMANAC_JOURNAL").unwrap_or_else(|_| DEFAULT_JOURNAL_PATH.to_string()),
    );
    // Absent means the admin surface (K11/M11/M9) refuses every
    // request rather than opening up; the ingest paths are unaffected.
    let bootstrap_token_hash = match std::env::var(BOOTSTRAP_TOKEN_ENV) {
        Ok(token) if !token.trim().is_empty() => Some(hash_token(token.trim())),
        _ => {
            tracing::warn!(
                "{BOOTSTRAP_TOKEN_ENV} is not set — the debug and capture surfaces will refuse \
                 every request. Set it via `latch run --` to use them."
            );
            None
        }
    };

    let state = Arc::new(AppState::new(
        profiles,
        Journal::new(journal_path.clone(), DEFAULT_MAX_BYTES),
        GoogleCalendarClient::new(http, tokens),
        bootstrap_token_hash,
    ));

    // Surface a damaged journal before binding rather than letting the
    // worker discover it a few seconds later.
    match state.journal.pending() {
        Ok(pending) if !pending.is_empty() => {
            tracing::info!(
                count = pending.len(),
                journal = %journal_path.display(),
                "journal holds undelivered entries from a previous run; they go out first"
            );
        }
        Ok(_) => {}
        Err(e) => die(e),
    }

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let worker = tokio::spawn(shell::worker::run(Arc::clone(&state), shutdown_rx));

    let listener = match tokio::net::TcpListener::bind(BIND_ADDRESS).await {
        Ok(listener) => listener,
        Err(e) => die(format!(
            "failed to bind {BIND_ADDRESS}: {e} — is another process already using this port?"
        )),
    };
    tracing::info!(address = BIND_ADDRESS, "almanac listening");

    let router = shell::build_router(state);
    let server = axum::serve(listener, router).with_graceful_shutdown(shutdown_signal());

    if let Err(e) = server.await {
        tracing::error!(error = %e, "server terminated unexpectedly");
    }

    // M2: the listener has stopped accepting and in-flight requests
    // have finished. Tell the worker to drain what they journalled
    // before the process exits.
    tracing::info!("http server stopped; draining the worker");
    let _ = shutdown_tx.send(true);
    if let Err(e) = worker.await {
        tracing::warn!(error = %e, "worker did not shut down cleanly");
    }
    tracing::info!("almanac stopped");
}

/// Resolves on SIGTERM (what systemd and `docker stop` send) or
/// SIGINT (Ctrl-C), so a restart or redeploy finishes in-flight
/// requests instead of cutting them off mid-write (M2).
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::warn!(error = %e, "failed to listen for Ctrl-C");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(e) => tracing::warn!(error = %e, "failed to listen for SIGTERM"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("received Ctrl-C"),
        _ = terminate => tracing::info!("received SIGTERM"),
    }
}
