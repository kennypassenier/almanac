//! Helpers for tests that drive the kit's dashboard pages.

use super::{KitHub, body_json};

pub async fn sources(hub: &KitHub) -> String {
    hub.page("/clients").await
}

/// What the Calendars page fills its calendars card from (4.3.0: the page
/// renders at once and asks for the list, so a slow Google holds only it).
pub async fn calendar_list(hub: &KitHub) -> serde_json::Value {
    body_json(hub.get("/calendars/data/google").await).await
}

/// The first calendar id the Calendars page is given.
pub fn first_calendar_id(list: &serde_json::Value) -> String {
    list["calendars"][0]["id"]
        .as_str()
        .expect("a calendar id in the list")
        .to_string()
}

pub fn calendar_names(list: &serde_json::Value) -> Vec<String> {
    list["calendars"]
        .as_array()
        .expect("the calendars")
        .iter()
        .filter_map(|c| c["name"].as_str().map(str::to_string))
        .collect()
}

/// Add a source the 4.0.2 way: one issue on the Sources page with the
/// calendar, as the admin.
pub async fn add_source(hub: &KitHub, name: &str, calendar: &str) -> reqwest::Response {
    hub.bearer(reqwest::Method::POST, "/api/clients", hub.token())
        .header("content-type", "application/json")
        .body(serde_json::json!({ "name": name, "calendar": calendar }).to_string())
        .send()
        .await
        .unwrap()
}

pub async fn client_id(hub: &KitHub, name: &str) -> String {
    let list = body_json(hub.get("/api/clients").await).await;
    list.as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .and_then(|c| c["id"].as_str())
        .expect("the client is listed")
        .to_string()
}
