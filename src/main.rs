//! Almanac — event-to-calendar hub.
//!
//! Startup order matters: everything that can be checked without side
//! effects is checked before the listener binds, so a misconfigured
//! process fails immediately and visibly rather than accepting traffic
//! it cannot serve.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use almanac::core::paths::Paths;
use almanac::core::token::hash_token;
use almanac::shell;
use almanac::shell::admin::BOOTSTRAP_TOKEN_ENV;
use almanac::shell::auth::{TokenManager, load_credentials};
use almanac::shell::calendar_client::GoogleCalendarClient;
use almanac::shell::datadir::DataDirLock;
use almanac::shell::ingest::AppState;
use almanac::shell::journal::{DEFAULT_MAX_BYTES, Journal};
use almanac::shell::notify::Notifier;
use almanac::shell::token_store::TokenStore;
use almanac::shell::update::{self, Startup, Updater};
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;

/// How long a freshly-installed version has to stay up before the
/// update is confirmed and the previous binary stops being a fallback
/// (AR23). Long enough to cover a panic on the first request or a
/// worker that dies on its first drain; short enough that an ordinary
/// update is settled within a minute.
const HEALTH_CONFIRM_DELAY: std::time::Duration = std::time::Duration::from_secs(60);

/// Backoff between startup authentication attempts, in seconds; the
/// last value repeats. Never gives up: a wedged unit that nobody
/// restarts is worse than one that keeps trying quietly (AR21).
const STARTUP_RETRY_WAITS: [u64; 5] = [2, 5, 15, 60, 300];

/// Where the listener binds. Configurable because it had to be
/// hardcoded to test the graceful shutdown at all — and because a
/// fixed port means two instances cannot coexist even briefly, and
/// changing it needs a rebuild. The default is unchanged.
const DEFAULT_BIND_ADDRESS: &str = "0.0.0.0:8080";

fn bind_address() -> String {
    std::env::var("ALMANAC_BIND").unwrap_or_else(|_| DEFAULT_BIND_ADDRESS.to_string())
}

fn die(e: impl std::fmt::Display) -> ! {
    eprintln!("{e}");
    std::process::exit(1);
}

/// K20. Every path, from one place. Resolved on each call rather than
/// cached, because the callers are a handful of one-shot startup paths
/// and a stale copy of "where my state lives" is a worse bargain than
/// re-reading four environment variables.
fn paths() -> Paths {
    Paths::resolve(|key| std::env::var(key).ok())
}

fn profiles_dir() -> PathBuf {
    paths().profiles_dir
}

fn data_dir() -> PathBuf {
    paths().data_dir
}

fn token_store_path() -> PathBuf {
    paths().token_store
}

/// `--check` (AR22): prove this binary can start on this machine, then
/// exit, without claiming the port or the data-directory lock that the
/// running process still holds.
///
/// It deliberately checks everything that can differ between versions
/// on one machine — profiles, the secrets Latch injects, the key that
/// opens the token store — and deliberately touches no network, so the
/// answer is about this build and this configuration rather than about
/// whether Google happens to be reachable this second.
async fn check_mode() -> ! {
    let version = env!("CARGO_PKG_VERSION");

    // Reported, not fatal: --check answers "can this binary run here",
    // and an unusable profile is a source that will not be served, not
    // a reason the process cannot start. It says so and carries on.
    for unusable in shell::profiles::load_all(&profiles_dir()).unusable {
        eprintln!(
            "--check: profile not usable: {} — {}",
            unusable.path.display(),
            unusable.reason
        );
    }
    if let Err(e) = load_credentials() {
        die(format!("--check failed: {e}\n  remedy: {}", e.remedy()));
    }
    match TokenStore::load(token_store_path()) {
        Ok(store) => {
            if let Err(e) = store.verify_key_opens_store().await {
                die(format!("--check failed: {e}\n  remedy: {}", e.remedy()));
            }
        }
        Err(e) => die(format!("--check failed: {e}\n  remedy: {}", e.remedy())),
    }

    println!("almanac {version} --check: ok");
    std::process::exit(0);
}

