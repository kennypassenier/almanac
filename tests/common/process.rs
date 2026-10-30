//! Helpers for tests that run the real `almanac` binary as a child
//! process: a scratch data directory, a throwaway key, the environment
//! the binary needs, and the waiting loops.

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How often the waiting loops look again.
pub const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// How long a process may take to take the data-directory lock.
pub const LOCK_WAIT: Duration = Duration::from_secs(10);
/// Pause after the lock appears, so the holder is fully running.
pub const LOCK_SETTLE: Duration = Duration::from_millis(500);
/// How long a process that must exit by itself may take.
pub const EXIT_WAIT: Duration = Duration::from_secs(10);
/// How long a process may take to start answering on its port.
pub const SERVE_WAIT: Duration = Duration::from_secs(20);
/// How long a SIGTERMed process may take to stop (below TimeoutStopSec).
pub const STOP_WAIT: Duration = Duration::from_secs(30);
/// The first startup backoff step is 2 s; this leaves room past it.
pub const RETRY_GRACE: Duration = Duration::from_secs(4);
/// Environment value for the secret key: 64 characters of "1".
const SECRET_KEY_LEN: usize = 64;

pub fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "almanac-lifecycle-{}-{}-{name}",
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

/// A fresh RSA key, so the binary can sign a JWT and reach the network
/// step. Never reused, never committed.
pub fn generate_key(dir: &std::path::Path) -> String {
    let path = dir.join("key.pem");
    let status = Command::new("openssl")
        .args([
            "genpkey",
            "-algorithm",
            "RSA",
            "-pkeyopt",
            "rsa_keygen_bits:2048",
            "-out",
        ])
        .arg(&path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("openssl is needed to generate a throwaway key for this test");
    assert!(status.success(), "openssl could not generate a key");
    std::fs::read_to_string(&path).unwrap()
}

/// A token endpoint that never answers, so the first token fetch fails
/// the way an unreachable Google does: a port this test bound and let go
/// of again, which refuses immediately — a connection failure rather
/// than a hang.
///
/// It used to be port 1, on the assumption that a reserved port always
/// refuses. Under WSL2's mirrored networking it does not: the connect
/// hangs until the client's timeout, so no retry line appeared inside
/// the four seconds `an_unreachable_google_makes_it_retry_rather_than_exit`
/// waits, and the test failed there while CI stayed green (measured
/// 2026-09-26: `curl http://127.0.0.1:1/` timed out after 10 s, a
/// released high port refused in 0 ms).
pub fn blackholed_token_url() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    let port = listener.local_addr().expect("its address").port();
    drop(listener);
    format!("http://127.0.0.1:{port}/token")
}

/// The environment every run of the binary gets in these tests.
pub fn configure<'a>(
    command: &'a mut Command,
    dir: &std::path::Path,
    key: &str,
    token_url: &str,
) -> &'a mut Command {
    command
        .env("ALMANAC_STATE_DIR", dir)
        // 3.0.0: the kit binds; `0` picks a free port so parallel tests never collide.
        .env("ALMANAC_LISTEN", "127.0.0.1:0")
        .env("ALMANAC_PROFILES_DIR", dir)
        .env("ALMANAC_DATA_DIR", dir)
        .env("ALMANAC_JOURNAL", dir.join("journal.jsonl"))
        .env("ALMANAC_TOKEN_STORE", dir.join("tokens.json"))
        .env("ALMANAC_SECRET_KEY", "1".repeat(SECRET_KEY_LEN))
        .env("ALMANAC_TOKEN", "a-login-token-for-the-tests")
        .env("CLIENT_EMAIL", "test@example.iam.gserviceaccount.com")
        .env("PRIVATE_KEY", key)
        .env("TOKEN_URI", token_url)
}

pub fn spawn(dir: &std::path::Path, key: &str, token_url: &str, extra: &[(&str, &str)]) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_almanac"));
    configure(&mut command, dir, key, token_url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in extra {
        command.env(k, v);
    }
    command.spawn().expect("failed to start almanac")
}

/// Waits until `predicate` holds, or fails after `limit`.
pub fn wait_until(limit: Duration, mut predicate: impl FnMut() -> bool, what: &str) {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    panic!("timed out waiting for {what}");
}

pub fn still_running(child: &mut Child) -> bool {
    matches!(child.try_wait(), Ok(None))
}

/// A token endpoint that answers, so the process gets past the
/// startup auth loop and actually binds a listener.
pub async fn serve_token_stub() -> String {
    use axum::Router;
    async fn token() -> axum::Json<serde_json::Value> {
        axum::Json(serde_json::json!({"access_token": "stub", "expires_in": 3600}))
    }
    let router = Router::new().route("/token", axum::routing::post(token));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.ok();
    });
    format!("http://{address}/token")
}

/// A port nothing is using, released before the caller binds it.
pub fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}
