//! Graceful SIGTERM stop after serving, and `--version` without configuration (M2, AR30).
//!
//! Split out of the former `tests/process_lifecycle.rs`; the helpers that
//! start the real binary live in `tests/common/process.rs`.

mod common;

use std::process::Command;
use std::time::Instant;

use common::process::{
    POLL_INTERVAL, SERVE_WAIT, STOP_WAIT, free_port, generate_key, scratch, serve_token_stub,
    spawn, still_running,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sigterm_stops_it_cleanly_after_it_is_serving() {
    // M2, and AR30 rides on the same path: self-update restarts by
    // raising SIGTERM on itself. If this exits non-zero or hangs past
    // TimeoutStopSec, systemd SIGKILLs it — possibly mid journal
    // write — during the operation that happens most often.
    //
    // The process has to be genuinely serving first: the graceful
    // handler is only installed once the listener is bound, so
    // signalling it during the startup retry loop would prove nothing.
    let dir = scratch("sigterm");
    let key = generate_key(&dir);
    let token_url = serve_token_stub().await;
    let port = free_port();

    let mut child = spawn(
        &dir,
        &key,
        &token_url,
        &[("ALMANAC_LISTEN", &format!("127.0.0.1:{port}"))],
    );

    let health = format!("http://127.0.0.1:{port}/healthz");
    let client = reqwest::Client::new();
    let deadline = Instant::now() + SERVE_WAIT;
    let mut serving = false;
    while Instant::now() < deadline {
        if client.get(&health).send().await.is_ok() {
            serving = true;
            break;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    assert!(serving, "the process never started serving");

    let pid = child.id() as i32;
    // SIGTERM is what systemd and `docker stop` send.
    assert_eq!(
        unsafe { libc::kill(pid, libc::SIGTERM) },
        0,
        "failed to signal the process"
    );

    let deadline = Instant::now() + STOP_WAIT;
    while Instant::now() < deadline && still_running(&mut child) {
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    assert!(
        !still_running(&mut child),
        "SIGTERM must stop it well within TimeoutStopSec, not need a SIGKILL"
    );

    let output = child.wait_with_output().unwrap();
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(&dir).ok();

    assert!(
        output.status.success(),
        "a graceful stop must exit zero, got {:?}:\n{printed}",
        output.status
    );
    // 3.0.0: the kit logs the stop ("stopped drained=true"); almanac's own
    // lines below still prove the drain ran and finished.
    assert!(
        printed.contains("stopped"),
        "the stop should be visible in the log:\n{printed}"
    );
    assert!(
        printed.contains("draining the worker"),
        "and the drain must actually run — that is the whole point:\n{printed}"
    );
    assert!(
        printed.contains("almanac stopped"),
        "and it should say it finished:\n{printed}"
    );
}

#[test]
fn version_needs_no_configuration_at_all() {
    // Found by the homelab session running the binary by hand outside
    // `latch run` to sanity-check a deploy: every other special mode
    // needs the full production configuration (by design — they answer
    // "can this run here"), so asking "what version is this" started
    // the whole process and complained about a missing webhook and an
    // unreadable profiles directory instead of just answering.
    let output = Command::new(env!("CARGO_BIN_EXE_almanac"))
        .arg("--version")
        .env_clear()
        .output()
        .unwrap();

    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        output.status.success(),
        "--version must succeed with no environment at all, got {:?}:\n{printed}",
        output.status
    );
    assert!(
        printed.contains(env!("CARGO_PKG_VERSION")),
        "it should print the version it was actually built as:\n{printed}"
    );
    assert!(
        !printed.contains("ALMANAC_NOTIFY_WEBHOOK"),
        "it must not touch anything that needs configuration:\n{printed}"
    );

    let short = Command::new(env!("CARGO_BIN_EXE_almanac"))
        .arg("-V")
        .env_clear()
        .output()
        .unwrap();
    assert!(short.status.success(), "-V is the same flag, spelled short");
}