/// K19 — `almanac update`: one update, no restart, for a supervisor
/// that owns both.
///
/// Exits 0 when there was nothing to do AND when a new version was
/// installed, because neither is a failure and the caller decides what
/// happens next by looking at the binary. Exits 1 only when the attempt
/// itself failed, which is the homelab's signal to leave the service on
/// the version it is already running.
async fn supervised_update() {
    let http = reqwest::Client::new();
    let notifier = Notifier::from_env(http.clone());

    let updater = match Updater::for_command(http, notifier, data_dir()) {
        Ok(updater) => updater,
        Err(e) => {
            eprintln!("almanac update: {e}");
            eprintln!("  remedy: {}", e.remedy());
            std::process::exit(1)
        }
    };

    match updater.supervised().check_once().await {
        Ok(update::Outcome::UpToDate(version)) => {
            println!("almanac update: already on {version}");
            std::process::exit(0)
        }
        Ok(update::Outcome::Skipped(reason)) => {
            println!("almanac update: an update was available but not installed — {reason}");
            std::process::exit(0)
        }
        Ok(update::Outcome::Installed { from, to }) => {
            println!(
                "almanac update: installed {from} -> {to}; the binary changed, restart when ready"
            );
            std::process::exit(0)
        }
        Err(e) => {
            eprintln!("almanac update failed: {e}");
            eprintln!("  remedy: {}", e.remedy());
            std::process::exit(1)
        }
    }
}

/// `--version` / `-V`: print the compiled version and exit, touching
/// nothing else.
///
/// Every other special mode — `--check`, `update` — needs the full
/// production configuration, on purpose: they answer "can this binary
/// run here", which is a question about this machine. "What version is
/// this" is not, and answering it should not need `latch run`, a
/// working directory next to `profiles/`, or `ALMANAC_NOTIFY_WEBHOOK`
/// set. Before this existed, asking required starting the whole
/// process — the homelab session hit exactly that running the binary
/// by hand to sanity-check a deploy.
///
/// Answers about the file, not the process: under a supervised update
/// (R12b) `almanac update` replaces this binary before the homelab
/// restarts the unit, so for that window `--version` and `/healthz`
/// can correctly disagree — `--version` already sees the new file,
/// `/healthz` still reports the version actually serving traffic until
/// the restart happens. Verifying what is running wants `/healthz`;
/// this is for the file on disk.
fn version_mode() -> ! {
    println!("almanac {}", env!("CARGO_PKG_VERSION"));
    std::process::exit(0);
}

