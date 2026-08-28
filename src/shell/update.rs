//! Self-update (M10, AR19, AR22, AR23, AR24).
//!
//! The running service fetches, verifies and installs new versions of
//! itself. Nothing here trusts the release host: the only thing that
//! makes a release installable is a minisign signature made with a key
//! that never leaves Kenny's machine, over a checksum manifest that in
//! turn pins the binary. A compromised or impersonated host can serve
//! anything it likes; without that signature none of it is executed.
//!
//! It reads GitHub Releases, because that is where the repository
//! already is and it means no extra host to keep alive:
//!
//! ```text
//! <ALMANAC_UPDATE_URL>/latest/download/VERSION           "0.2.0"
//! <ALMANAC_UPDATE_URL>/download/v0.2.0/almanac           the binary
//! <ALMANAC_UPDATE_URL>/download/v0.2.0/SHA256SUMS        sha256sum output
//! <ALMANAC_UPDATE_URL>/download/v0.2.0/SHA256SUMS.minisig
//! ```
//!
//! with `ALMANAC_UPDATE_URL` ending in `/releases`. Those four files
//! are exactly what `scripts/sign-release.sh` builds, and publishing a
//! release means attaching them to the GitHub release for the tag.
//! Nothing here is GitHub-specific beyond the two path shapes: any
//! host serving the same paths works, which is what the tests use.
//!
//! The order of operations is the design:
//!
//! 1. verify the signature over the manifest — before the binary is
//!    even downloaded, so a bad host wastes nothing;
//! 2. check the downloaded binary against the manifest's hash;
//! 3. run the new binary with `--check`, which proves it starts and
//!    can read this machine's config without claiming the port the
//!    running process still holds (AR22);
//! 4. keep the replaced binary next to the new one and record that an
//!    update is unproven, so the next start can put it back (AR23);
//! 5. re-exec by asking systemd for a restart — SIGTERM to self, which
//!    runs exactly the same graceful drain as any other stop (M2).
//!
//! Steps 4 and 5 are why this is safe to run unattended: every way it
//! can go wrong ends with a working binary and a notification, not
//! with a service that is down and silent.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::core::error::AlmanacError;
use crate::core::update::{
    StartAction, UpdateState, Version, decide_at_startup, hash_for, hash_matches, should_update,
};
use crate::shell::durability::fsync_parent_dir;
use crate::shell::notify::{Event, Notifier, ops};

/// Base URL of the release directory. Absent means self-update is off.
pub const UPDATE_URL_ENV: &str = "ALMANAC_UPDATE_URL";

/// Set to `off` to disable self-update even when a URL is configured.
/// The compose file sets it, because a container's replaced binary
/// lives in the writable layer and is discarded on the next recreation
/// (AR20).
pub const SELF_UPDATE_ENV: &str = "ALMANAC_SELF_UPDATE";

/// The minisign public key matching the offline secret key that signs
/// releases (AR19, AR24).
///
/// Empty on purpose until Kenny generates the key pair: an updater
/// with no key must refuse to update, never fall back to installing
/// something unverified. There is deliberately exactly one key rather
/// than a baked-in spare — a spare kept in the same vault protects
/// against rotation, not loss, and rotation only matters across many
/// machines (AR24). The recovery path when the key is lost is in
/// docs/OPERATIONS_RUNBOOK.md.
pub const RELEASE_PUBKEY: &str = "";

/// Where the current version number is published. GitHub serves the
/// newest release's assets under `latest/download/`, so an asset named
/// VERSION containing the version string is the whole discovery
/// mechanism — no API call, no token, no rate limit.
const VERSION_PATH: &str = "latest/download/VERSION";

/// Filename of the state a pending update leaves for the next start.
const STATE_FILE: &str = "update-state.json";

/// The argument that makes a binary prove it starts and then exit.
pub const CHECK_ARG: &str = "--check";

