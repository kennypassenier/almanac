//! Each source delivers onto its own calendar.

use super::*;

#[tokio::test]
async fn each_source_lands_on_its_own_calendar() {
    // K3's acceptance criterion, and the point of the whole
    // project: two profiles writing to two different calendars,
    // with nothing crossing over. It had no test at any level —
    // the calendar id is read inside `deliver`, which until now
    // could only be reached by talking to the real Google.
    let calendar = crate::shell::testing::CalendarStub::start().await;
    let client = stubbed_client(&calendar).await;
    let locks = KeyLocks::new();

    deliver(
        &payload_entry("home-assistant", "bin day"),
        &profile_for("home-assistant", "household@group.calendar.google.com"),
        &client,
        &locks,
    )
    .await
    .unwrap();

    deliver(
        &payload_entry("uptime-kuma", "jellyfin down"),
        &profile_for("uptime-kuma", "infra@group.calendar.google.com"),
        &client,
        &locks,
    )
    .await
    .unwrap();

    assert_eq!(
        calendar.state.calendars_written().await,
        vec![
            "household@group.calendar.google.com".to_string(),
            "infra@group.calendar.google.com".to_string()
        ],
        "an infra alert on the household calendar is exactly what K3 exists to prevent"
    );

    let created = calendar.state.created.lock().await;
    let household: Vec<&str> = created
        .iter()
        .filter(|(cal, _)| cal.starts_with("household"))
        .map(|(_, body)| body["summary"].as_str().unwrap())
        .collect();
    assert_eq!(household, vec!["bin day"], "no cross-contamination");
}

#[tokio::test]
async fn the_lookup_before_an_upsert_uses_the_profiles_own_calendar() {
    // Reading from the wrong calendar would make every delivery
    // look new, so a redelivery would duplicate rather than update
    // — K2 broken by a K3 bug.
    let calendar = crate::shell::testing::CalendarStub::start().await;
    let client = stubbed_client(&calendar).await;
    let locks = KeyLocks::new();

    deliver(
        &payload_entry("uptime-kuma", "jellyfin down"),
        &profile_for("uptime-kuma", "infra@group.calendar.google.com"),
        &client,
        &locks,
    )
    .await
    .unwrap();

    let requests = calendar.state.requests.lock().await;
    assert!(
        requests
            .iter()
            .all(|(_, cal)| cal == "infra@group.calendar.google.com"),
        "every request for this delivery must name the profile's calendar, got {requests:?}"
    );
}
