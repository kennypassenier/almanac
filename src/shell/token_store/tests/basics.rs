//! Issue, verify, revoke, reveal and list: the store's plain contract.

use super::*;

#[tokio::test]
async fn an_issued_token_verifies_and_a_wrong_one_does_not() {
    let (store, dir) = temp_store("verify");
    store
        .issue("home-assistant", "tok-abc", "now")
        .await
        .unwrap();

    assert!(store.verify("home-assistant", "tok-abc").await);
    assert!(!store.verify("home-assistant", "tok-xyz").await);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn one_sources_token_does_not_verify_for_another() {
    let (store, dir) = temp_store("cross");
    store
        .issue("home-assistant", "ha-tok", "now")
        .await
        .unwrap();
    store.issue("uptime-kuma", "kuma-tok", "now").await.unwrap();

    assert!(!store.verify("uptime-kuma", "ha-tok").await);
    assert!(store.verify("uptime-kuma", "kuma-tok").await);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_revoked_token_stops_working_immediately() {
    // M12's sharpest exit criterion: revocation must take effect on
    // the next request, not on the next restart.
    let (store, dir) = temp_store("revoke");
    store
        .issue("home-assistant", "tok-abc", "now")
        .await
        .unwrap();
    assert!(store.verify("home-assistant", "tok-abc").await);

    assert!(store.revoke("home-assistant").await.unwrap());
    assert!(!store.verify("home-assistant", "tok-abc").await);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn revoking_something_that_was_never_issued_says_so() {
    let (store, dir) = temp_store("revoke-missing");
    assert!(!store.revoke("never-existed").await.unwrap());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn reissuing_replaces_the_previous_token() {
    let (store, dir) = temp_store("reissue");
    store.issue("home-assistant", "old", "t1").await.unwrap();
    store.issue("home-assistant", "new", "t2").await.unwrap();

    assert!(
        !store.verify("home-assistant", "old").await,
        "the old one must stop working"
    );
    assert!(store.verify("home-assistant", "new").await);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_file_on_disk_never_contains_the_plaintext_token() {
    // Standing rule 10, asserted rather than assumed.
    let (store, dir) = temp_store("plaintext");
    store
        .issue("home-assistant", "PLAINTEXT-MARKER-9f3", "now")
        .await
        .unwrap();

    let contents = std::fs::read_to_string(store.path()).unwrap();
    assert!(
        !contents.contains("PLAINTEXT-MARKER-9f3"),
        "the token leaked into the store file:\n{contents}"
    );
    assert!(
        contents.contains("home-assistant"),
        "but the source id is fine to store"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn reveal_returns_a_working_token_and_none_for_an_unknown_source() {
    let (store, dir) = temp_store("reveal");
    store
        .issue("home-assistant", "tok-abc", "now")
        .await
        .unwrap();

    assert_eq!(
        store.reveal("home-assistant").await.unwrap().as_deref(),
        Some("tok-abc")
    );
    assert_eq!(store.reveal("nobody").await.unwrap(), None);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn listing_reports_sources_without_their_tokens() {
    let (store, dir) = temp_store("list");
    store
        .issue("home-assistant", "tok-abc", "2026-08-28")
        .await
        .unwrap();

    let listed = store.list().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].0, "home-assistant");
    assert_eq!(listed[0].1, "2026-08-28");

    std::fs::remove_dir_all(&dir).ok();
}
