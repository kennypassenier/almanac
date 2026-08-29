//! One-off: removes a calendar the service account owns, with its
//! events. Used when a source is retired and its calendar would
//! otherwise sit empty in someone's calendar list forever.
//!
//! Irreversible. Names what it is about to delete before doing it, so a
//! wrong id shows up as a wrong name rather than as a missing calendar.
//!
//!   ALMANAC_DELETE_CALENDAR=<id> latch run -- cargo run --example delete_calendar

use almanac::shell::auth::{TokenManager, load_credentials};

#[tokio::main]
async fn main() {
    let calendar = std::env::var("ALMANAC_DELETE_CALENDAR").expect("set ALMANAC_DELETE_CALENDAR");
    let credentials = load_credentials().expect("credentials via latch run");
    let http = reqwest::Client::new();
    let tokens = TokenManager::new(http.clone(), credentials);
    let token = tokens.token().await.expect("an access token");
    let base = "https://www.googleapis.com/calendar/v3/calendars";
    let id = calendar.replace('@', "%40");

    let meta: serde_json::Value = http
        .get(format!("{base}/{id}"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("reading the calendar")
        .json()
        .await
        .expect("parsing it");
    println!(
        "about to delete: {}",
        meta["summary"].as_str().unwrap_or("<unknown>")
    );

    let status = http
        .delete(format!("{base}/{id}"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("deleting the calendar")
        .status();
    println!("delete: HTTP {status}");
}
