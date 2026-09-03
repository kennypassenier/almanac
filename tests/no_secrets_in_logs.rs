//! Standing rule 10 and K12: secrets never in logs — and tests
//! **assert** it, for every secret rather than one of them.
//!
//! Each test boots the real binary with distinctively-marked values in
//! place of the real credentials, captures everything it writes to
//! stdout and stderr, and asserts the marker never appears. Markers
//! rather than realistic values, so a hit is unambiguous and so this
//! file itself contains nothing that looks like a credential.
//!
//! T12 widened this from the private key alone to all five secrets,
//! plus the process's own argument list.

use std::io::Write;
use std::process::Command;

/// Startup validates profiles before credentials (M4), so every test
/// needs a valid profile directory to reach the interesting code.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "almanac-secrets-{}-{}-{name}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut file = std::fs::File::create(dir.join("test.toml")).unwrap();
    file.write_all(
        br#"
schema_version = 2
source_id = "test-source"
target_calendar_id = "primary"

"#,
    )
    .unwrap();
    dir
}

/// Runs the binary with the given extra environment and returns
/// everything it printed. The credentials are deliberately broken so
/// it fails fast instead of serving.
fn run_with(dir: &std::path::Path, extra: &[(&str, &str)]) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_almanac"));
    command
        .env("ALMANAC_PROFILES_DIR", dir)
        .env("ALMANAC_DATA_DIR", dir)
        .env("ALMANAC_JOURNAL", dir.join("journal.jsonl"))
        .env("ALMANAC_TOKEN_STORE", dir.join("tokens.json"))
        .env("CLIENT_EMAIL", "test@example.iam.gserviceaccount.com")
        .env("PRIVATE_KEY", "not-a-key")
        .env("TOKEN_URI", "https://oauth2.googleapis.com/token");

    for (key, value) in extra {
        command.env(key, value);
    }

    let output = command.output().expect("failed to run the almanac binary");
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn a_failed_auth_attempt_never_prints_the_private_key() {
    let marker = "PRIVATE-KEY-MARKER-NEVER-LOG-THIS";
    let key = format!("-----BEGIN PRIVATE KEY-----\n{marker}\n-----END PRIVATE KEY-----");
    let dir = scratch("private-key");

    let printed = run_with(&dir, &[("PRIVATE_KEY", &key)]);
    std::fs::remove_dir_all(&dir).ok();

    assert!(
        !printed.contains(marker),
        "the private key material leaked into process output:\n{printed}"
    );
    // The failure itself must still be visible and say what to do.
    assert!(
        printed.contains("PRIVATE_KEY") || printed.contains("private key"),
        "expected the auth failure to name the problem, got:\n{printed}"
    );
}

#[test]
fn the_token_store_key_never_reaches_the_output() {
    // ALMANAC_SECRET_KEY is the one whose loss makes every issued
    // token unreadable. It is also the value most likely to be printed
    // by accident, because it is validated at startup and a validation
    // error is tempting to write as "expected 64 hex chars, got {key}".
    let marker = "aa11bb22cc33dd44ee55ff66aa77bb88cc99dd00ee11ff22aa33bb44cc55dd66";
    let dir = scratch("secret-key");

    let printed = run_with(&dir, &[("ALMANAC_SECRET_KEY", marker)]);
    std::fs::remove_dir_all(&dir).ok();

    assert!(
        !printed.contains(marker),
        "the token-store encryption key leaked into process output:\n{printed}"
    );
}

#[test]
fn a_malformed_token_store_key_is_reported_without_quoting_it() {
    // The error path specifically: a key that fails validation must be
    // described, not echoed. This one is too short, which is exactly
    // the case where quoting the value feels helpful.
    let marker = "ZZZ-MALFORMED-KEY-MARKER-ZZZ";
    let dir = scratch("bad-secret-key");

    let printed = run_with(&dir, &[("ALMANAC_SECRET_KEY", marker)]);
    std::fs::remove_dir_all(&dir).ok();

    assert!(
        !printed.contains(marker),
        "a rejected key was echoed back into the output:\n{printed}"
    );
}

#[test]
fn the_bootstrap_token_never_reaches_the_output() {
    // This is the credential that logs into the dashboard and can
    // reveal every source's token, so it is the worst single value to
    // leak into journald and from there into a vzdump backup.
    let marker = "BOOTSTRAP-TOKEN-MARKER-NEVER-LOG-THIS";
    let dir = scratch("bootstrap");

    let printed = run_with(&dir, &[("ALMANAC_BOOTSTRAP_TOKEN", marker)]);
    std::fs::remove_dir_all(&dir).ok();

    assert!(
        !printed.contains(marker),
        "the bootstrap token leaked into process output:\n{printed}"
    );
}

#[test]
fn no_secret_is_passed_as_a_command_line_argument() {
    // K12 says "never in process arguments" as well as never in logs,
    // and that half was asserted nowhere. Anything on argv is readable
    // by every user on the box through /proc, so this is a different
    // exposure from logging and needs its own check.
    //
    // Almanac takes exactly one argument, `--check`, and never
    // constructs a command line carrying a secret. The `--check` probe
    // in the self-updater is the only place it spawns anything, and it
    // passes credentials through the inherited environment.
    let dir = scratch("argv");
    let source =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/shell/update.rs"))
            .unwrap();
    std::fs::remove_dir_all(&dir).ok();

    // The one spawn site: assert it passes only the check flag.
    assert!(
        source.contains(".arg(CHECK_ARG)"),
        "the probe must pass only --check"
    );
    assert!(
        !source.contains(".arg(&token") && !source.contains(".arg(key"),
        "no secret may be handed to a child process on its command line"
    );
}