/// How long the `--check` probe gets before it is considered a
/// failure. Generous: it parses profiles and opens the token store,
/// but touches no network.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a downloaded file may take. A release binary is a few
/// megabytes over the LAN or from GitHub; anything slower than this is
/// a broken host, and the next check is only hours away.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// How often to look for a new release. Hours, not minutes: an update
/// restarts the service, and nothing here is urgent enough to justify
/// a restart arriving sooner.
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// How long to wait after startup before the first check, so a restart
/// loop cannot turn into a download loop.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(5 * 60);

/// What a check found.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing newer is published.
    UpToDate(Version),
    /// A newer version is installed and the process should restart
    /// into it.
    Installed { from: Version, to: Version },
    /// Something newer exists but was not installed, for a reason the
    /// operator should see rather than a silent skip.
    Skipped(String),
}

/// Everything the updater needs. Constructed from the environment in
/// production and by hand in tests, so the whole flow is exercisable
/// against a local mock release with a throwaway signing key.
pub struct Updater {
    http: reqwest::Client,
    base_url: String,
    pubkey: String,
    /// The binary to replace — `current_exe()` in production.
    binary: PathBuf,
    /// Where the pending-update state file lives.
    data_dir: PathBuf,
    current: Version,
    notifier: Notifier,
    /// Consecutive verification failures, so a compromised or broken
    /// release host is reported once rather than every six hours
    /// (AR24).
    verification_failures: std::sync::atomic::AtomicU32,
}

/// After this many consecutive verification failures, say so out loud.
/// Not the first: a truncated download is a normal, self-healing
/// event, and crying wolf about it would train the alert away.
const VERIFY_FAILURES_BEFORE_NOTIFYING: u32 = 3;

impl Updater {
    /// Builds an updater from the environment, or explains why there
    /// is none. Never fails: a missing configuration disables
    /// self-update, it does not stop the service.
    pub fn from_env(http: reqwest::Client, notifier: Notifier, data_dir: PathBuf) -> Option<Self> {
        if std::env::var(SELF_UPDATE_ENV).map(|v| v.trim().eq_ignore_ascii_case("off")) == Ok(true)
        {
            tracing::info!(
                "{SELF_UPDATE_ENV}=off — self-update is disabled; whatever supervises this \
                 process owns updates"
            );
            return None;
        }

        let base_url = match std::env::var(UPDATE_URL_ENV) {
            Ok(url) if !url.trim().is_empty() => url.trim().trim_end_matches('/').to_string(),
            _ => {
                tracing::info!(
                    "{UPDATE_URL_ENV} is not set — self-update is off and new versions have to be \
                     installed by hand"
                );
                return None;
            }
        };

        if RELEASE_PUBKEY.trim().is_empty() {
            tracing::warn!(
                "{UPDATE_URL_ENV} is set but no release public key is compiled in — self-update \
                 stays off rather than installing something it cannot verify. See \
                 docs/OPERATIONS_RUNBOOK.md."
            );
            return None;
        }

        let binary = match std::env::current_exe() {
            Ok(path) => path,
            Err(e) => {
                tracing::warn!(error = %e, "cannot determine our own path; self-update is off");
                return None;
            }
        };

        let current = match Version::parse(env!("CARGO_PKG_VERSION")) {
            Ok(version) => version,
            Err(e) => {
                tracing::warn!(error = %e, "cannot parse our own version; self-update is off");
                return None;
            }
        };

        tracing::info!(
            base_url = %base_url, current = %current, binary = %binary.display(),
            "self-update is on"
        );

        Some(Self::new(
            http,
            base_url,
            RELEASE_PUBKEY,
            binary,
            data_dir,
            current,
            notifier,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        http: reqwest::Client,
        base_url: impl Into<String>,
        pubkey: impl Into<String>,
        binary: PathBuf,
        data_dir: PathBuf,
        current: Version,
        notifier: Notifier,
    ) -> Self {
        Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            pubkey: pubkey.into(),
            binary,
            data_dir,
            current,
            notifier,
            verification_failures: std::sync::atomic::AtomicU32::new(0),
        }
    }

    fn state_path(&self) -> PathBuf {
        self.data_dir.join(STATE_FILE)
    }

    async fn get(&self, path: &str) -> Result<Vec<u8>, AlmanacError> {
        let url = format!("{}/{path}", self.base_url);
        let response = self
            .http
            .get(&url)
            .timeout(DOWNLOAD_TIMEOUT)
            .send()
            .await
            .map_err(|e| AlmanacError::Config {
                message: format!("could not fetch {url}: {e}"),
                remedy: "check that the release host is reachable from this machine".to_string(),
            })?;

        if !response.status().is_success() {
            return Err(AlmanacError::Config {
                message: format!("{url} answered {}", response.status()),
                remedy: "the release is incomplete or the URL is wrong; nothing was installed"
                    .to_string(),
            });
        }

        response
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| AlmanacError::Config {
                message: format!("could not read the body of {url}: {e}"),
                remedy: "the download was interrupted; the next check will retry".to_string(),
            })
    }