#[tokio::main]
async fn main() {
    if std::env::args().any(|arg| arg == "--version" || arg == "-V") {
        version_mode();
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    if std::env::args().any(|arg| arg == update::CHECK_ARG) {
        check_mode().await;
    }

    // K19. Before the data-directory lock, deliberately: this runs
    // while the service is up, and taking the lock would make it refuse
    // to start against its own running instance. Nothing here touches
    // the journal — it reads a release, verifies it, and replaces a
    // file on disk.
    if std::env::args().any(|arg| arg == update::UPDATE_ARG) {
        supervised_update().await;
    }

    let data_dir = data_dir();

    // AR22: take the data-directory lock before anything reads or
    // writes the journal, and before the startup work that can take
    // minutes — a self-update handover must not run two workers over
    // one journal, and the second process should say so at once
    // rather than after retrying Google for five minutes.
    let _data_lock = match DataDirLock::acquire(&data_dir) {
        Ok(lock) => lock,
        Err(e) => die(e),
    };

    let http = reqwest::Client::new();
    let notifier = Notifier::from_env(http.clone());

    // AR23: settle a pending self-update before doing anything else.
    // This start counts as the new version's attempt, and it is
    // recorded now rather than later, so a version that dies in the
    // next few lines is still reverted on the following start.
    match update::handle_pending_update(
        &data_dir,
        &std::env::current_exe().unwrap_or_else(|_| PathBuf::from("almanac")),
        &notifier,
    )
    .await
    {
        Startup::Reverted => {
            tracing::error!(
                "the previous binary is back in place; exiting so the supervisor starts it"
            );
            std::process::exit(0);
        }
        Startup::OnProbation | Startup::Continue => {}
    }

    // Profiles first: a typo in a profile should stop the process
    // before it has authenticated against anything (M4).
    let profiles_dir = profiles_dir();
    let loaded = shell::profiles::load_all(&profiles_dir);

    // Reported one line per file, at error level: an unusable profile
    // is a source that is NOT being served, and the quietest possible
    // failure would be a count that simply came out lower than
    // expected. They are also listed on the dashboard, where they can
    // be deleted — which is why refusing to start over one would be
    // exactly the wrong move: the way to fix it is the thing that
    // would not have started.
    for unusable in &loaded.unusable {
        tracing::error!(
            path = %unusable.path.display(),
            reason = %unusable.reason,
            "a profile could not be used; this source is not being served"
        );
    }

    let profiles = loaded.profiles;
    if profiles.is_empty() {
        // Serving nothing is a legitimate state, not a failure: it is
        // what a fresh machine looks like before anyone has added a
        // source, and the dashboard is how they add one.
        tracing::warn!(
            directory = %profiles_dir.display(),
            unusable = loaded.unusable.len(),
            "no usable mapping profiles — almanac is serving no sources; add one from /dashboard/sources"
        );
    } else {
        tracing::info!(
            count = profiles.len(),
            unusable = loaded.unusable.len(),
            sources = ?profiles.iter().map(|p| p.source_id.as_str()).collect::<Vec<_>>(),
            "loaded mapping profiles"
        );
    }
    let profiles: HashMap<String, _> = profiles
        .into_iter()
        .map(|p| (p.source_id.clone(), p))
        .collect();

    let credentials = match load_credentials() {
        Ok(credentials) => credentials,
        Err(e) => die(e),
    };

    // One set of counters for the whole process: the token manager is
    // built here, well before the shared state exists, and both have to
    // count into the same place (M13).
    let metrics = Arc::new(almanac::core::metrics::Metrics::default());
    let tokens = TokenManager::with_metrics(http.clone(), credentials, Arc::clone(&metrics));

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

    let journal_path = paths().journal;
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

    // The encrypted token store is the only authority on who may post
    // (AR17 as amended). It refuses to load without its key rather than
    // falling back to something unencrypted.
    let token_store = match TokenStore::load(token_store_path()) {
        Ok(store) => store,
        Err(e) => die(e),
    };

    // Prove the key opens the store before serving anything (L3): a
    // wrong key otherwise surfaces as every source getting a 401
    // against a store that looks intact.
    if let Err(e) = token_store.verify_key_opens_store().await {
        die(e);
    }

    // S2: a capture-only credential, so learning what an unknown
    // webhook sends never requires handing that system the token that
    // also logs into the dashboard.
    let capture_token_hash = match std::env::var(almanac::shell::admin::CAPTURE_TOKEN_ENV) {
        Ok(token) if !token.trim().is_empty() => Some(hash_token(token.trim())),
        _ => {
            tracing::info!(
                "{} is not set — posting a capture needs the bootstrap token, which is also the \
                 dashboard login. Set a capture token before pointing a third-party system at the \
                 capture endpoint.",
                almanac::shell::admin::CAPTURE_TOKEN_ENV
            );
            None
        }
    };

    let state = Arc::new(
        AppState::new(
            profiles,
            Journal::new(journal_path.clone(), DEFAULT_MAX_BYTES),
            GoogleCalendarClient::new(http.clone(), tokens),
            bootstrap_token_hash,
            token_store,
        )
        .with_capture_token(capture_token_hash)
        .with_profiles_dir(profiles_dir.clone())
        .with_calendar_owner(
            std::env::var("ALMANAC_CALENDAR_OWNER")
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty()),
        )
        .with_metrics(metrics),
    );

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
    let worker = tokio::spawn(shell::worker::run(
        Arc::clone(&state),
        shutdown_rx.clone(),
        notifier.clone(),
    ));

    // M14: one line per interval, so a silent almanac and a wedged one
    // are distinguishable. Started before the listener so the first
    // interval is measured from the same moment the rest of the process
    // begins, not from whenever binding happened to finish.
    match shell::heartbeat::interval_from(|key| std::env::var(key).ok()) {
        Some(every) => {
            tokio::spawn(shell::heartbeat::run(
                Arc::clone(&state),
                shutdown_rx.clone(),
                every,
            ));
        }
        None => tracing::info!(
            "{} is 0 — no heartbeat line will be written",
            shell::heartbeat::INTERVAL_ENV
        ),
    }

    let bind_address = bind_address();
    let listener = match tokio::net::TcpListener::bind(&bind_address).await {
        Ok(listener) => listener,
        Err(e) => die(format!(
            "failed to bind {bind_address}: {e} — is another process already using this port?"
        )),
    };
    tracing::info!(address = %bind_address, "almanac listening");

    // AR23: the listener is up, so a freshly-installed version has
    // done the one thing a broken one cannot. Confirm it after a
    // settling period rather than immediately, so a version that binds
    // and then panics on its first request is still reverted.
    let confirm_dir = data_dir.clone();
    let confirm_notifier = notifier.clone();
    tokio::spawn(async move {
        tokio::time::sleep(HEALTH_CONFIRM_DELAY).await;
        update::confirm_healthy(&confirm_dir, &confirm_notifier).await;
    });

    // M10: check for new releases in the background. Configuration
    // decides whether this does anything at all — no URL, no compiled
    // release key, or ALMANAC_SELF_UPDATE=off and it never runs.
    if let Some(updater) = Updater::from_env(http, notifier.clone(), data_dir.clone()) {
        tokio::spawn(shell::update::run(
            updater,
            Arc::clone(&state),
            shutdown_rx.clone(),
        ));
    }

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
