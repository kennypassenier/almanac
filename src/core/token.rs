//! Per-source bearer-token authentication (K6/AR17). Profiles store
//! only a SHA-256 hash of each source's token, never the plaintext, so
//! the profile files stay safe to keep in git. Comparison is constant
//! time: a byte-by-byte early-return would leak how much of a guessed
//! token was correct through response timing.

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Hashes a plaintext bearer token into the lowercase hex form stored
/// as a profile's `token_hash`.
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether `presented` is the token behind `expected_hash`. Constant
/// time with respect to the hash contents; a malformed stored hash
/// simply never matches (fail closed, standing rule 12).
pub fn verify_token(presented: &str, expected_hash: &str) -> bool {
    let actual = hash_token(presented);
    // Same length always (both are 64 hex chars from SHA-256) unless
    // the stored hash is malformed, in which case this is false and no
    // comparison happens at all.
    if actual.len() != expected_hash.len() {
        return false;
    }
    actual.as_bytes().ct_eq(expected_hash.as_bytes()).into()
}

/// Extracts the token from an `Authorization: Bearer <token>` header
/// value. Returns `None` for any other scheme or a malformed value.
pub fn parse_bearer(header_value: &str) -> Option<&str> {
    let rest = header_value.strip_prefix("Bearer ")?;
    let rest = rest.trim();
    if rest.is_empty() { None } else { Some(rest) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashing_matches_the_known_sha256_of_a_known_input() {
        // Pinned against an externally-computed value (`printf
        // 'almanac' | sha256sum`), not against our own output: if the
        // hash algorithm ever silently changed, every deployed
        // profile's token_hash would stop matching, and a test that
        // only compared our output to itself would not notice.
        assert_eq!(
            hash_token("almanac"),
            "409e0a333bd76871853626d35492d87e5226d3a0e4788bca07261ad3436b5b93"
        );
    }

    #[test]
    fn a_correct_token_verifies() {
        let hash = hash_token("s3cret-home-assistant-token");
        assert!(verify_token("s3cret-home-assistant-token", &hash));
    }

    #[test]
    fn a_wrong_token_does_not_verify() {
        let hash = hash_token("s3cret-home-assistant-token");
        assert!(!verify_token("s3cret-home-assistant-toke", &hash));
        assert!(!verify_token("", &hash));
        assert!(!verify_token("completely different", &hash));
    }

    #[test]
    fn a_malformed_stored_hash_never_matches() {
        assert!(!verify_token("anything", ""));
        assert!(!verify_token("anything", "not-a-hash"));
        assert!(!verify_token("anything", "deadbeef"));
    }

    #[test]
    fn different_tokens_hash_differently() {
        assert_ne!(hash_token("token-a"), hash_token("token-b"));
    }

    #[test]
    fn parses_a_well_formed_bearer_header() {
        assert_eq!(parse_bearer("Bearer abc123"), Some("abc123"));
    }

    #[test]
    fn rejects_other_schemes_and_malformed_values() {
        assert_eq!(parse_bearer("Basic abc123"), None);
        assert_eq!(parse_bearer("bearer abc123"), None); // case matters
        assert_eq!(parse_bearer("Bearer "), None);
        assert_eq!(parse_bearer("abc123"), None);
        assert_eq!(parse_bearer(""), None);
    }
}