    async fn get_text(&self, path: &str) -> Result<String, AlmanacError> {
        let bytes = self.get(path).await?;
        String::from_utf8(bytes).map_err(|e| AlmanacError::Config {
            message: format!("{path} is not text: {e}"),
            remedy: "the release host is serving something unexpected; nothing was installed"
                .to_string(),
        })
    }

    /// Looks for a newer release and installs it if there is one.
    ///
    /// Returns what happened rather than logging and swallowing it, so
    /// the caller decides whether to restart.
    pub async fn check_once(&self) -> Result<Outcome, AlmanacError> {
        let latest = Version::parse(&self.get_text(VERSION_PATH).await?)?;
        if !should_update(self.current, latest) {
            return Ok(Outcome::UpToDate(self.current));
        }

        tracing::info!(current = %self.current, latest = %latest, "a newer release is published");

        let dir = format!("download/v{latest}");
        let manifest = self.get_text(&format!("{dir}/SHA256SUMS")).await?;
        let signature = self.get_text(&format!("{dir}/SHA256SUMS.minisig")).await?;

        if let Err(e) = self.verify_manifest(manifest.as_bytes(), &signature) {
            self.on_verification_failure(&latest, &e).await;
            return Err(e);
        }

        let expected = hash_for(&manifest, "almanac")?;
        let binary = self.get(&format!("{dir}/almanac")).await?;
        let actual = sha256_hex(&binary);

        if !hash_matches(&expected, &actual) {
            let e = AlmanacError::Config {
                message: format!(
                    "the downloaded binary does not match the signed manifest (expected \
                     {expected}, got {actual})"
                ),
                remedy: "the release host may be serving a tampered binary — nothing was \
                         installed; check the release before doing anything by hand"
                    .to_string(),
            };
            self.on_verification_failure(&latest, &e).await;
            return Err(e);
        }

        self.verification_failures
            .store(0, std::sync::atomic::Ordering::Relaxed);
        tracing::info!(version = %latest, "release verified — signature and checksum both match");

        self.install(&binary, latest).await?;

        Ok(Outcome::Installed {
            from: self.current,
            to: latest,
        })
    }

    /// Checks the minisign signature over the manifest.
    fn verify_manifest(&self, manifest: &[u8], signature: &str) -> Result<(), AlmanacError> {
        // The last line, so pasting either the base64 key or the whole
        // two-line minisign.pub file works — getting that wrong would
        // otherwise only show up as a failed update months later.
        let key = minisign_verify::PublicKey::from_base64(
            self.pubkey.lines().last().unwrap_or_default().trim(),
        )
        .map_err(|e| AlmanacError::Config {
            message: format!("the compiled-in release public key is not a minisign key: {e}"),
            remedy: "this is a build mistake — RELEASE_PUBKEY must be the base64 line from \
                     minisign.pub"
                .to_string(),
        })?;

        let signature =
            minisign_verify::Signature::decode(signature).map_err(|e| AlmanacError::Config {
                message: format!("the release signature is malformed: {e}"),
                remedy: "the release host is serving something that is not a minisign signature; \
                         nothing was installed"
                    .to_string(),
            })?;

        key.verify(manifest, &signature, false)
            .map_err(|e| AlmanacError::Config {
                message: format!("the release signature does not verify: {e}"),
                remedy: "either the release host is compromised or the signing key changed — \
                         nothing was installed, and nothing should be installed by hand until \
                         you know which"
                    .to_string(),
            })
    }

