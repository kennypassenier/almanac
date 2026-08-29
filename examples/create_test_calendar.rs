//! One-off setup tool: creates the "almanac-test" scratch calendar
//! (standing rule 14) under the existing service account and prints
//! its id, so it can be set as `ALMANAC_TEST_CALENDAR_ID` in Latch.
//!
//! This is K3 (multiple calendars) borrowed one milestone early,
//! purely to unblock L1's own E2E test: since the service account
//! creates the calendar itself, it already owns it — no manual step
//! in the Google Calendar UI, no sharing step.
//!
//! Run once:
//!   latch run -- cargo run --example create_test_calendar

use almanac::shell::auth::{TokenManager, load_credentials};
use almanac::shell::calendar_client::GoogleCalendarClient;

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
    let client = GoogleCalendarClient::new(http, tokens);

    // The owner has to be named: a calendar nobody can see is how the
    // original almanac-test calendar sat unnoticed for months.
    let owner = std::env::var("ALMANAC_CALENDAR_OWNER").unwrap_or_default();
    if owner.trim().is_empty() {
        eprintln!("ALMANAC_CALENDAR_OWNER is not set — set it in Latch first");
        std::process::exit(1);
    }

    match client.create_calendar("almanac-test", owner.trim()).await {
        Ok(id) => {
            println!("Created calendar 'almanac-test'.");
            println!("Set this in Latch as ALMANAC_TEST_CALENDAR_ID:");
            println!("{id}");
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
