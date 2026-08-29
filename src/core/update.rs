//! The pure half of self-update (M10): deciding whether a release is
//! newer, and verifying a manifest before anything is trusted.
//!
//! Downloading, replacing the binary and restarting are `shell`'s job
//! (AR13). What lives here is everything that can be got wrong without
//! touching the network — which is where the security-relevant
//! mistakes are.

use serde::{Deserialize, Serialize};

use crate::core::error::AlmanacError;

/// A semantic version, compared numerically. Comparing version strings
/// lexically is the classic bug: "0.10.0" sorts before "0.9.0" as
/// text, so an updater would refuse the newer release forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    /// Parses `1.2.3` or `v1.2.3`. Anything else is refused rather
    /// than guessed at.
    pub fn parse(text: &str) -> Result<Self, AlmanacError> {
        let trimmed = text.trim().trim_start_matches('v');
        let mut parts = trimmed.split('.');

        let mut next = |what: &str| -> Result<u64, AlmanacError> {
            parts
                .next()
                .ok_or(())
                .and_then(|p| p.parse::<u64>().map_err(|_| ()))
                .map_err(|_| AlmanacError::Config {
                    message: format!(
                        "\"{text}\" is not a version: the {what} part is missing or not a number"
                    ),
                    remedy: "expected MAJOR.MINOR.PATCH, optionally prefixed with v".to_string(),
                })
        };

        let major = next("major")?;
        let minor = next("minor")?;
        let patch = next("patch")?;

        if parts.next().is_some() {
            return Err(AlmanacError::Config {
                message: format!("\"{text}\" has more than three version parts"),
                remedy: "expected MAJOR.MINOR.PATCH, optionally prefixed with v".to_string(),
            });
        }

        Ok(Self {
            major,
            minor,
            patch,
        })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Whether to move to `candidate`, given what is running.
///
/// Strictly newer only. Accepting an equal version would make the
/// updater replace its own binary on every poll for no reason;
/// accepting an older one turns a compromised or rolled-back release
/// index into a downgrade attack, where an attacker serves a genuine,
/// correctly-signed *old* release with a known hole.
pub fn should_update(running: Version, candidate: Version) -> bool {
    candidate > running
}

/// Finds the hash a manifest records for `filename`.
///
/// The manifest is `sha256sum` output: one `<hex>  <name>` per line.
/// An entry that is missing is a hard error — never a reason to skip
/// the check, which would defeat the whole verification.
pub fn hash_for(manifest: &str, filename: &str) -> Result<String, AlmanacError> {
    for line in manifest.lines() {
        let mut parts = line.split_whitespace();
        let (Some(hash), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        // sha256sum writes "hash  name" for text and "hash *name" for
        // binary mode; accept both.
        if name.trim_start_matches('*') == filename {
            return Ok(hash.to_string());
        }
    }

    Err(AlmanacError::Config {
        message: format!("the release manifest has no entry for \"{filename}\""),
        remedy: "the release is incomplete or was built differently; do not install it".to_string(),
    })
}

/// Compares a downloaded file's hash against the manifest's.
pub fn hash_matches(expected: &str, actual: &str) -> bool {
    // Case-insensitive because different tools disagree on hex case,
    // and constant-time because this is an integrity decision.
    crate::core::token::constant_time_eq(&expected.to_lowercase(), &actual.to_lowercase())
}

/// How many starts a freshly-installed version gets to prove itself
/// before the previous binary is put back (AR23).
///
/// Two, not one: the start that applies the update is followed by
/// exactly one start of the new binary, so a second start of the same
/// pending update means that one did not reach healthy. Three would
/// mean a crash-looping version keeps the service down longer for no
/// extra information.
pub const MAX_START_ATTEMPTS: u32 = 2;

/// What a self-update left behind for the next start to find.
///
/// Written before the restart and cleared once the new version has
/// proved itself, so this file existing at startup is exactly the
/// statement "an update is on probation" (AR23).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateState {
    pub from_version: String,
    pub to_version: String,
    /// Where the binary that was replaced was kept, so a revert is a
    /// rename rather than another download from a host that may be
    /// exactly what went wrong.
    pub previous_binary: String,
    /// How many times a process has started while this update was
    /// still unproven. Defaulted so a hand-written state file (a
    /// manual recovery, most likely) is still understood.
    #[serde(default)]
    pub attempts: u32,
}

/// What to do about a pending update at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartAction {
    /// No update is pending; start normally.
    Normal,
    /// A freshly-installed version is starting for the first time.
    /// Persist the returned state, start normally, and clear it once
    /// the service is actually serving.
    Probation(UpdateState),
    /// The new version has now failed to prove itself. Put the
    /// previous binary back, notify, and exit so the supervisor starts
    /// the restored one.
    Revert(UpdateState),
}