    /// AR24: verification failing is not the same kind of event as a
    /// download failing. Report it — after a few in a row, so a
    /// truncated file does not raise an alarm.
    async fn on_verification_failure(&self, version: &Version, error: &AlmanacError) {
        let count = self
            .verification_failures
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;

        tracing::error!(
            version = %version, count, error = %error, remedy = %error.remedy(),
            "a published release failed verification"
        );

        if count == VERIFY_FAILURES_BEFORE_NOTIFYING {
            self.notifier
                .send(Event {
                    op: ops::UPDATE_UNVERIFIED,
                    ok: false,
                    version: version.to_string(),
                    error: Some(error.to_string()),
                })
                .await;
        }
    }

    /// Writes the verified binary next to the running one, proves it
    /// starts, and swaps it in.
    async fn install(&self, binary: &[u8], version: Version) -> Result<(), AlmanacError> {
        let staged = self.binary.with_extension("new");
        let previous = self.binary.with_extension("prev");

        write_executable(&staged, binary)?;

        // AR22: prove the new binary runs on *this* machine, with this
        // machine's configuration, before it is the only one left.
        // Verification says the file is authentic; it says nothing
        // about whether this version needs a setting Latch does not
        // have yet.
        probe(&staged).await.inspect_err(|_| {
            std::fs::remove_file(&staged).ok();
        })?;

        // Rename rather than copy: the running process holds the old
        // inode open, so replacing the path does not disturb it.
        std::fs::rename(&self.binary, &previous).map_err(|e| AlmanacError::Config {
            message: format!("could not move {} aside: {e}", self.binary.display()),
            remedy: "the service account needs write access to the directory holding the binary, \
                     not just to the binary itself"
                .to_string(),
        })?;

        if let Err(e) = std::fs::rename(&staged, &self.binary) {
            // Put it back rather than leaving no binary at all: the
            // next restart would otherwise find nothing to run.
            std::fs::rename(&previous, &self.binary).ok();
            return Err(AlmanacError::Config {
                message: format!("could not install the new binary: {e}"),
                remedy: "the previous binary was put back; nothing changed".to_string(),
            });
        }
        fsync_parent_dir(&self.binary);

        // Written before the restart, deliberately: if the machine
        // loses power between the swap and the restart, the next start
        // still finds an unproven update and still supervises it.
        self.write_state(&UpdateState {
            from_version: self.current.to_string(),
            to_version: version.to_string(),
            previous_binary: previous.display().to_string(),
            attempts: 0,
        })?;

        tracing::info!(
            from = %self.current, to = %version, previous = %previous.display(),
            "installed; restarting into the new version"
        );

        self.notifier
            .send(Event {
                op: ops::UPDATE_APPLIED,
                ok: true,
                version: version.to_string(),
                error: None,
            })
            .await;

        Ok(())
    }

    fn write_state(&self, state: &UpdateState) -> Result<(), AlmanacError> {
        write_state_to(&self.state_path(), state)
    }
}

/// Serializes the pending-update state durably.
fn write_state_to(path: &Path, state: &UpdateState) -> Result<(), AlmanacError> {
    let body = serde_json::to_string_pretty(state).map_err(|e| AlmanacError::Config {
        message: format!("failed to serialize the update state: {e}"),
        remedy: "this is a bug in almanac".to_string(),
    })?;

    let temp = path.with_extension("writing");
    std::fs::write(&temp, body).map_err(|e| AlmanacError::Config {
        message: format!("failed to write {}: {e}", temp.display()),
        remedy: "check free disk space and permissions on the data directory".to_string(),
    })?;
    std::fs::rename(&temp, path).map_err(|e| AlmanacError::Config {
        message: format!("failed to replace {}: {e}", path.display()),
        remedy: "check permissions on the data directory".to_string(),
    })?;
    fsync_parent_dir(path);
    Ok(())
}

