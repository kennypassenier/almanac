//! Almanac — event-to-calendar hub. On chassis since 3.0.0: the kit owns
//! the command line, the transport knobs, logging, `/healthz`, `/metrics`,
//! readiness, the graceful stop and signed self-update. This file assembles
//! Almanac on top of it.
//!
//! Startup order still matters: everything that can be checked without
//! side effects — profiles, the credentials Latch injects, the key that
//! opens the token store, the journal — is checked before the kit binds, so
//! a misconfigured process fails immediately and visibly. What needs the
//! network (authenticating against Google, AR21) runs AFTER the bind, so a
//! power cut that starts Almanac before the network settles never parks
//! the unit: it serves, the journal accepts events, and delivery starts
//! the moment Google answers.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use almanac::core::paths::{self, Paths};
use almanac::shell;
use almanac::shell::auth::{TokenManager, load_credentials};
use almanac::shell::datadir::DataDirLock;
use almanac::shell::kit::mount;
use almanac::shell::notify::Notifier;
use almanac::shell::profiles::Loaded;
use almanac::shell::state::AppState;
use axum::Router;
use chassis::{App, AppSpec, Control};
use startup::state::build_state;
use tokio::sync::watch;

mod startup;

use startup::deliver::{Delivery, WorkerSlot};
use startup::report::StartupReport;

/// What `--help` says beyond the kit's knobs: Almanac's own environment.
const HELP_EXTRA: &str = "Almanac's own environment (read next to the knobs above):
  ALMANAC_SECRET_KEY            64 hex chars; seals the token store (mandatory)
  ALMANAC_TOKEN                 the login token (kit knob): dashboard login and admin bearer; required
                                (was ALMANAC_BOOTSTRAP_TOKEN; ALMANAC_CAPTURE_TOKEN is gone — use a client token)
  ALMANAC_NOTIFY_WEBHOOK        Home Assistant webhook for almanac's own notifications
  ALMANAC_HEARTBEAT_INTERVAL_SECS  one heartbeat line per interval (default 3600, 0 = off)
  ALMANAC_CALENDAR_OWNER        who new calendars are shared with
  ALMANAC_PROFILES_DIR, _DATA_DIR, _JOURNAL, _TOKEN_STORE  2.x per-path overrides of the
                                state root (deprecated; derived from ALMANAC_STATE_DIR)
  CLIENT_EMAIL, PRIVATE_KEY, TOKEN_URI  the Google service account, injected by `latch run --`
  ALMANAC_BIND, ALMANAC_SELF_UPDATE, RUST_LOG  2.x names of ALMANAC_LISTEN, ALMANAC_UPDATE_MODE
                                and ALMANAC_LOG; still honoured, with a warning";

fn die(e: impl std::fmt::Display) -> ExitCode {
    eprintln!("{e}");
    ExitCode::FAILURE
}

#[tokio::main]
async fn main() -> ExitCode {
    let mut env: BTreeMap<String, String> = std::env::vars().collect();
    let mut warnings = startup::compat::compat(&mut env);

    let spec = AppSpec {
        name: "almanac",
        version: env!("CARGO_PKG_VERSION"),
        repository: Some("kennypassenier/almanac"),
        help_extra: Some(HELP_EXTRA),
        ..Default::default()
    };
    let args: Vec<String> = std::env::args().collect();
    // The routes need the state, which needs the paths the kit resolves —
    // so they are mounted below, behind the kit's door (client tokens) and
    // its admin login.
    let mut app = match App::from_args_with_env(spec, args, env.clone(), Router::new()) {
        Ok(app) => app,
        Err(e) => return die(e),
    };
    // fix-15 (chassis-rs 3.2.0): without this the kit's dashboard pages and
    // layout show the binary name ("almanac") as the brand; this is the
    // name the service already uses everywhere else (README, templates).
    app.brand_title("Almanac");
    if !app.needs_project_config() {
        // --version, --help, --healthcheck, --print-config, gen-secret,
        // update, rekey: the kit's alone, and they must work without
        // Latch, a token store or a profiles directory.
        return app.run().await;
    }
    if let Err(refusal) = startup::compat::check_retired_tokens(&env, &mut warnings) {
        return die(refusal);
    }
    preflight_then_start(app, env, warnings).await
}

