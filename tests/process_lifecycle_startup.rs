//! What the real binary does at startup when Google is unreachable or the key is broken (T5, AR21).
//!
//! Split out of the former `tests/process_lifecycle.rs`; the helpers that
//! start the real binary live in `tests/common/process.rs`.

mod common;

use common::process::{
    EXIT_WAIT, RETRY_GRACE, blackholed_token_url, generate_key, scratch, spawn, still_running,
    wait_until,
};

#[test]
fn an_unreachable_google_makes_it_retry_rather_than_exit() {
    // AR21, and SCOPE criterion 3. After a power cut the LXC can start
    // Almanac before the network settles. If a network failure were
    // treated as permanent the unit would exit, and with no start
    // limit systemd would restart-loop it forever; if a genuinely
    // broken key were treated as transient it would look alive while
    // doing nothing. This is the transient half, which nothing covered.
    let dir = scratch("startup-retry");
    let key = generate_key(&dir);
    let mut child = spawn(&dir, &key, &blackholed_token_url(), &[]);

    // The first backoff step is 2s; give it room and then some.
    std::thread::sleep(RETRY_GRACE);

    let alive = still_running(&mut child);
    child.kill().ok();
    let output = child.wait_with_output().unwrap();
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(&dir).ok();

    assert!(
        alive,
        "an unreachable token endpoint must not end the process:\n{printed}"
    );
    assert!(
        printed.contains("retrying"),
        "and it should say what it is doing:\n{printed}"
    );
}

#[test]
fn a_broken_private_key_exits_instead_of_retrying_forever() {
    // The other half of AR21: a malformed key never fixes itself, so
    // retrying it forever would leave Almanac looking alive while
    // delivering nothing.
    let dir = scratch("startup-permanent");
    let mut child = spawn(&dir, "not-a-pem", &blackholed_token_url(), &[]);

    wait_until(
        EXIT_WAIT,
        || !still_running(&mut child),
        "a broken key to end the process",
    );

    let output = child.wait_with_output().unwrap();
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(&dir).ok();

    assert!(!output.status.success(), "it must exit non-zero");
    assert!(
        printed.contains("private key"),
        "and name the problem:\n{printed}"
    );
}