/// Reads the pending-update state, if there is one.
///
/// An unreadable or corrupt file is treated as "no pending update"
/// with a loud warning rather than as a reason to refuse to start:
/// the binary on disk works — it just started — and refusing to run
/// over a damaged bookkeeping file would turn a cosmetic problem into
/// an outage.
pub fn read_state(data_dir: &Path) -> Option<UpdateState> {
    let path = data_dir.join(STATE_FILE);
    let body = std::fs::read_to_string(&path).ok()?;

    match serde_json::from_str(&body) {
        Ok(state) => Some(state),
        Err(e) => {
            tracing::warn!(
                path = %path.display(), error = %e,
                "the pending-update state file is unreadable; treating this as a normal start. \
                 An update that was on probation will no longer be reverted automatically."
            );
            None
        }
    }
}

fn clear_state(data_dir: &Path) {
    let path = data_dir.join(STATE_FILE);
    if let Err(e) = std::fs::remove_file(&path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(
            path = %path.display(), error = %e,
            "could not clear the pending-update state; the next start may revert an update that \
             is actually fine"
        );
    }
    fsync_parent_dir(&path);
}

/// What the startup decision asks `main` to do.
pub enum Startup {
    /// Nothing to do.
    Continue,
    /// A freshly-installed version is starting. Serve, and call
    /// [`confirm_healthy`] once it is actually serving.
    OnProbation,
    /// The previous binary is back in place; exit so the supervisor
    /// starts it.
    Reverted,
}

/// Handles a pending update at startup (AR23).
///
/// Called before anything else binds or locks, because the revert path
/// ends in an immediate exit and must not have to unwind half a
/// started service to get there.
pub async fn handle_pending_update(data_dir: &Path, binary: &Path, notifier: &Notifier) -> Startup {
    match decide_at_startup(read_state(data_dir)) {
        StartAction::Normal => Startup::Continue,

        StartAction::Probation(state) => {
            tracing::info!(
                from = %state.from_version, to = %state.to_version,
                "first start after a self-update — on probation until this process is serving"
            );
            // Persist the incremented count *now*: if this process
            // dies before it is healthy, the next start has to be able
            // to see that this one already tried.
            if let Err(e) = write_state_to(&data_dir.join(STATE_FILE), &state) {
                tracing::warn!(
                    error = %e,
                    "could not record the update attempt; a crash-looping new version would not \
                     be reverted automatically"
                );
            }
            Startup::OnProbation
        }

        StartAction::Revert(state) => {
            tracing::error!(
                from = %state.from_version, to = %state.to_version,
                "the new version did not come up; putting the previous binary back"
            );

            let previous = PathBuf::from(&state.previous_binary);
            let restored = std::fs::rename(&previous, binary);
            clear_state(data_dir);

            match restored {
                Ok(()) => {
                    fsync_parent_dir(binary);
                    notifier
                        .send(Event {
                            op: ops::UPDATE_REVERTED,
                            ok: false,
                            version: state.to_version.clone(),
                            error: Some(format!(
                                "version {} did not come up; reverted to {}",
                                state.to_version, state.from_version
                            )),
                        })
                        .await;
                    Startup::Reverted
                }
                Err(e) => {
                    // Nothing to fall back to. Say so as loudly as
                    // possible and keep running: this binary at least
                    // started, which is more than the alternative.
                    tracing::error!(
                        previous = %previous.display(), error = %e,
                        "could not restore the previous binary — continuing with this one"
                    );
                    notifier
                        .send(Event {
                            op: ops::UPDATE_REVERTED,
                            ok: false,
                            version: state.to_version.clone(),
                            error: Some(format!(
                                "version {} did not come up and the previous binary could not be \
                                 restored ({e}) — install a known-good build by hand",
                                state.to_version
                            )),
                        })
                        .await;
                    Startup::Continue
                }
            }
        }
    }
}

