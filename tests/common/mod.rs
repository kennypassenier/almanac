//! The in-process kit harness: Almanac assembled exactly as the binary
//! assembles it — `almanac::shell::kit::mount` on a real `chassis::App` —
//! started on a free port with the kit's door in front, Google and its
//! token endpoint stubbed. Tests about the dashboard, the door and the
//! debug surfaces go through here.
//!
//! Wraps `chassis::testing::TestApp` (chassis-rs 1.8.0) since [meta]: the
//! kit now ships the harness this file used to hand-write — starting the
//! app, logging in, issuing a client, sending JSON, shutting down cleanly.
//! What stays Almanac's own: building `AppState` (the journal, the Google
//! stub, the profiles directory), seeding profile files and a 3.x token
//! store on disk before the app ever starts, and the calendar owner. The
//! public shape of `KitHub` is unchanged — every test file that calls it
//! still does.
//!
//! chassis-rs 2.0.0 (fix from CF-12): `extra_env` now wins for
//! `TestApp::token()`/`login()` too, not only for what the App itself
//! receives — so `ALMANAC_TOKEN` is a fixed constant again below, the way
//! it read before the 1.8.0 workaround.
#![allow(dead_code)]

mod hub;
mod spawn;

pub mod dashboard;
pub mod inproc;
pub mod process;

pub use hub::KitHub;
#[allow(unused_imports)]
pub use spawn::{spawn_kit, spawn_kit_in, spawn_kit_with};

pub const TOKEN: &str = "a-login-token-that-is-long-enough";
pub const KEY: &str = "abababababababababababababababababababababababababababababababab";

pub fn profile_toml(source_id: &str) -> String {
    format!(
        r#"
schema_version = 2
source_id = "{source_id}"
target_calendar_id = "primary"
"#
    )
}

pub async fn body_json(response: reqwest::Response) -> serde_json::Value {
    let text = response.text().await.expect("a body");
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("expected JSON, got {text:?}: {e}"))
}

pub fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}
