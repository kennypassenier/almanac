//! The pure half of self-update (M10): deciding whether a release is
//! newer, and verifying a manifest before anything is trusted.
//!
//! Downloading, replacing the binary and restarting are `shell`'s job
//! (AR13). What lives here is everything that can be got wrong without
//! touching the network — which is where the security-relevant
//! mistakes are.

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
