//! One-off setup tool: creates the calendars the mapping profiles
//! target, and makes sure they are shared with a real person.
//!
//! This is K3 (multiple calendars, each source writing to its own) made
//! concrete. Kenny asked for exactly this during L1 — "je kan die toch
//! zelf een nieuwe agenda laten aanmaken? dat is toch één van de punten
//! van dit project?" — so the service account creates and owns them,
//! with no manual step in the Google Calendar UI.
//!
//! **Sharing is not optional and not separate.** A calendar the service
//! account creates is owned by the service account and invisible to
//! everyone else. The first run of this tool created two calendars that
//! nobody could see, and the same turned out to be true of the
//! `almanac-test` calendar from L1: its access list held only the
//! calendar and the service account, so months of live tests wrote into
//! something Kenny had never laid eyes on. Creating without sharing is
//! therefore treated as a half-finished job, and every run re-checks
//! every calendar rather than only the ones it just made.
//!
//! Idempotent throughout: an existing calendar is left alone, and an
//! access rule that already grants what is wanted is not rewritten.
//!
//!   latch run -- cargo run --example create_calendars

use almanac::shell::auth::{TokenManager, load_credentials};
use almanac::shell::calendar_client::GoogleCalendarClient;

const CALENDAR_LIST: &str = "https://www.googleapis.com/calendar/v3/users/me/calendarList";
const CALENDARS: &str = "https://www.googleapis.com/calendar/v3/calendars";

/// Who the calendars are shared with. An address rather than a secret,
/// but it lives in Latch with the rest of the configuration so there is
/// one place that knows how this deployment is set up.
const OWNER_ENV: &str = "ALMANAC_CALENDAR_OWNER";

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

    let owner = match std::env::var(OWNER_ENV) {
        Ok(owner) if !owner.trim().is_empty() => owner.trim().to_string(),
        _ => {
            eprintln!(
                "{OWNER_ENV} is not set — without it these calendars would be created and left \
                 invisible to everyone, which is the mistake this tool exists to stop repeating. \
                 Set it in Latch to the Google account the calendars should appear in."
            );
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

    let existing = calendars(&http, &token).await;

    // Create whatever is missing.
    for (name, sources) in WANTED {
        let already = existing
            .iter()
            .find(|c| c["summary"].as_str() == Some(name))
            .and_then(|c| c["id"].as_str());

        match already {
            Some(id) => println!("{name}\n  exists\n  id: {id}\n  for: {sources}"),
            None => match client.create_calendar(name, &owner).await {
                Ok(id) => println!("{name}\n  created\n  id: {id}\n  for: {sources}"),
                Err(e) => {
                    eprintln!("{name}\n  FAILED: {e}\n  remedy: {}", e.remedy());
                    std::process::exit(1);
                }
            },
        }
    }

    // Then make sure *every* calendar this account can reach is shared,
    // including any created a moment ago and any created earlier by
    // something else.
    println!("\nsharing with {owner}:");
    for calendar in calendars(&http, &token).await {
        let Some(id) = calendar["id"].as_str() else {
            continue;
        };
        let name = calendar["summary"].as_str().unwrap_or(id);
        match share(&http, &token, &client, id, &owner).await {
            Shared::Already => println!("  {name}: already shared"),
            Shared::Granted => println!("  {name}: shared"),
            Shared::Failed(why) => println!("  {name}: FAILED — {why}"),
        }
    }
}

async fn calendars(http: &reqwest::Client, token: &str) -> Vec<serde_json::Value> {
    let list: serde_json::Value = http
        .get(CALENDAR_LIST)
        .bearer_auth(token)
        .send()
        .await
        .expect("calendar list request failed")
        .json()
        .await
        .expect("calendar list did not parse");
    list["items"].as_array().cloned().unwrap_or_default()
}

enum Shared {
    Already,
    Granted,
    Failed(String),
}

/// Grants `owner` access to one calendar, unless it already has it.
///
/// The check first is only to keep the output honest about what
/// changed; `share_calendar` itself is safe to call either way.
async fn share(
    http: &reqwest::Client,
    token: &str,
    client: &GoogleCalendarClient,
    id: &str,
    owner: &str,
) -> Shared {
    let acl: serde_json::Value = match http
        .get(format!("{CALENDARS}/{id}/acl"))
        .bearer_auth(token)
        .send()
        .await
    {
        Ok(response) => response.json().await.unwrap_or(serde_json::Value::Null),
        Err(e) => return Shared::Failed(format!("could not read the access list: {e}")),
    };

    let already = acl["items"]
        .as_array()
        .map(|rules| {
            rules.iter().any(|rule| {
                rule["scope"]["value"].as_str() == Some(owner)
                    && matches!(rule["role"].as_str(), Some("writer") | Some("owner"))
            })
        })
        .unwrap_or(false);
    if already {
        return Shared::Already;
    }

    match client.share_calendar(id, owner).await {
        Ok(()) => Shared::Granted,
        Err(e) => Shared::Failed(e.to_string()),
    }
}
