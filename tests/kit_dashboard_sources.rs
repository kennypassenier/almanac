//! The dashboard on the kit (M12 as amended 2026-09-06, S1 4.0.2): the kit
//! owns the door, the layout, the login and the Sources page — profile,
//! token and last requests in one row, made in one issue; Almanac keeps
//! its Calendars page. Ported from `dashboard_http.rs`; the login, session,
//! cookie and theme assertions are the kit's own suite now.
//!
//! This file: adding, deleting and listing sources, and their last requests.

mod common;

use common::dashboard::{
    add_source, calendar_list, calendar_names, client_id, first_calendar_id, sources,
};
use common::{body_json, spawn_kit, spawn_kit_with, urlencode};

#[tokio::test]
async fn k21_adding_a_source_writes_its_profile_on_the_chosen_calendar() {
    let hub = spawn_kit_with(&[], Some("kenny@example.com")).await;
    // The dropdown offers what Google lists; make one first.
    let made = hub
        .form("/calendars", &format!("name={}", urlencode("Huishouden")))
        .await;
    let made_status = made.status();
    let made_body = made.text().await.unwrap();
    assert_eq!(
        made_status, 303,
        "the calendar is made and shared: {made_body}"
    );
    let list = calendar_list(&hub).await;
    assert!(
        calendar_names(&list).iter().any(|n| n == "Huishouden"),
        "the new calendar is in the list: {list}"
    );
    let id = first_calendar_id(&list);
    // Visiting the Calendars page remembered the names for the Sources page.
    let page = sources(&hub).await;
    assert!(
        page.contains(&format!("<option value=\"{id}\">Huishouden</option>")),
        "the dropdown offers it by name: {page}"
    );
    // One issue makes profile and token (S1).
    let added = add_source(&hub, "job-tracker", &id).await;
    let added_status = added.status();
    let added_body = added.text().await.unwrap();
    assert_eq!(added_status, 201, "{added_body}");
    assert!(
        hub.dir.path().join("profiles/job-tracker.toml").exists(),
        "the profile is on disk"
    );
    assert!(
        hub.state.profiles().contains_key("job-tracker"),
        "loaded without a restart"
    );
    let page = sources(&hub).await;
    assert!(
        page.contains("job-tracker") && page.contains("Huishouden"),
        "the row shows the calendar by name, not its id: {page}"
    );
    // The id itself is the kit's: its form field and its own "Calendar"
    // column (and, since chassis-rs 3.9.0, that column's filter choices).
    // Almanac's "Calendar name" column shows the name, checked above.
    assert!(page.contains(&id), "the kit's own calendar column: {page}");
    // A rejected name issues nothing and writes nothing.
    let rejected = add_source(&hub, ".bad name", &id).await;
    assert_eq!(rejected.status(), 400);
    let body = rejected.text().await.unwrap();
    assert!(
        body.contains("not allowed") || body.contains("cannot be a source name"),
        "{body}"
    );
    assert!(!hub.dir.path().join("profiles/.bad name.toml").exists());
    let list = body_json(hub.get("/api/clients").await).await;
    assert_eq!(
        list.as_array().unwrap().len(),
        1,
        "no token for the bad name"
    );
    // No calendar: refused too.
    let refused = add_source(&hub, "no-calendar", "").await;
    assert_eq!(refused.status(), 400);
    assert!(refused.text().await.unwrap().contains("pick the calendar"));
    hub.shutdown().await;
}

