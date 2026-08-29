//! One-off: read back what is actually on a calendar, so a claim about
//! what Almanac wrote can be checked against Google rather than against
//! Almanac's own log.
//!
//!   ALMANAC_SHOW_CALENDAR=<id> latch run -- cargo run --example show_events

use almanac::shell::auth::{TokenManager, load_credentials};

#[tokio::main]
async fn main() {
    let calendar = std::env::var("ALMANAC_SHOW_CALENDAR").expect("set ALMANAC_SHOW_CALENDAR");
    let credentials = load_credentials().expect("credentials via latch run");
    let http = reqwest::Client::new();
    let tokens = TokenManager::new(http.clone(), credentials);
    let token = tokens.token().await.expect("an access token");

    let url = format!(
        "https://www.googleapis.com/calendar/v3/calendars/{}/events?maxResults=20",
        urlencoding_minimal(&calendar)
    );
    let body: serde_json::Value = http
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .expect("listing events")
        .json()
        .await
        .expect("parsing the event list");

    for e in body["items"].as_array().cloned().unwrap_or_default() {
        println!("  summary      : {}", e["summary"].as_str().unwrap_or("-"));
        println!("  start        : {}", e["start"]);
        println!("  end          : {}", e["end"]);
        println!("  transparency : {}", e["transparency"]);
        println!("  private prop : {}", e["extendedProperties"]["private"]);
        println!();
    }
}

/// Enough escaping for a calendar id, which is an email-shaped string.
fn urlencoding_minimal(s: &str) -> String {
    s.replace('@', "%40")
}
