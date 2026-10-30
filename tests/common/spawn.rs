//! Starting a `KitHub`: state, stubs and the chassis app around them.

use std::collections::HashMap;
use std::sync::Arc;

use almanac::core::profile::Profile;
use almanac::shell::auth::TokenManager;
use almanac::shell::calendar_client::GoogleCalendarClient;
use almanac::shell::journal::{DEFAULT_MAX_BYTES, Journal};
use almanac::shell::kit::{import_source_tokens, mount};
use almanac::shell::state::AppState;
use almanac::shell::testing::{CalendarStub, TokenStub, stub_credentials};
use axum::Router;
use chassis::AppSpec;
use chassis::testing::TestApp;

use super::{KEY, KitHub, TOKEN, profile_toml};

/// A hub with one profile (`home-assistant` on `primary`), no owner.
pub async fn spawn_kit() -> KitHub {
    spawn_kit_with(&["home-assistant"], None).await
}

/// A hub with the given profiles written to disk and an optional calendar
/// owner (K24 needs one to create calendars).
pub async fn spawn_kit_with(sources: &[&str], owner: Option<&str>) -> KitHub {
    let dir = tempfile::tempdir().expect("a temp dir");
    let profiles_dir = dir.path().join("profiles");
    std::fs::create_dir_all(&profiles_dir).unwrap();
    for source in sources {
        std::fs::write(
            profiles_dir.join(format!("{source}.toml")),
            profile_toml(source),
        )
        .unwrap();
    }
    spawn_kit_in(dir, owner).await
}

pub async fn spawn_kit_in(dir: tempfile::TempDir, owner: Option<&str>) -> KitHub {
    let profiles_dir = dir.path().join("profiles");
    std::fs::create_dir_all(&profiles_dir).unwrap();
    let calendar = CalendarStub::start().await;
    let tokens = TokenStub::start(3600).await;
    let http = reqwest::Client::new();
    let profiles: HashMap<String, Profile> = almanac::shell::profiles::load_map(&profiles_dir);
    let state = Arc::new(
        AppState::new(
            profiles,
            Journal::new(dir.path().join("journal.jsonl"), DEFAULT_MAX_BYTES),
            GoogleCalendarClient::with_base_url(
                http.clone(),
                TokenManager::new(http, stub_credentials(&tokens.url)),
                &calendar.base_url,
            ),
        )
        .with_profiles_dir(profiles_dir)
        .with_calendar_owner(owner.map(str::to_string)),
    );
    // The 3.x token store, when a test seeded one, is imported like the
    // binary does it on the first start of 4.0.0. `dir.path()` is the
    // state directory this function is about to hand the kit via
    // ALMANAC_STATE_DIR below, so there is nothing to read back out of
    // the App for it.
    import_source_tokens(dir.path(), &dir.path().join("tokens.json"), KEY)
        .await
        .expect("the import runs");

    let spec = AppSpec {
        name: "almanac",
        version: env!("CARGO_PKG_VERSION"),
        repository: Some("kennypassenier/almanac"),
        ..Default::default()
    };
    // ALMANAC_TOKEN and ALMANAC_SECRET_KEY are both fixed: chassis-rs
    // 2.0.0 fixed extra_env to win for TestApp::token()/login() too (CF-12),
    // so the override reaches both the app and the harness's own idea of
    // what to log in with.
    let state_dir = dir.path().display().to_string();
    let for_mount = Arc::clone(&state);
    let mut app = TestApp::start_with_env(
        spec,
        Router::new(),
        &[
            ("ALMANAC_STATE_DIR", state_dir.as_str()),
            ("ALMANAC_TOKEN", TOKEN),
            ("ALMANAC_SECRET_KEY", KEY),
        ],
        move |app| mount(app, for_mount),
    )
    .await;
    // CF-12's Almanac half, held as a check rather than a comment: the
    // harness logs in with the token the app was given, not one it made.
    assert_eq!(
        app.token(),
        TOKEN,
        "extra_env must win for TestApp::token()"
    );
    app.login().await;
    let addr = app.addr();

    KitHub {
        addr,
        state,
        calendar,
        dir,
        app,
        _tokens: tokens,
    }
}