/// The pre-flight `--check` and a start share (AR22): everything that
/// can differ between versions on one machine, and nothing that needs
/// the network or disturbs a running instance (no lock, no port).
async fn preflight_then_start(
    mut app: App,
    env: BTreeMap<String, String>,
    warnings: Vec<String>,
) -> ExitCode {
    let checking = matches!(app.control, Some(Control::Check));
    let state_dir = app
        .loaded
        .as_ref()
        .expect("a start or --check loads configuration")
        .state_dir
        .clone();
    // K20: every path from one place — the kit's root — with the 2.x
    // per-path overrides still honoured (deprecated, see --help).
    let paths = Paths::resolve(|key| {
        if key == paths::STATE_DIR_ENV {
            Some(state_dir.display().to_string())
        } else {
            env.get(key).cloned()
        }
    });
    let loaded = shell::profiles::load_all(&paths.profiles_dir);
    if checking {
        report_unusable_profiles(&loaded);
    }
    let credentials = match load_credentials() {
        Ok(credentials) => credentials,
        Err(e) => return die(format!("{e}\n  remedy: {}", e.remedy())),
    };
    if checking {
        app.on_check(|| {
            println!("almanac {} --check: ok", env!("CARGO_PKG_VERSION"));
            Ok(())
        });
        return app.run().await;
    }
    let start = Start {
        env,
        warnings,
        paths,
        state_dir,
        loaded,
        credentials,
    };
    start.run(app).await
}

/// Reported, not fatal: an unusable profile is a source that will not
/// be served, not a reason the process cannot start.
fn report_unusable_profiles(loaded: &Loaded) {
    for unusable in &loaded.unusable {
        eprintln!(
            "--check: profile not usable: {} — {}",
            unusable.path.display(),
            unusable.reason
        );
    }
}

/// What a real start has once the pre-flight passed.
struct Start {
    env: BTreeMap<String, String>,
    warnings: Vec<String>,
    paths: Paths,
    state_dir: std::path::PathBuf,
    loaded: Loaded,
    credentials: almanac::core::auth::ServiceAccountCredentials,
}

impl Start {
    async fn run(self, mut app: App) -> ExitCode {
        // AR22: take the data-directory lock before anything reads or
        // writes the journal — two processes over one journal is the one
        // thing a supervised update handover must never produce.
        let _data_lock = match DataDirLock::acquire(&self.paths.data_dir) {
            Ok(lock) => lock,
            Err(e) => return die(e),
        };
        let http = almanac::shell::calendar_client::http_client();
        let notifier = Notifier::from_env(http.clone());
        let Start {
            env,
            warnings,
            paths,
            state_dir,
            loaded,
            credentials,
        } = self;
        let profile_names: Vec<String> = loaded
            .profiles
            .iter()
            .map(|p| p.source_id.clone())
            .collect();
        // One set of counters for the whole process (M13).
        let metrics = Arc::new(almanac::core::metrics::Metrics::default());
        let tokens = TokenManager::with_metrics(http.clone(), credentials, Arc::clone(&metrics));
        let state = match build_state(&env, &paths, loaded.profiles, &http, &tokens, metrics) {
            Ok(state) => state,
            Err(e) => return die(e),
        };

        // Surface a damaged journal before binding rather than letting
        // the worker discover it a few seconds later.
        let pending = match state.journal.pending() {
            Ok(pending) => pending.len(),
            Err(e) => return die(e),
        };
        startup::hooks::forward_update_events(&mut app, notifier.clone());
        if let Err(e) = startup::state::import_tokens(&env, &state_dir, &paths).await {
            return die(e);
        }
        startup::hooks::prime_calendar_cache(Arc::clone(&state));
        mount(&mut app, Arc::clone(&state));

        let report = StartupReport {
            warnings,
            unusable: loaded.unusable,
            profile_names,
            profiles_dir: paths.profiles_dir.clone(),
            pending,
            journal_path: paths.journal.clone(),
        };
        wire_start(&mut app, report, state, tokens, notifier, env);
        app.run().await
    }
}

/// After the bind: log what the pre-flight found, start the heartbeat
/// and hand over to delivery once Google answers; on the way out, drain
/// the worker.
fn wire_start(
    app: &mut App,
    report: StartupReport,
    state: Arc<AppState>,
    tokens: Arc<TokenManager>,
    notifier: Notifier,
    env: BTreeMap<String, String>,
) {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let worker: WorkerSlot = Arc::new(Mutex::new(None));
    let delivery = Delivery {
        tokens,
        state: Arc::clone(&state),
        shutdown: shutdown_rx.clone(),
        notifier,
        worker: Arc::clone(&worker),
    };
    app.on_start(move || {
        report.log();
        startup::deliver::start_heartbeat(state, shutdown_rx, &env);
        startup::deliver::authenticate_then_deliver(delivery);
    });
    startup::hooks::drain_worker_on_flush(app, shutdown_tx, worker);
}
