//! AR15 regression fixtures: profile + payload + expected event,
//! pinned as separate files (not just inline test data) so a change
//! to the mapping-profile format or the JSON wire shape shows up as a
//! diff against a real fixture, not just a passing/failing assertion
//! buried in a unit test.
//!
//! Since 2.0.0 there are two: the plain shape a dashboard-added source
//! sends, and one carrying every option a source can set per event. The
//! grafana and uptime-kuma fixtures went with the translation layer
//! they existed to prove — neither had ever delivered an event.

use almanac::core::mapping::map_payload;
use almanac::core::profile::Profile;

fn run_fixture(name: &str) {
    let profile_path = format!("fixtures/profiles/{name}.toml");
    let payload_path = format!("fixtures/payloads/{name}_sample.json");
    let expected_path = format!("fixtures/expected/{name}_sample_event.json");

    let profile_toml = std::fs::read_to_string(&profile_path)
        .unwrap_or_else(|e| panic!("reading {profile_path}: {e}"));
    let profile = Profile::parse(&profile_toml, &profile_path).unwrap();

    let payload_json = std::fs::read_to_string(&payload_path)
        .unwrap_or_else(|e| panic!("reading {payload_path}: {e}"));
    let payload: serde_json::Value = serde_json::from_str(&payload_json).unwrap();

    let expected_json = std::fs::read_to_string(&expected_path)
        .unwrap_or_else(|e| panic!("reading {expected_path}: {e}"));
    let expected: serde_json::Value = serde_json::from_str(&expected_json).unwrap();

    let event = map_payload(&payload, &profile, &profile_path).unwrap();
    let actual = serde_json::to_value(&event).unwrap();

    assert_eq!(
        actual, expected,
        "{name}: mapped event no longer matches the pinned fixture — \
         if this change is intentional, update {expected_path} deliberately"
    );
}

/// Every per-event option in one payload. Since 2.0.0 these come from
/// the call rather than the profile, so this is the fixture that proves
/// the new contract — and the one whose JSON shape is most likely to
/// break silently: Google accepts exactly one of `date` and
/// `dateTime`, and the first build of all-day support sent the wrong
/// field names altogether.
#[test]
fn the_everything_fixture_matches_pinned_output() {
    run_fixture("everything");
}

#[test]
fn home_assistant_fixture_matches_pinned_output() {
    run_fixture("home-assistant");
}
