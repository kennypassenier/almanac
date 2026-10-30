use super::*;

/// Re-exported from `shell::testing`, which is where it lives now
/// that the stubs are compiled into the library: a `cfg(test)`
/// module cannot be referenced from one that is not.
pub(crate) use crate::shell::testing::TEST_ONLY_THROWAWAY_KEY;

fn test_manager(expires_at_secs: u64) -> TokenManager {
    test_manager_with_key(expires_at_secs, "unused-in-this-test")
}

fn test_manager_with_key(expires_at_secs: u64, private_key: &str) -> TokenManager {
    TokenManager {
        http: Client::new(),
        credentials: ServiceAccountCredentials {
            client_email: "sa@example.iam.gserviceaccount.com".to_string(),
            private_key: private_key.to_string(),
            token_url: "https://example.invalid/token".to_string(),
        },
        metrics: Arc::new(Metrics::default()),
        state: Mutex::new(Some(TokenState {
            access_token: "cached-token".to_string(),
            expires_at_secs,
        })),
    }
}

#[tokio::test]
async fn a_token_well_within_its_lifetime_is_reused_without_a_network_call() {
    let far_future = now_secs().unwrap() + JWT_LIFETIME_SECS;
    let manager = test_manager(far_future);
    // No mock server configured — if this tried to refresh, the
    // connection to the invalid host would fail and this would
    // return Err instead of the cached token.
    let token = manager.token().await.unwrap();
    assert_eq!(token, "cached-token");
}

#[tokio::test]
async fn a_token_inside_the_refresh_margin_triggers_a_refresh_attempt() {
    let about_to_expire = now_secs().unwrap() + 60; // < REFRESH_MARGIN_SECS
    let manager = test_manager(about_to_expire);
    // The refresh targets an unreachable host, so this proves a
    // refresh was actually attempted (and surfaced its failure)
    // rather than silently reusing the stale cached token.
    let result = manager.token().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn a_network_failure_during_refresh_is_transient_not_a_dead_end() {
    // Answers Kenny's question directly: a token refresh that
    // fails because the token endpoint was briefly unreachable
    // must be retried automatically by the caller
    // (shell::calendar_client::send_with_retry), not require
    // restarting the process by hand. A permanently bad key
    // (see the PEM-parse-failure case, exercised via
    // tests/no_secrets_in_logs.rs) is the one case that genuinely
    // cannot self-heal.
    let about_to_expire = now_secs().unwrap() + 60;
    // A syntactically valid key so signing succeeds and the
    // failure actually comes from the network call, not PEM
    // parsing — otherwise this would test the wrong thing.
    let manager = test_manager_with_key(about_to_expire, TEST_ONLY_THROWAWAY_KEY);
    let err = manager.token().await.unwrap_err();
    assert!(
        err.is_transient(),
        "a network-level failure reaching the token endpoint must be classified transient"
    );
}

#[tokio::test]
async fn a_successful_refresh_is_stored_and_the_next_call_reuses_it() {
    // K4's whole point. The three tests that existed proved a
    // refresh was *attempted*; none proved one could succeed, so a
    // regression in storing or reading back the new token would
    // reproduce the original "dies after an hour" defect with CI
    // fully green.
    let stub = crate::shell::testing::TokenStub::start(3600).await;
    let manager = TokenManager::new(
        Client::new(),
        crate::shell::testing::stub_credentials(&stub.url),
    );

    let first = manager.token().await.unwrap();
    assert_eq!(first, "stub-token-0");
    assert_eq!(stub.state.hits(), 1);

    let second = manager.token().await.unwrap();
    assert_eq!(second, first, "a valid token must be reused, not refetched");
    assert_eq!(stub.state.hits(), 1, "no second request to Google");
}

#[tokio::test]
async fn a_token_inside_the_refresh_margin_is_replaced_by_the_new_one() {
    // expires_in = 0 means every check falls inside the 5-minute
    // refresh margin, so this exercises the refresh path itself —
    // and asserts the caller gets the *new* token back, not the
    // stale one it was holding.
    let stub = crate::shell::testing::TokenStub::start(0).await;
    let manager = TokenManager::new(
        Client::new(),
        crate::shell::testing::stub_credentials(&stub.url),
    );

    let first = manager.token().await.unwrap();
    let second = manager.token().await.unwrap();

    assert_eq!(first, "stub-token-0");
    assert_eq!(
        second, "stub-token-1",
        "an expired token must be replaced, and the fresh one returned"
    );
    assert_eq!(stub.state.hits(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn twenty_concurrent_callers_share_one_refresh() {
    // AR18. Without the single-flight lock, a burst of deliveries
    // after a restart each fetch their own token — twenty requests
    // to Google for one credential, which is both wasteful and a
    // good way to get rate limited at exactly the wrong moment.
    let stub = crate::shell::testing::TokenStub::start(3600).await;
    let manager = TokenManager::new(
        Client::new(),
        crate::shell::testing::stub_credentials(&stub.url),
    );

    let mut handles = Vec::new();
    for _ in 0..20 {
        let manager = Arc::clone(&manager);
        handles.push(tokio::spawn(async move { manager.token().await }));
    }

    let mut tokens = Vec::new();
    for handle in handles {
        tokens.push(handle.await.unwrap().unwrap());
    }

    assert_eq!(
        stub.state.hits(),
        1,
        "twenty callers must share one refresh, not start twenty"
    );
    assert!(
        tokens.iter().all(|t| t == "stub-token-0"),
        "and they must all end up with the same token"
    );
}

#[tokio::test]
async fn the_first_call_on_a_cold_manager_fetches_rather_than_failing() {
    // The None state: nothing cached yet, which is every process
    // start and therefore every reboot.
    let stub = crate::shell::testing::TokenStub::start(3600).await;
    let manager = TokenManager::new(
        Client::new(),
        crate::shell::testing::stub_credentials(&stub.url),
    );

    assert_eq!(manager.token().await.unwrap(), "stub-token-0");
    assert_eq!(stub.state.hits(), 1);
}
