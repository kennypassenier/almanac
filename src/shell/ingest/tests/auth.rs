//! Tests of who may post as which source (K6).

use super::*;

#[tokio::test]
async fn the_sources_own_client_authenticates() {
    let state = state_with("home-assistant", "correct-token").await;
    assert!(
        authenticate(&state, "home-assistant", &client("home-assistant"))
            .await
            .is_ok()
    );
    // The admin's login token posts as any source (Kenny's scripts).
    assert!(
        authenticate(&state, "home-assistant", &Caller::Admin)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn a_foreign_client_and_an_unknown_source_answer_the_same_401() {
    // K6's actual promise: one source's token must not open another,
    // and the answer must not reveal which source ids exist.
    let state = state_with("home-assistant", "ha-token").await;
    let mut profiles = (*state.profiles()).clone();
    profiles.insert("uptime-kuma".to_string(), profile("uptime-kuma"));
    state.set_profiles(profiles);
    assert!(
        authenticate(&state, "uptime-kuma", &client("uptime-kuma"))
            .await
            .is_ok()
    );
    let foreign = authenticate(&state, "uptime-kuma", &client("home-assistant"))
        .await
        .unwrap_err();
    assert_eq!(foreign.0, StatusCode::UNAUTHORIZED);
    let unknown = authenticate(&state, "nope", &client("nope"))
        .await
        .unwrap_err();
    assert_eq!(unknown.0, StatusCode::UNAUTHORIZED);
    assert_eq!(foreign.1.0["message"], unknown.1.0["message"]);
}