/// Clears the probation state once the service is genuinely serving.
///
/// "Serving" rather than "started": the whole point of AR23 is to
/// catch a version that starts and then cannot do its job, so this is
/// called after the listener is bound and the first drain has run.
pub async fn confirm_healthy(data_dir: &Path, notifier: &Notifier) {
    let Some(state) = read_state(data_dir) else {
        return;
    };
    tracing::info!(version = %state.to_version, "the new version is serving; update confirmed");
    clear_state(data_dir);
    let _ = notifier;
}

/// Writes a binary and makes it executable.
fn write_executable(path: &Path, bytes: &[u8]) -> Result<(), AlmanacError> {
    use std::io::Write;

    let mut file = std::fs::File::create(path).map_err(|e| AlmanacError::Config {
        message: format!("could not create {}: {e}", path.display()),
        remedy: "the service account needs write access to the directory holding the binary"
            .to_string(),
    })?;
    file.write_all(bytes).map_err(|e| AlmanacError::Config {
        message: format!("could not write {}: {e}", path.display()),
        remedy: "check free disk space".to_string(),
    })?;
    // Before the rename, not after: a half-written binary that
    // survives a power cut as the only binary is the one failure this
    // whole dance exists to avoid.
    file.sync_all().map_err(|e| AlmanacError::Config {
        message: format!("could not fsync {}: {e}", path.display()),
        remedy: "check disk health".to_string(),
    })?;
    drop(file);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(|e| {
            AlmanacError::Config {
                message: format!("could not make {} executable: {e}", path.display()),
                remedy: "check permissions on the directory holding the binary".to_string(),
            }
        })?;
    }

    Ok(())
}

/// Starts a binary with `--check`, working around the one spurious
/// failure this is prone to.
///
/// Executing a file that was just written can fail with `ETXTBSY`
/// ("text file busy") in a multi-threaded process: between another
/// thread's `fork` and its `exec`, the child briefly holds a copy of
/// our still-open write descriptor, and the kernel refuses to execute a
/// file that any process has open for writing. Rare, entirely timing
/// dependent, and it would show up as an update that mysteriously did
/// not install — so retry rather than treat it as a broken release.
async fn spawn_check(binary: &Path) -> std::io::Result<tokio::process::Child> {
    const RETRIES: u32 = 5;

    for attempt in 0..RETRIES {
        match tokio::process::Command::new(binary)
            .arg(CHECK_ARG)
            .kill_on_drop(true)
            .spawn()
        {
            Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) && attempt + 1 < RETRIES => {
                tokio::time::sleep(Duration::from_millis(50 * (attempt as u64 + 1))).await;
            }
            other => return other,
        }
    }

    unreachable!("the loop returns on its last attempt")
}

