//! Standing rule 10: secrets never in logs — and tests **assert** it.
//! Boots the app just far enough to authenticate against Google with a
//! deliberately fake (syntactically invalid) private key, captures
//! everything written to stdout/stderr, and asserts the raw key
//! material never appears in it — only the fact that auth failed and
//! why.

use std::process::Command;

#[test]
fn a_failed_auth_attempt_never_prints_the_private_key() {
    let fake_key =
        "-----BEGIN PRIVATE KEY-----\nTHIS-IS-NOT-A-REAL-KEY-MARKER\n-----END PRIVATE KEY-----";

    let output = Command::new(env!("CARGO_BIN_EXE_almanac"))
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
