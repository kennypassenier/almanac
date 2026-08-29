//! One-off setup tool: creates the real calendars the mapping profiles
//! target, under the service account that already holds the
//! credentials.
//!
//! This is K3 (multiple calendars, each source writing to its own) made
//! concrete. Kenny asked for exactly this during L1 — "je kan die toch
//! zelf een nieuwe agenda laten aanmaken? dat is toch één van de punten
//! van dit project?" — so the service account creates and owns them,
//! with no manual step in the Google Calendar UI.
//!
//! It does NOT share them. A calendar the service account owns is
//! invisible to everyone else until an ACL rule says otherwise, and the
//! address to share with is Kenny's to name, not this tool's to guess.
//! `share_calendars` is the second half.
//!
//! Idempotent: a calendar whose name already exists is reported and
//! left alone, so re-running this never produces duplicates.
//!
//!   latch run -- cargo run --example create_calendars

use almanac::shell::auth::{TokenManager, load_credentials};
use almanac::shell::calendar_client::GoogleCalendarClient;

const CALENDAR_LIST: &str = "https://www.googleapis.com/calendar/v3/users/me/calendarList";

/// The calendars, and which mapping profiles point at each.
const WANTED: [(&str, &str); 2] = [
    ("Almanac · Huishouden", "home-assistant"),
    ("Almanac · Infra", "grafana, uptime-kuma"),
];

#[tokio::main]
async fn main() {
    let credentials = match load_credentials() {
        Ok(credentials) => credentials,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let http = reqwest::Client::new();
    let tokens = TokenManager::new(http.clone(), credentials);
    let client = GoogleCalendarClient::new(http.clone(), tokens.clone());

    let token = match tokens.token().await {
        Ok(token) => token,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    // What already exists, so a second run is harmless.
    let existing: serde_json::Value = http
        .get(CALENDAR_LIST)
        .bearer_auth(&token)
        .send()
        .await
        .expect("calendar list request failed")
        .json()
        .await
        .expect("calendar list did not parse");
    let existing = existing["items"].as_array().cloned().unwrap_or_default();

    for (name, sources) in WANTED {
        let already = existing
            .iter()
            .find(|c| c["summary"].as_str() == Some(name))
            .and_then(|c| c["id"].as_str());

        match already {
            Some(id) => println!("{name}\n  already exists\n  id: {id}\n  for: {sources}\n"),
            None => match client.create_calendar(name).await {
                Ok(id) => println!("{name}\n  created\n  id: {id}\n  for: {sources}\n"),
                Err(e) => {
                    eprintln!("{name}\n  FAILED: {e}\n  remedy: {}", e.remedy());
                    std::process::exit(1);
                }
            },
        }
    }

    println!("Next: share them with a real account, or nobody can see what lands there.");
}