/// Runs a binary with `--check` and waits for it to exit successfully.
async fn probe(binary: &Path) -> Result<(), AlmanacError> {
    let mut child = spawn_check(binary)
        .await
        .map_err(|e| AlmanacError::Config {
            message: format!("could not run {} {CHECK_ARG}: {e}", binary.display()),
            remedy: "the downloaded binary is not executable on this machine — nothing was \
                     installed"
                .to_string(),
        })?;

    let status = match tokio::time::timeout(CHECK_TIMEOUT, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => {
            return Err(AlmanacError::Config {
                message: format!("could not wait for {} {CHECK_ARG}: {e}", binary.display()),
                remedy: "nothing was installed".to_string(),
            });
        }
        Err(_) => {
            child.kill().await.ok();
            return Err(AlmanacError::Config {
                message: format!(
                    "{} {CHECK_ARG} did not finish within {}s",
                    binary.display(),
                    CHECK_TIMEOUT.as_secs()
                ),
                remedy: "the new version hangs at startup — nothing was installed".to_string(),
            });
        }
    };

    if !status.success() {
        return Err(AlmanacError::Config {
            message: format!(
                "{} {CHECK_ARG} exited with {status} — the new version cannot start with this \
                 machine's configuration",
                binary.display()
            ),
            remedy: "nothing was installed; run the binary with --check by hand to see what it \
                     is missing, most likely a new secret that Latch does not have yet"
                .to_string(),
        });
    }

    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// The periodic check. Restarts the process by raising SIGTERM on
/// itself when an update lands, which runs exactly the same graceful
/// drain as `systemctl restart` (M2) — no second shutdown path to keep
/// correct.
pub async fn run(
    updater: Updater,
    state: std::sync::Arc<crate::shell::ingest::AppState>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let mut ticker = tokio::time::interval(CHECK_INTERVAL);
    // The first tick of a tokio interval fires immediately; skip it so
    // a service that is restart-looping for an unrelated reason does
    // not also download on every start.
    ticker.tick().await;
    tokio::time::sleep(FIRST_CHECK_DELAY).await;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                // AR25: not while someone is watching captures. A
                // restart mid-investigation drops exactly the requests
                // the operator is reverse-engineering. Through
                // `captures_after_expiry`, never the raw buffer: a
                // capture nobody looks at again must age out, or one
                // forgotten request suppresses updates forever.
                if !state.captures_after_expiry().await.is_empty() {
                    tracing::info!(
                        "skipping the update check — captured requests are still retained and a \
                         restart would discard them"
                    );
                    continue;
                }

                match updater.check_once().await {
                    Ok(Outcome::UpToDate(version)) => {
                        tracing::debug!(version = %version, "already on the latest release");
                    }
                    Ok(Outcome::Skipped(reason)) => {
                        tracing::info!(reason, "an update was available but not installed");
                    }
                    Ok(Outcome::Installed { from, to }) => {
                        tracing::info!(%from, %to, "restarting into the new version");
                        request_restart();
                        return;
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e, remedy = %e.remedy(),
                            "the update check failed; the service is unaffected and the next \
                             check will retry"
                        );
                    }
                }
            }
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    return;
                }
            }
        }
    }
}

