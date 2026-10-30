mod calendars;
mod keys;

use super::*;
use crate::core::calendar::{EventDateTime, ExtendedProperties, GoogleEvent};
use serde_json::json;

fn event_with_property(value: Option<&str>) -> GoogleEvent {
    let extended_properties = value.map(|v| {
        let mut private = HashMap::new();
        private.insert(UPSERT_PROPERTY.to_string(), v.to_string());
        ExtendedProperties { private }
    });
    GoogleEvent {
        id: None,
        summary: "t".to_string(),
        description: None,
        location: None,
        color_id: None,
        start: EventDateTime::timed("2026-08-28T09:00:00+00:00".to_string(), "UTC".to_string()),
        end: EventDateTime::timed("2026-08-28T10:00:00+00:00".to_string(), "UTC".to_string()),
        transparency: None,
        status: None,
        reminders: None,
        extended_properties,
    }
}

fn entry(idempotency_key: Option<&str>) -> Entry {
    Entry {
        id: "j1".to_string(),
        source_id: "home-assistant".to_string(),
        received_at: "2026-08-28T09:00:00+00:00".to_string(),
        payload: json!({}),
        idempotency_key: idempotency_key.map(|k| k.to_string()),
    }
}

/// A profile that maps a trivial payload onto `calendar`.
fn profile_for(source_id: &str, calendar: &str) -> Profile {
    let toml = format!(
        r#"
schema_version = 2
source_id = "{source_id}"
target_calendar_id = "{calendar}"

"#
    );
    Profile::parse(&toml, "test.toml").unwrap()
}

fn payload_entry(source_id: &str, title: &str) -> Entry {
    Entry {
        id: format!("j-{title}"),
        source_id: source_id.to_string(),
        received_at: "2026-08-28T09:00:00+00:00".to_string(),
        payload: json!({
            "title": title,
            "start": "2026-08-28T09:00:00+00:00",
            "external_id": title
        }),
        idempotency_key: None,
    }
}

async fn stubbed_client(calendar: &crate::shell::testing::CalendarStub) -> GoogleCalendarClient {
    let tokens = crate::shell::testing::TokenStub::start(3600).await;
    GoogleCalendarClient::with_base_url(
        reqwest::Client::new(),
        crate::shell::auth::TokenManager::new(
            reqwest::Client::new(),
            crate::shell::testing::stub_credentials(&tokens.url),
        ),
        &calendar.base_url,
    )
}
