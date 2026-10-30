use super::*;

fn temp_store(name: &str) -> (TokenStore, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "almanac-tokenstore-{}-{}-{name}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tokens.json");
    (TokenStore::with_key(path, [3u8; KEY_BYTES]), dir)
}

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

#[tokio::test]
async fn an_empty_store_passes_the_startup_key_check() {
    let (store, dir) = temp_store("keycheck-empty");
    assert!(store.verify_key_opens_store().await.is_ok());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn logging_in_while_a_token_is_issued_does_not_deadlock() {
    // `issue` takes tokens-then-sessions; `start_session` used to
    // take sessions-then-tokens. Under real concurrency that is a
    // deadlock, and because `verify` also wants `tokens` and
    // tokio's RwLock is write-preferring, it takes ingest down with
    // it — while /healthz keeps answering 200. Every other test in
    // this file is sequential, which is exactly why none of them
    // saw it.
    let (store, dir) = temp_store("deadlock");
    let store = std::sync::Arc::new(store);

    let mut work = Vec::new();
    for i in 0..20 {
        let issuing = std::sync::Arc::clone(&store);
        work.push(tokio::spawn(async move {
            issuing
                .issue(&format!("source-{i}"), &format!("token-{i}"), "2026-08-28")
                .await
                .unwrap();
        }));
        let logging_in = std::sync::Arc::clone(&store);
        work.push(tokio::spawn(async move {
            logging_in
                .start_session(&format!("session-{i}"), 2_000_000_000, 1_787_000_000)
                .await
                .unwrap();
        }));
        let verifying = std::sync::Arc::clone(&store);
        work.push(tokio::spawn(async move {
            verifying.verify(&format!("source-{i}"), "whatever").await;
        }));
    }

    let all = futures_join(work);
    let finished = tokio::time::timeout(std::time::Duration::from_secs(20), all).await;
    assert!(
        finished.is_ok(),
        "logins, token issues and verifies must not deadlock each other"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// Awaits every handle in order. A plain helper rather than a new
/// dependency on `futures` for one test.
async fn futures_join(handles: Vec<tokio::task::JoinHandle<()>>) {
    for handle in handles {
        handle.await.unwrap();
    }
}

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

#[tokio::test]
async fn the_startup_key_check_passes_with_the_right_key() {
    let (store, dir) = temp_store("keycheck-ok");
    store.issue("home-assistant", "tok", "now").await.unwrap();
    assert!(store.verify_key_opens_store().await.is_ok());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_startup_key_check_catches_a_wrong_key_instead_of_401ing_later() {
    // The failure this exists to prevent: without the check the
    // process starts happily and every source gets a 401 against a
    // store that looks perfectly intact.
    let (store, dir) = temp_store("keycheck-wrong");
    store.issue("home-assistant", "tok", "now").await.unwrap();

    let wrong = TokenStore::with_key_loading(store.path().to_path_buf(), [9u8; KEY_BYTES])
        .expect("the file itself parses fine — that is the whole problem");
    let err = wrong.verify_key_opens_store().await.unwrap_err();

    assert!(err.to_string().contains("does not open"));
    assert!(
        err.remedy().contains("escrow"),
        "the remedy must point at the way out"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_store_written_with_one_key_does_not_verify_under_another() {
    // Why the encryption key must stay stable, demonstrated: a
    // changed key does not silently accept or reject at random, it
    // consistently fails to match.
    let (store, dir) = temp_store("keychange");
    store
        .issue("home-assistant", "tok-abc", "now")
        .await
        .unwrap();

    let reopened = TokenStore::with_key(store.path().to_path_buf(), [4u8; KEY_BYTES]);
    {
        let mut tokens = reopened.tokens.write().await;
        *tokens = store.tokens.read().await.clone();
    }
    assert!(!reopened.verify("home-assistant", "tok-abc").await);

    std::fs::remove_dir_all(&dir).ok();
}