/// Decides what a starting process should do about a pending update,
/// counting this start as an attempt.
///
/// Deliberately pure: the interesting mistakes here — reverting on the
/// very start that is meant to succeed, or never reverting because the
/// counter is written after the crash instead of before — are logic
/// errors, and this way they are provable without a real crash loop.
pub fn decide_at_startup(state: Option<UpdateState>) -> StartAction {
    let Some(mut state) = state else {
        return StartAction::Normal;
    };

    state.attempts = state.attempts.saturating_add(1);

    if state.attempts >= MAX_START_ATTEMPTS {
        StartAction::Revert(state)
    } else {
        StartAction::Probation(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(attempts: u32) -> UpdateState {
        UpdateState {
            from_version: "0.1.0".to_string(),
            to_version: "0.2.0".to_string(),
            previous_binary: "/opt/almanac/almanac.prev".to_string(),
            attempts,
        }
    }

    fn v(major: u64, minor: u64, patch: u64) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }

    #[test]
    fn versions_parse_with_and_without_the_v_prefix() {
        assert_eq!(Version::parse("1.2.3").unwrap(), v(1, 2, 3));
        assert_eq!(Version::parse("v1.2.3").unwrap(), v(1, 2, 3));
        assert_eq!(Version::parse("  v0.1.0\n").unwrap(), v(0, 1, 0));
    }

    #[test]
    fn nonsense_is_refused_rather_than_guessed_at() {
        for bad in ["", "v", "1.2", "1.2.3.4", "1.2.x", "latest", "v1.2.-3"] {
            assert!(Version::parse(bad).is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn ten_is_newer_than_nine_which_string_comparison_would_get_wrong() {
        // The classic bug: "0.10.0" < "0.9.0" as text, so a lexical
        // updater would refuse every release after 0.9.
        assert!(should_update(v(0, 9, 0), v(0, 10, 0)));
        assert!(!should_update(v(0, 10, 0), v(0, 9, 0)));
    }

    #[test]
    fn only_a_strictly_newer_version_triggers_an_update() {
        assert!(should_update(v(1, 0, 0), v(1, 0, 1)));
        assert!(should_update(v(1, 0, 0), v(1, 1, 0)));
        assert!(should_update(v(1, 0, 0), v(2, 0, 0)));

        assert!(
            !should_update(v(1, 0, 0), v(1, 0, 0)),
            "an equal version would make it replace its binary on every poll"
        );
    }

    #[test]
    fn an_older_release_is_refused_so_a_downgrade_cannot_be_forced() {
        // A correctly-signed but old release with a known hole is a
        // real attack, not a hypothetical.
        assert!(!should_update(v(2, 0, 0), v(1, 9, 9)));
        assert!(!should_update(v(1, 1, 0), v(1, 0, 9)));
    }

    #[test]
    fn the_manifest_hash_is_found_by_filename() {
        let manifest = "\
abc123  almanac
def456  something-else
";
        assert_eq!(hash_for(manifest, "almanac").unwrap(), "abc123");
        assert_eq!(hash_for(manifest, "something-else").unwrap(), "def456");
    }

    #[test]
    fn binary_mode_manifests_are_understood_too() {
        assert_eq!(hash_for("abc123 *almanac\n", "almanac").unwrap(), "abc123");
    }

    #[test]
    fn a_missing_entry_is_an_error_not_a_skipped_check() {
        let err = hash_for("abc123  other\n", "almanac").unwrap_err();
        assert!(err.remedy().contains("do not install"));
    }

    #[test]
    fn an_empty_or_malformed_manifest_is_refused() {
        assert!(hash_for("", "almanac").is_err());
        assert!(hash_for("nonsense\n", "almanac").is_err());
    }

    #[test]
    fn hashes_compare_case_insensitively_but_still_have_to_match() {
        assert!(hash_matches("ABC123", "abc123"));
        assert!(hash_matches("abc123", "abc123"));
        assert!(!hash_matches("abc123", "abc124"));
        assert!(!hash_matches("abc123", "abc12"));
    }

    #[test]
    fn an_ordinary_start_with_nothing_pending_is_left_alone() {
        assert_eq!(decide_at_startup(None), StartAction::Normal);
    }

    #[test]
    fn the_first_start_after_an_update_runs_on_probation_not_reverted() {
        // Reverting here would undo every update the moment it was
        // installed — the new version has not had a chance to fail yet.
        match decide_at_startup(Some(pending(0))) {
            StartAction::Probation(state) => assert_eq!(state.attempts, 1),
            other => panic!("expected probation, got {other:?}"),
        }
    }

    #[test]
    fn a_second_start_with_the_update_still_unproven_reverts() {
        // The first start never cleared the state, so it never became
        // healthy: the new binary is broken in a way `--check` did not
        // catch.
        match decide_at_startup(Some(pending(1))) {
            StartAction::Revert(state) => {
                assert_eq!(state.attempts, 2);
                assert_eq!(state.previous_binary, "/opt/almanac/almanac.prev");
            }
            other => panic!("expected a revert, got {other:?}"),
        }
    }

    #[test]
    fn a_state_file_that_somehow_survived_more_starts_still_reverts() {
        // Rather than counting past the threshold and doing nothing.
        assert!(matches!(
            decide_at_startup(Some(pending(7))),
            StartAction::Revert(_)
        ));
        assert!(matches!(
            decide_at_startup(Some(pending(u32::MAX))),
            StartAction::Revert(_)
        ));
    }

    #[test]
    fn the_attempt_count_is_optional_so_a_hand_written_state_file_still_parses() {
        // Manual recovery is a documented path; a missing counter
        // must not make the file unreadable at the worst moment.
        let state: UpdateState = serde_json::from_str(
            r#"{"from_version":"0.1.0","to_version":"0.2.0",
                "previous_binary":"/opt/almanac/almanac.prev"}"#,
        )
        .unwrap();
        assert_eq!(state.attempts, 0);
    }

    #[test]
    fn the_state_file_round_trips() {
        let state = pending(1);
        let text = serde_json::to_string(&state).unwrap();
        assert_eq!(serde_json::from_str::<UpdateState>(&text).unwrap(), state);
    }
}

// ---------------------------------------------------------------
// Whether self-update should run here at all.
// ---------------------------------------------------------------

/// What `ALMANAC_SELF_UPDATE` was set to, if anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfUpdateSetting {
    /// Explicitly switched on — overrides everything, including the
    /// container default below.
    On,
    /// Explicitly switched off.
    Off,
    /// Not set: fall back to what the environment suggests.
    Unset,
}

/// Parses the setting, generously. Someone writing this into a
/// `docker-compose.yml` will type whichever of these comes to mind, and
/// a typo silently meaning "on" is the wrong way round for a switch
/// whose whole job is to stop the process replacing its own binary.
pub fn parse_self_update_setting(raw: Option<&str>) -> SelfUpdateSetting {
    match raw.map(str::trim) {
        None => SelfUpdateSetting::Unset,
        Some("") => SelfUpdateSetting::Unset,
        Some(v) if v.eq_ignore_ascii_case("off") => SelfUpdateSetting::Off,
        Some(v) if v.eq_ignore_ascii_case("false") => SelfUpdateSetting::Off,
        Some("0") => SelfUpdateSetting::Off,
        Some(v) if v.eq_ignore_ascii_case("no") => SelfUpdateSetting::Off,
        Some(v) if v.eq_ignore_ascii_case("on") => SelfUpdateSetting::On,
        Some(v) if v.eq_ignore_ascii_case("true") => SelfUpdateSetting::On,
        Some("1") => SelfUpdateSetting::On,
        Some(v) if v.eq_ignore_ascii_case("yes") => SelfUpdateSetting::On,
        // Anything else is a typo. Treat it as off: refusing to update
        // is recoverable by fixing the value, whereas a process that
        // rewrites its own binary because someone typed "offf" is not
        // what anyone meant.
        Some(_) => SelfUpdateSetting::Off,
    }
}

/// What the filesystem says about where we are running.
///
/// Gathered by the shell (these are all file reads) and judged here.
#[derive(Debug, Clone, Default)]
pub struct ContainerEvidence {
    /// `/.dockerenv` exists. Docker creates this in every container.
    pub dockerenv: bool,
    /// `/run/.containerenv` exists. Podman's equivalent.
    pub containerenv: bool,
    /// The contents of `/proc/1/cgroup`, empty if unreadable.
    pub pid1_cgroup: String,
}

/// Whether we are inside an image somebody else builds and ships.
///
/// **LXC deliberately does not count.** Almanac's own deployment is an
/// LXC container on Proxmox, where self-update is exactly what is
/// wanted — the container is a long-lived machine with a filesystem
/// that survives, not a rebuilt artifact. Docker and Podman are the
/// opposite: the binary comes from an image, a new version means a new
/// image, and a process that rewrites its own binary inside a container
/// loses that change the moment the container is recreated. Worse, it
/// diverges from the image while looking identical to it, which is the
/// kind of difference that costs an afternoon.
///
/// So the test is for OCI runtimes specifically, never for
/// "am I in a container", which would switch self-update off on the
/// very machine it was built for.
pub fn is_managed_image(evidence: &ContainerEvidence) -> bool {
    if evidence.dockerenv || evidence.containerenv {
        return true;
    }
    // cgroup v1 lines look like `1:name=systemd:/docker/<id>`; v2 gives
    // a single `0::/` line inside Docker, which carries no marker at
    // all — which is why the files above are checked first and this is
    // only a fallback for older hosts.
    evidence.pid1_cgroup.lines().any(|line| {
        ["/docker", "/docker-", "libpod", "containerd"]
            .iter()
            .any(|m| line.contains(m))
    })
}

/// The final answer, and the reason for it — the reason is logged, so a
/// hub that is not updating can always say why in one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfUpdateDecision {
    Run,
    OffByConfiguration,
    OffBecauseImageManaged,
}

pub fn decide_self_update(
    setting: SelfUpdateSetting,
    evidence: &ContainerEvidence,
) -> SelfUpdateDecision {
    match setting {
        // An explicit `on` wins even inside an image. Someone running a
        // container as a long-lived pet, with the data directory on a
        // volume, is allowed to say so.
        SelfUpdateSetting::On => SelfUpdateDecision::Run,
        SelfUpdateSetting::Off => SelfUpdateDecision::OffByConfiguration,
        SelfUpdateSetting::Unset if is_managed_image(evidence) => {
            SelfUpdateDecision::OffBecauseImageManaged
        }
        SelfUpdateSetting::Unset => SelfUpdateDecision::Run,
    }
}

#[cfg(test)]
mod self_update_policy_tests {
    use super::*;

    fn nothing() -> ContainerEvidence {
        ContainerEvidence::default()
    }

    #[test]
    fn an_unset_variable_on_an_ordinary_machine_leaves_self_update_on() {
        assert_eq!(
            decide_self_update(parse_self_update_setting(None), &nothing()),
            SelfUpdateDecision::Run
        );
    }

    #[test]
    fn docker_switches_it_off_without_anyone_configuring_anything() {
        let evidence = ContainerEvidence {
            dockerenv: true,
            ..Default::default()
        };
        assert_eq!(
            decide_self_update(parse_self_update_setting(None), &evidence),
            SelfUpdateDecision::OffBecauseImageManaged
        );
    }

    #[test]
    fn podman_counts_too() {
        let evidence = ContainerEvidence {
            containerenv: true,
            ..Default::default()
        };
        assert!(is_managed_image(&evidence));
    }

    #[test]
    fn an_lxc_container_still_updates_itself() {
        // The regression that would matter most: Almanac's own
        // deployment is an LXC container on Proxmox. A check for
        // "am I in a container" rather than "am I in an image" would
        // switch self-update off on the one machine it was built for,
        // and the symptom would be a version that quietly never moves.
        let evidence = ContainerEvidence {
            pid1_cgroup: "0::/\n11:name=systemd:/lxc/112\n10:devices:/lxc/112".to_string(),
            ..Default::default()
        };
        assert!(!is_managed_image(&evidence));
        assert_eq!(
            decide_self_update(parse_self_update_setting(None), &evidence),
            SelfUpdateDecision::Run
        );
    }

    #[test]
    fn a_cgroup_v1_docker_line_is_recognised_on_older_hosts() {
        let evidence = ContainerEvidence {
            pid1_cgroup: "1:name=systemd:/docker/3f2a9c1e".to_string(),
            ..Default::default()
        };
        assert!(is_managed_image(&evidence));
    }

    #[test]
    fn an_explicit_on_wins_even_inside_an_image() {
        let evidence = ContainerEvidence {
            dockerenv: true,
            ..Default::default()
        };
        assert_eq!(
            decide_self_update(parse_self_update_setting(Some("on")), &evidence),
            SelfUpdateDecision::Run
        );
    }

    #[test]
    fn an_explicit_off_wins_everywhere() {
        assert_eq!(
            decide_self_update(parse_self_update_setting(Some("off")), &nothing()),
            SelfUpdateDecision::OffByConfiguration
        );
    }

    #[test]
    fn the_spellings_someone_would_actually_type_all_work() {
        for off in ["off", "OFF", "false", "0", "no", " off "] {
            assert_eq!(
                parse_self_update_setting(Some(off)),
                SelfUpdateSetting::Off,
                "{off:?} should mean off"
            );
        }
        for on in ["on", "ON", "true", "1", "yes"] {
            assert_eq!(
                parse_self_update_setting(Some(on)),
                SelfUpdateSetting::On,
                "{on:?} should mean on"
            );
        }
    }

    #[test]
    fn a_typo_means_off_rather_than_on() {
        // Which way a typo falls matters: "offf" meaning on would let a
        // process rewrite its own binary because of a slipped finger.
        assert_eq!(
            parse_self_update_setting(Some("offf")),
            SelfUpdateSetting::Off
        );
        assert_eq!(
            parse_self_update_setting(Some("maybe")),
            SelfUpdateSetting::Off
        );
    }

    #[test]
    fn an_empty_value_is_the_same_as_not_setting_it() {
        // `ALMANAC_SELF_UPDATE=` in a compose file or an env file is a
        // leftover, not a decision.
        assert_eq!(
            parse_self_update_setting(Some("")),
            SelfUpdateSetting::Unset
        );
        assert_eq!(
            parse_self_update_setting(Some("   ")),
            SelfUpdateSetting::Unset
        );
    }
}
