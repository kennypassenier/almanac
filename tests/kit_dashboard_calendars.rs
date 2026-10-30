//! The dashboard on the kit (M12 as amended 2026-09-06, S1 4.0.2): the kit
//! owns the door, the layout, the login and the Sources page — profile,
//! token and last requests in one row, made in one issue; Almanac keeps
//! its Calendars page. Ported from `dashboard_http.rs`; the login, session,
//! cookie and theme assertions are the kit's own suite now.
//!
//! This file: creating and deleting calendars on the Calendars page (K24).

mod common;

use common::dashboard::{add_source, calendar_list, calendar_names, client_id, first_calendar_id};
use common::{spawn_kit, spawn_kit_with, urlencode};

#[tokio::test]
async fn k24_without_an_owner_no_calendar_is_created_and_the_page_says_so() {
    let hub = spawn_kit().await;
    let list = calendar_list(&hub).await;
    assert_eq!(
        list["can_create"], false,
        "the page is told it cannot make one, and says so: {list}"
    );
    let made = hub.form("/calendars", "name=Nope").await;
    assert_eq!(made.status(), 200);
    assert!(
        made.text()
            .await
            .unwrap()
            .contains("ALMANAC_CALENDAR_OWNER")
    );
    hub.shutdown().await;
}

#[tokio::test]
async fn k24_a_calendar_in_use_cannot_be_deleted_and_an_unused_one_can() {
    let hub = spawn_kit_with(&[], Some("kenny@example.com")).await;
    let made = hub.form("/calendars", "name=Werk").await;
    let made_status = made.status();
    assert_eq!(made_status, 303, "{}", made.text().await.unwrap());
    let id = first_calendar_id(&calendar_list(&hub).await);
    assert_eq!(add_source(&hub, "werk", &id).await.status(), 201);
    let refused = hub
        .form(&format!("/calendars/{}/delete", urlencode(&id)), "")
        .await;
    assert_eq!(refused.status(), 200);
    assert!(refused.text().await.unwrap().contains("werk writes to it"));
    // The source goes first — on the Sources page, profile and token together.
    let werk = client_id(&hub, "werk").await;
    let gone = hub
        .bearer(
            reqwest::Method::DELETE,
            &format!("/api/clients/{werk}"),
            hub.token(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), 204);
    assert!(!hub.dir.path().join("profiles/werk.toml").exists());
    let deleted = hub
        .form(&format!("/calendars/{}/delete", urlencode(&id)), "")
        .await;
    assert_eq!(deleted.status(), 303, "then the calendar can go");
    let list = calendar_list(&hub).await;
    assert!(
        !calendar_names(&list).iter().any(|n| n == "Werk"),
        "gone from the list at once: {list}"
    );
    hub.shutdown().await;
}
