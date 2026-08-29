//! Read-only setup tool: shows which calendars this service account can
//! reach and who else has access to each.
//!
//! Needed before creating the real calendars, because a calendar the
//! service account creates is owned by the service account — invisible
//! to Kenny until it is explicitly shared. This prints the accounts
//! already on the existing calendars, so the sharing step targets the
//! address he actually uses rather than a guess.
//!
//! Reads nothing but calendar metadata, writes nothing.
//!
//!   latch run -- cargo run --example inspect_calendar_access

use almanac::shell::auth::{TokenManager, load_credentials};

const CALENDAR_LIST: &str = "https://www.googleapis.com/calendar/v3/users/me/calendarList";
const CALENDARS: &str = "https://www.googleapis.com/calendar/v3/calendars";

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
    let token = match tokens.token().await {
        Ok(token) => token,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let list: serde_json::Value = http
        .get(CALENDAR_LIST)
        .bearer_auth(&token)
        .send()
        .await
        .expect("calendar list request failed")
        .json()
        .await
        .expect("calendar list did not parse");

    let items = list["items"].as_array().cloned().unwrap_or_default();
    println!("calendars this service account can reach: {}", items.len());

    for calendar in items {
        let id = calendar["id"].as_str().unwrap_or("<no id>");
        println!(
            "\n  {}  ({})\n    id: {id}",
            calendar["summary"].as_str().unwrap_or("<no name>"),
            calendar["accessRole"].as_str().unwrap_or("?")
        );

        let acl: serde_json::Value = http
            .get(format!("{CALENDARS}/{id}/acl"))
            .bearer_auth(&token)
            .send()
            .await
            .expect("acl request failed")
            .json()
            .await
            .expect("acl did not parse");

        match acl["items"].as_array() {
            Some(rules) => {
                for rule in rules {
                    println!(
                        "    access: {:<10} {}",
                        rule["role"].as_str().unwrap_or("?"),
                        rule["scope"]["value"].as_str().unwrap_or("<everyone>")
                    );
                }
            }
            None => println!("    access: (could not read the ACL on this calendar)"),
        }
    }
}