#[tokio::test]
async fn k21_deleting_a_source_removes_its_profile_and_its_token() {
    let hub = spawn_kit_with(&["home-assistant", "printer"], None).await;
    let token = hub.issue_client("printer").await;
    assert_eq!(
        hub.post_json(
            "/v1/ingest/printer",
            Some(&token),
            r#"{"external_id":"x","title":"t","start":"2026-08-28T09:00:00+00:00"}"#
        )
        .await
        .status(),
        202
    );
    // Events waiting: the kit asks Almanac first, and Almanac refuses.
    let printer = client_id(&hub, "printer").await;
    let refused = hub
        .bearer(
            reqwest::Method::DELETE,
            &format!("/api/clients/{printer}"),
            hub.token(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 400);
    assert!(
        refused
            .text()
            .await
            .unwrap()
            .contains("waiting to be delivered")
    );
    assert!(hub.dir.path().join("profiles/printer.toml").exists());
    assert_eq!(
        hub.post_json("/v1/ingest/printer", Some(&token), "{}")
            .await
            .status(),
        422,
        "its token still works — nothing was deleted"
    );
    let pending = hub.state.journal.pending().unwrap();
    for entry in pending {
        hub.state.journal.mark_done(&entry.id).await.unwrap();
    }
    let deleted = hub
        .bearer(
            reqwest::Method::DELETE,
            &format!("/api/clients/{printer}"),
            hub.token(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), 204);
    assert!(!hub.dir.path().join("profiles/printer.toml").exists());
    assert_eq!(
        hub.post_json("/v1/ingest/printer", Some(&token), "{}")
            .await
            .status(),
        401,
        "its token is gone with it"
    );
    assert!(!hub.page("/clients").await.contains("printer"));
    hub.shutdown().await;
}

#[tokio::test]
async fn k23_an_unusable_profile_is_listed_and_can_be_deleted_and_reload_picks_up_a_hand_written_one()
 {
    let hub = spawn_kit().await;
    std::fs::write(
        hub.dir.path().join("profiles/broken.toml"),
        "this is not = toml [",
    )
    .unwrap();
    // Reload notices the broken file; the page lists it.
    assert_eq!(hub.form("/sources/reload", "").await.status(), 303);
    let files = body_json(hub.get("/calendars/data/profiles").await).await;
    assert!(
        files["unserved"]
            .as_array()
            .is_some_and(|u| u.iter().any(|f| f["file"] == "broken.toml")),
        "{files}"
    );
    assert_eq!(
        hub.form("/profiles/broken.toml/delete", "").await.status(),
        303
    );
    assert!(!hub.dir.path().join("profiles/broken.toml").exists());
    assert_eq!(
        hub.form("/profiles/broken.toml/delete", "").await.status(),
        404
    );
    // A profile placed by hand is served after a reload, no restart.
    std::fs::write(
        hub.dir.path().join("profiles/by-hand.toml"),
        common::profile_toml("by-hand"),
    )
    .unwrap();
    assert!(!hub.state.profiles().contains_key("by-hand"));
    assert_eq!(hub.form("/sources/reload", "").await.status(), 303);
    assert!(hub.state.profiles().contains_key("by-hand"));
    hub.shutdown().await;
}

#[tokio::test]
async fn a_ping_from_a_source_lands_on_its_row_as_a_last_request() {
    // A2-2 revisited (2026-09-06): Almanac's own captures page is gone;
    // the kit keeps each client's last requests on its row (K13). This is
    // what the Sources page's "Send test" button exercises.
    let hub = spawn_kit().await;
    let token = hub.issue_client("probe").await;
    let pinged = hub
        .bearer(reqwest::Method::POST, "/v1/ping", &token)
        .header("content-type", "text/plain")
        .body("<script>alert('pwned')</script>")
        .send()
        .await
        .unwrap();
    assert_eq!(pinged.status(), 200);
    assert_eq!(body_json(pinged).await["caller"], "probe");
    // The row's "Last requests" button fetches the kit's capture list for
    // that client; the page itself never holds the bodies.
    let clients = body_json(hub.get("/api/clients").await).await;
    let id = clients
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "probe")
        .and_then(|c| c["id"].as_str())
        .unwrap()
        .to_string();
    let requests = body_json(hub.get(&format!("/api/clients/{id}/requests")).await).await;
    let listed = requests.to_string();
    assert!(
        listed.contains("/v1/ping"),
        "the row shows the request: {listed}"
    );
    assert!(
        listed.contains("alert('pwned')"),
        "the body is kept verbatim in the JSON (the page escapes it): {listed}"
    );
    hub.shutdown().await;
}
