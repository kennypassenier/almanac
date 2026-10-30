//! The store under concurrent logins and issues.

use super::*;

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
