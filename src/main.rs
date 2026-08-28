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
use almanac::shell::datadir::DataDirLock;
use almanac::shell::ingest::AppState;
use almanac::shell::journal::{DEFAULT_MAX_BYTES, Journal};
use almanac::shell::token_store::TokenStore;
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;

const DEFAULT_PROFILES_DIR: &str = "profiles";
const DEFAULT_JOURNAL_PATH: &str = "data/journal.jsonl";
const DEFAULT_TOKEN_STORE: &str = "data/tokens.json";
const DEFAULT_DATA_DIR: &str = "data";

/// Backoff between startup authentication attempts, in seconds; the
/// last value repeats. Never gives up: a wedged unit that nobody
/// restarts is worse than one that keeps trying quietly (AR21).
const STARTUP_RETRY_WAITS: [u64; 5] = [2, 5, 15, 60, 300];
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

    // AR21: distinguish a broken key from an unreachable Google. A
    // malformed key never fixes itself, so exit. A transient failure —
    // which is exactly what a power cut produces, when the LXC starts
    // Almanac before the network settles — must not park the unit in
    // `failed` forever with nobody watching. Keep trying instead.
    let mut attempt = 0u32;
    loop {
        match tokens.token().await {
            Ok(_) => {
                tracing::info!("authenticated against Google");
                break;
            }
            Err(e) if !e.is_transient() => die(e),
            Err(e) => {
                attempt += 1;
                let wait =
                    STARTUP_RETRY_WAITS[(attempt as usize - 1).min(STARTUP_RETRY_WAITS.len() - 1)];
                tracing::warn!(
                    attempt,
                    wait_seconds = wait,
                    error = %e,
                    "could not reach Google yet; retrying — the service stays up and the journal \
                     accepts events meanwhile"
                );
                tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            }
        }
    }

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

    // AR22: take the data-directory lock before anything reads or
    // writes the journal, so a self-update handover cannot run two
    // workers over one journal.
    let data_dir = PathBuf::from(
        std::env::var("ALMANAC_DATA_DIR").unwrap_or_else(|_| DEFAULT_DATA_DIR.to_string()),
    );
    let _data_lock = match DataDirLock::acquire(&data_dir) {
        Ok(lock) => lock,
        Err(e) => die(e),
    };

    // The encrypted token store is the only authority on who may post
    // (AR17 as amended). It refuses to load without its key rather than
    // falling back to something unencrypted.
    let token_store = match TokenStore::load(PathBuf::from(
        std::env::var("ALMANAC_TOKEN_STORE").unwrap_or_else(|_| DEFAULT_TOKEN_STORE.to_string()),
    )) {
        Ok(store) => store,
        Err(e) => die(e),
    };

    let state = Arc::new(AppState::new(
        profiles,
        Journal::new(journal_path.clone(), DEFAULT_MAX_BYTES),
        GoogleCalendarClient::new(http, tokens),
        bootstrap_token_hash,
        token_store,
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
