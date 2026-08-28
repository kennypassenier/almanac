//! Standing rule 10: secrets never in logs — and tests **assert** it.
//! Boots the app far enough to attempt authentication with a
//! deliberately invalid private key, captures everything written to
//! stdout/stderr, and asserts the raw key material never appears in
//! it — only the fact that auth failed and why.

use std::io::Write;
use std::process::Command;

/// Startup validates profiles before credentials (M4), so the test
/// needs a valid profile directory to reach the auth step at all.
fn profiles_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("almanac-secrets-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut file = std::fs::File::create(dir.join("test.toml")).unwrap();
    file.write_all(
        br#"
schema_version = 1
source_id = "test-source"
target_calendar_id = "primary"

[mapping]
title_field = "title"
start_field = "start"
duration_minutes = 60
"#,
    )
    .unwrap();
    dir
}

#[test]
fn a_failed_auth_attempt_never_prints_the_private_key() {
    let fake_key =
        "-----BEGIN PRIVATE KEY-----\nTHIS-IS-NOT-A-REAL-KEY-MARKER\n-----END PRIVATE KEY-----";
    let dir = profiles_dir();

    let output = Command::new(env!("CARGO_BIN_EXE_almanac"))
        .env("ALMANAC_PROFILES_DIR", &dir)
        .env("ALMANAC_JOURNAL", dir.join("journal.jsonl"))
        .env("CLIENT_EMAIL", "test@example.iam.gserviceaccount.com")
        .env("PRIVATE_KEY", fake_key)
        .env("TOKEN_URI", "https://oauth2.googleapis.com/token")
        .output()
        .expect("failed to run the almanac binary");

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    std::fs::remove_dir_all(&dir).ok();

    assert!(
        !combined.contains("THIS-IS-NOT-A-REAL-KEY-MARKER"),
        "the private key material leaked into process output:\n{combined}"
    );
    // The failure itself should still be visible and explain what to do.
    assert!(
        combined.contains("PRIVATE_KEY") || combined.contains("private key"),
        "expected the auth failure to name the problem, got:\n{combined}"
    );
}