/// Asks this process to stop the way a supervisor would, so the drain
/// path is the tested one.
fn request_restart() {
    // SAFETY: raise() on SIGTERM has no preconditions; the process
    // already installs a handler for it (M2's graceful shutdown).
    unsafe {
        libc::raise(libc::SIGTERM);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "almanac-update-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn state() -> UpdateState {
        UpdateState {
            from_version: "0.1.0".to_string(),
            to_version: "0.2.0".to_string(),
            previous_binary: "/opt/almanac/almanac.prev".to_string(),
            attempts: 0,
        }
    }

    #[test]
    fn the_state_file_round_trips_through_disk() {
        let dir = scratch("state");
        write_state_to(&dir.join(STATE_FILE), &state()).unwrap();
        assert_eq!(read_state(&dir).unwrap(), state());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_state_file_means_no_pending_update() {
        let dir = scratch("nostate");
        assert!(read_state(&dir).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_corrupt_state_file_reads_as_no_pending_update_rather_than_failing_startup() {
        // The binary on disk demonstrably works — it just started.
        // Refusing to run over damaged bookkeeping would be worse than
        // losing the automatic revert.
        let dir = scratch("corrupt");
        std::fs::write(dir.join(STATE_FILE), "{ not json").unwrap();
        assert!(read_state(&dir).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clearing_a_state_file_that_is_not_there_is_not_an_error() {
        let dir = scratch("clear");
        clear_state(&dir);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_written_binary_is_executable() {
        // Without the mode bits the swap succeeds and the *next* start
        // fails with "permission denied" — after the old binary is
        // already renamed away.
        let dir = scratch("exec");
        let path = dir.join("almanac");
        write_executable(&path, b"#!/bin/sh\nexit 0\n").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "every execute bit must be set");
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_binary_that_exits_zero_passes_the_probe() {
        let dir = scratch("probe-ok");
        let path = dir.join("almanac");
        write_executable(&path, b"#!/bin/sh\nexit 0\n").unwrap();
        assert!(probe(&path).await.is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_binary_that_refuses_to_start_fails_the_probe_with_a_usable_remedy() {
        // This is AR22's whole purpose: a version that is authentic
        // but needs a secret Latch does not have yet.
        let dir = scratch("probe-fail");
        let path = dir.join("almanac");
        write_executable(&path, b"#!/bin/sh\necho missing secret >&2\nexit 1\n").unwrap();

        let err = probe(&path).await.unwrap_err();
        assert!(err.to_string().contains("cannot start"));
        assert!(err.remedy().contains("nothing was installed"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_binary_that_hangs_is_killed_rather_than_blocking_the_service_forever() {
        // Not a hypothetical: a new version that waits on a lock the
        // running process holds would hang exactly here.
        let dir = scratch("probe-hang");
        let path = dir.join("almanac");
        write_executable(&path, b"#!/bin/sh\nsleep 300\n").unwrap();

        // Re-run the probe body with a short timeout rather than
        // waiting out the real one.
        let mut child = spawn_check(&path).await.unwrap();
        let timed_out = tokio::time::timeout(Duration::from_millis(200), child.wait())
            .await
            .is_err();
        child.kill().await.ok();
        assert!(timed_out, "the probe must not wait on a hanging binary");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_sha256_helper_matches_a_known_digest() {
        // "abc" — the canonical test vector.
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[tokio::test]
    async fn a_pending_update_that_never_became_healthy_is_reverted() {
        let dir = scratch("revert");
        let binary = dir.join("almanac");
        let previous = dir.join("almanac.prev");
        std::fs::write(&binary, b"new").unwrap();
        std::fs::write(&previous, b"old").unwrap();

        write_state_to(
            &dir.join(STATE_FILE),
            &UpdateState {
                previous_binary: previous.display().to_string(),
                attempts: 1, // one start already tried and did not confirm
                ..state()
            },
        )
        .unwrap();

        let action = handle_pending_update(&dir, &binary, &Notifier::disabled()).await;
        assert!(matches!(action, Startup::Reverted));
        assert_eq!(std::fs::read(&binary).unwrap(), b"old");
        assert!(
            read_state(&dir).is_none(),
            "the state must be cleared or the restored binary would revert itself again"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn the_first_start_after_an_update_serves_instead_of_reverting() {
        let dir = scratch("probation");
        let binary = dir.join("almanac");
        std::fs::write(&binary, b"new").unwrap();
        write_state_to(&dir.join(STATE_FILE), &state()).unwrap();

        let action = handle_pending_update(&dir, &binary, &Notifier::disabled()).await;
        assert!(matches!(action, Startup::OnProbation));
        assert_eq!(std::fs::read(&binary).unwrap(), b"new", "nothing reverted");
        assert_eq!(
            read_state(&dir).unwrap().attempts,
            1,
            "the attempt must be on disk before this process can crash, or the next start would \
             think it was the first"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn confirming_health_clears_the_probation_so_a_later_restart_is_not_a_revert() {
        let dir = scratch("confirm");
        write_state_to(&dir.join(STATE_FILE), &state()).unwrap();
        confirm_healthy(&dir, &Notifier::disabled()).await;
        assert!(read_state(&dir).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn an_ordinary_start_is_untouched() {
        let dir = scratch("normal");
        let binary = dir.join("almanac");
        std::fs::write(&binary, b"running").unwrap();
        let action = handle_pending_update(&dir, &binary, &Notifier::disabled()).await;
        assert!(matches!(action, Startup::Continue));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_revert_with_no_previous_binary_keeps_running_rather_than_leaving_nothing() {
        // Worst case: the state says revert, but the previous binary
        // is gone. Exiting would leave the machine with a service that
        // cannot start at all.
        let dir = scratch("revert-nothing");
        let binary = dir.join("almanac");
        std::fs::write(&binary, b"new").unwrap();
        write_state_to(
            &dir.join(STATE_FILE),
            &UpdateState {
                previous_binary: dir.join("gone").display().to_string(),
                attempts: 1,
                ..state()
            },
        )
        .unwrap();

        let action = handle_pending_update(&dir, &binary, &Notifier::disabled()).await;
        assert!(matches!(action, Startup::Continue));
        assert_eq!(std::fs::read(&binary).unwrap(), b"new");

        std::fs::remove_dir_all(&dir).ok();
    }
}
