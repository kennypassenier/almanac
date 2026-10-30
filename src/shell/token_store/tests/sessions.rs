//! Session expiry, forged cookies and sessions racing token issues.

use super::*;

#[tokio::test]
async fn a_session_that_has_passed_its_expiry_is_no_longer_live() {
    // The 7-day TTL is enforced in code and was asserted nowhere,
    // even though the clock is injectable and this is two lines.
    let (store, dir) = temp_store("session-ttl");
    store.start_session("cookie", 1_000, 500).await.unwrap();

    assert!(store.session_is_live("cookie", 999).await, "still valid");
    assert!(
        !store.session_is_live("cookie", 1_000).await,
        "a session must not survive its own expiry timestamp"
    );
    assert!(
        !store.session_is_live("cookie", 5_000).await,
        "nor anything after it"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_forged_session_cookie_is_not_live() {
    let (store, dir) = temp_store("session-forged");
    store
        .start_session("real-cookie", 2_000_000_000, 1_000)
        .await
        .unwrap();

    assert!(!store.session_is_live("", 1_000).await);
    assert!(!store.session_is_live("real-cookie-x", 1_000).await);
    assert!(!store.session_is_live("real-cooki", 1_000).await);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn starting_a_session_does_not_lose_a_token_issued_at_the_same_time() {
    // Both maps live in one file. `start_session` used to clone the
    // token map before mutating sessions, so a token issued in
    // between was written and then overwritten away again.
    let (store, dir) = temp_store("no-lost-update");
    let store = std::sync::Arc::new(store);

    store
        .issue("first", "token-first", "2026-08-28")
        .await
        .unwrap();
    store
        .start_session("cookie", 2_000_000_000, 1_000)
        .await
        .unwrap();

    let reopened =
        TokenStore::with_key_loading(store.path().to_path_buf(), [3u8; KEY_BYTES]).unwrap();
    assert!(
        reopened.verify("first", "token-first").await,
        "the token must survive a session being written after it"
    );
    assert!(reopened.session_is_live("cookie", 1_000).await);

    std::fs::remove_dir_all(&dir).ok();
}
