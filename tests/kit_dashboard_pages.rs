//! The dashboard on the kit (M12 as amended 2026-09-06, S1 4.0.2): the kit
//! owns the door, the layout, the login and the Sources page — profile,
//! token and last requests in one row, made in one issue; Almanac keeps
//! its Calendars page. Ported from `dashboard_http.rs`; the login, session,
//! cookie and theme assertions are the kit's own suite now.
//!
//! This file: the door's addresses, the Sources page layout and the Status page.

mod common;

use common::dashboard::sources;
use common::{body_json, spawn_kit};

#[tokio::test]
async fn the_kit_owns_the_door_and_the_old_addresses_keep_working() {
    let hub = spawn_kit().await;
    for path in [
        "/",
        "/calendars",
        "/clients",
        "/dashboard",
        "/dashboard/sources",
    ] {
        let response = hub.get_anon(path).await;
        assert_eq!(response.status(), 303, "{path} needs a login");
    }
    for (old, new) in [
        ("/dashboard", "/"),
        ("/dashboard/sources", "/calendars"),
        ("/sources", "/calendars"),
        ("/dashboard/captures", "/clients"),
        ("/captures", "/clients"),
    ] {
        let response = hub.get(old).await;
        assert_eq!(response.status(), 303, "{old} redirects");
        assert!(
            response.headers()["location"]
                .to_str()
                .unwrap()
                .ends_with(new),
            "{old} → {new}"
        );
    }
    let status = hub.page("/").await;
    assert!(
        status.contains("Journal") && status.contains("Sources"),
        "{status}"
    );
    let sources = body_json(hub.get("/status/data/sources").await).await;
    assert!(
        sources.to_string().contains("home-assistant"),
        "the Sources section is filled from its route: {sources}"
    );
    let clients = hub.page("/clients").await;
    assert!(
        clients.contains(">Sources</h1>"),
        "the clients page keeps Almanac's word for them"
    );
    hub.shutdown().await;
}

#[tokio::test]
async fn the_sources_page_is_the_kits_with_a_calendar_field_and_column() {
    let hub = spawn_kit().await;
    let page = sources(&hub).await;
    assert!(
        page.contains("kp-lede") && page.contains("kp-nav"),
        "inside the kit's layout"
    );
    assert!(
        page.contains("id=\"field-calendar\" name=\"calendar\""),
        "the issue form asks for the calendar: {page}"
    );
    assert!(
        page.contains("href=\"&#x2f;calendars\""),
        "a Calendars page beside the Sources page (hrefs are escaped): {page}"
    );
    assert!(
        !page.contains("href=\"&#x2f;sources\""),
        "no second Sources page any more: {page}"
    );
    let token = hub.issue_client("home-assistant").await;
    let page = sources(&hub).await;
    assert!(
        page.contains("home-assistant") && page.contains(">Calendar name</th>"),
        "the row and Almanac's own calendar-by-name column: {page}"
    );
    assert!(
        page.contains(">Calendar</th>"),
        "the kit's own field-derived column too (feat-clients-2, chassis-rs 2.0.0): {page}"
    );
    assert!(
        !page.contains(&token),
        "the token never appears in the page"
    );
    hub.shutdown().await;
}

/// K28 (chassis-rs 1.8.0): `vocabulary("source", "sources")` replaces
/// `clients_label`, so every kit sentence — not only the page heading —
/// uses Almanac's word. A refusal is the clearest place a leftover
/// "client" would still show up.
#[tokio::test]
async fn the_kits_own_refusals_say_source_not_client() {
    let hub = spawn_kit().await;
    let response = hub.get("/api/clients/does-not-exist/token").await;
    assert_eq!(response.status(), 404);
    let body = body_json(response).await;
    let message = body["error"].as_str().expect("an error message");
    assert!(
        message.contains("no source with id"),
        "the kit's own wording should say source: {message}"
    );
    assert!(!message.contains("client"), "no leftover client: {message}");
    hub.shutdown().await;
}

/// K29, as rebuilt in 4.3.0: "Reload profiles from disk" is a flash button
/// on the status page's Sources section that stays on the page, and an
/// entry in the Calendars page's More menu; the form route still works.
#[tokio::test]
async fn the_reload_action_moved_to_the_status_pages_sources_section() {
    let hub = spawn_kit().await;
    let status = hub.page("/").await;
    assert!(
        status.contains(r#"data-chassis-flash="/status/sources/reload""#)
            && status.contains("Reload profiles from disk"),
        "the status page's Sources section carries the action: {status}"
    );
    let answer = hub.form("/status/sources/reload", "").await;
    assert_eq!(answer.status(), 200, "the button's route answers in place");
    assert!(
        body_json(answer).await["message"]
            .as_str()
            .is_some_and(|m| m.contains("reloaded")),
        "with the count it reloaded"
    );
    let reloaded = hub.form("/sources/reload", "").await;
    assert_eq!(
        reloaded.status(),
        303,
        "the route the button posts to still works"
    );
    hub.shutdown().await;
}
