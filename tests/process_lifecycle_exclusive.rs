//! Two processes on one data directory: the lock, and `--check` against a live instance (T8, AR22).
//!
//! Split out of the former `tests/process_lifecycle.rs`; the helpers that
//! start the real binary live in `tests/common/process.rs`.

mod common;

use std::process::Command;

use common::process::{
    LOCK_SETTLE, LOCK_WAIT, blackholed_token_url, configure, generate_key, scratch, spawn,
    still_running, wait_until,
};

#[test]
fn a_second_process_on_the_same_data_directory_refuses_to_start() {
    // AR22 across two real processes, which is the scenario the lock
    // exists for — the existing tests take the flock twice from within
    // one process. Two workers over one journal deliver the same event
    // twice and can lose delivery records.
    let dir = scratch("two-processes");
    let key = generate_key(&dir);

    let mut first = spawn(&dir, &key, &blackholed_token_url(), &[]);
    // Let it take the lock before the second one tries.
    wait_until(
        LOCK_WAIT,
        || dir.join(".lock").exists(),
        "the first process to take the lock",
    );
    std::thread::sleep(LOCK_SETTLE);

    let second = spawn(&dir, &key, &blackholed_token_url(), &[])
        .wait_with_output()
        .unwrap();
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );

    let first_alive = still_running(&mut first);
    first.kill().ok();
    first.wait().ok();
    std::fs::remove_dir_all(&dir).ok();

    assert!(
        !second.status.success(),
        "the second process must refuse to start:\n{printed}"
    );
    assert!(printed.contains("already holds"), "and say why:\n{printed}");
    assert!(
        first_alive,
        "while the first keeps running — the lock protects it, it does not kill it"
    );
}

#[test]
fn check_mode_runs_against_a_live_instance_without_disturbing_it() {
    // AR22/AR29: the self-update probe runs `--check` while the old
    // process is still serving, so it must take neither the port nor
    // the data-directory lock. If a future change made it reuse the
    // startup path — easy to do — every self-update probe would fail
    // against the running instance and updates would stop, quietly.
    let dir = scratch("check-against-live");
    let key = generate_key(&dir);

    let mut running = spawn(&dir, &key, &blackholed_token_url(), &[]);
    wait_until(
        LOCK_WAIT,
        || dir.join(".lock").exists(),
        "the running instance to take the lock",
    );
    std::thread::sleep(LOCK_SETTLE);

    let mut check_command = Command::new(env!("CARGO_BIN_EXE_almanac"));
    check_command.arg("--check");
    let check = configure(&mut check_command, &dir, &key, &blackholed_token_url())
        .output()
        .unwrap();

    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    let first_alive = still_running(&mut running);
    running.kill().ok();
    running.wait().ok();
    std::fs::remove_dir_all(&dir).ok();

    assert!(
        check.status.success(),
        "--check must succeed against a live instance:\n{printed}"
    );
    assert!(
        printed.contains("--check: ok"),
        "and report it plainly:\n{printed}"
    );
    assert!(first_alive, "and leave the running instance alone");
}
