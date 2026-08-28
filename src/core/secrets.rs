//! Encryption for the app-token store (M12/AR17). Tokens are kept
//! encrypted rather than hashed so the dashboard can reproduce a
//! working copy-paste command at any time — a hash could only ever
//! show a token once, at creation.
//!
//! XChaCha20-Poly1305 with a random 24-byte nonce per record, matching
//! what Latch already uses so both projects rest on the same primitive.
//! The nonce is stored in front of the ciphertext; it is not secret,
//! only unique. Authentication is what makes tampering with the store
//! fail loudly instead of silently yielding a different token.

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use crate::core::error::AlmanacError;

/// Raw key length. The operator supplies it as 64 hex characters.
pub const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 24;

/// Parses the hex-encoded encryption key an operator configures.
///
/// A wrong-length or non-hex key is a configuration error the process
/// must refuse to start with, not something to paper over: a silently
/// derived or truncated key would decrypt nothing that was written
/// before it.
pub fn parse_key(hex_key: &str) -> Result<[u8; KEY_BYTES], AlmanacError> {
    let bytes = hex::decode(hex_key.trim()).map_err(|e| AlmanacError::Config {
        message: format!("the encryption key is not valid hex: {e}"),
        remedy: format!(
            "set a {}-character hex key; generate one with `openssl rand -hex {KEY_BYTES}`",
            KEY_BYTES * 2
        ),
    })?;

    bytes.try_into().map_err(|_| AlmanacError::Config {
        message: format!("the encryption key must be exactly {KEY_BYTES} bytes"),
        remedy: format!(
            "set a {}-character hex key; generate one with `openssl rand -hex {KEY_BYTES}`",
            KEY_BYTES * 2
        ),
    })
}

fn cipher(key: &[u8; KEY_BYTES]) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new(key.into())
}

/// Encrypts `plaintext`, returning hex of `nonce || ciphertext`.
pub fn seal(key: &[u8; KEY_BYTES], plaintext: &str) -> Result<String, AlmanacError> {
    let mut nonce_bytes = [0u8; NONCE_BYTES];
    getrandom::fill(&mut nonce_bytes).map_err(|e| AlmanacError::Config {
        message: format!("the OS refused to provide randomness: {e}"),
        remedy: "this is a system-level problem; check the entropy source".to_string(),
    })?;
    let nonce = XNonce::from(nonce_bytes);

    let ciphertext = cipher(key)
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|_| AlmanacError::Config {
            message: "failed to encrypt a token".to_string(),
            remedy: "this is a bug in almanac".to_string(),
        })?;

    let mut envelope = nonce_bytes.to_vec();
    envelope.extend_from_slice(&ciphertext);
    Ok(hex::encode(envelope))
}

/// Reverses [`seal`]. Fails on a wrong key, a truncated record, or any
/// tampering — all of which are real problems that must surface rather
/// than degrade into a wrong token.
pub fn open(key: &[u8; KEY_BYTES], sealed_hex: &str) -> Result<String, AlmanacError> {
    let envelope = hex::decode(sealed_hex.trim()).map_err(|e| AlmanacError::Config {
        message: format!("a stored token is not valid hex: {e}"),
        remedy: "the token store is damaged; restore it from backup or re-issue the token"
            .to_string(),
    })?;

    if envelope.len() <= NONCE_BYTES {
        return Err(AlmanacError::Config {
            message: "a stored token is too short to contain a nonce and ciphertext".to_string(),
            remedy: "the token store is damaged; restore it from backup or re-issue the token"
                .to_string(),
        });
    }

    let (nonce_bytes, ciphertext) = envelope.split_at(NONCE_BYTES);
    let nonce = XNonce::try_from(nonce_bytes).map_err(|_| AlmanacError::Config {
        message: "a stored token has a malformed nonce".to_string(),
        remedy: "the token store is damaged; restore it from backup or re-issue the token"
            .to_string(),
    })?;

    let plaintext = cipher(key)
        .decrypt(&nonce, ciphertext)
        .map_err(|_| AlmanacError::Config {
            message: "a stored token could not be decrypted".to_string(),
            remedy: "the encryption key does not match the one the store was written with, or the \
                     store was modified; check ALMANAC_SECRET_KEY before re-issuing tokens"
                .to_string(),
        })?;

    String::from_utf8(plaintext).map_err(|e| AlmanacError::Config {
        message: format!("a decrypted token is not valid UTF-8: {e}"),
        remedy: "the token store is damaged; re-issue the token".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; KEY_BYTES] {
        [7u8; KEY_BYTES]
    }

    #[test]
    fn a_sealed_token_opens_back_to_itself() {
        let key = test_key();
        let sealed = seal(&key, "s3cret-token").unwrap();
        assert_eq!(open(&key, &sealed).unwrap(), "s3cret-token");
    }

    #[test]
    fn the_ciphertext_never_contains_the_plaintext() {
        let sealed = seal(&test_key(), "s3cret-token").unwrap();
        assert!(!sealed.contains("s3cret"));
    }

    #[test]
    fn sealing_the_same_value_twice_gives_different_ciphertext() {
        // A fresh nonce per record: without it, two sources holding the
        // same token would be visibly identical in the store.
        let key = test_key();
        assert_ne!(seal(&key, "same").unwrap(), seal(&key, "same").unwrap());
    }

    #[test]
    fn the_wrong_key_fails_loudly_rather_than_returning_something_else() {
        let sealed = seal(&test_key(), "s3cret-token").unwrap();
        let err = open(&[9u8; KEY_BYTES], &sealed).unwrap_err();
        assert!(err.remedy().contains("ALMANAC_SECRET_KEY"));
    }

    #[test]
    fn tampering_with_a_record_is_detected() {
        // The whole point of an authenticated cipher: a flipped byte in
        // the store must not silently decrypt to a different token.
        let key = test_key();
        let sealed = seal(&key, "s3cret-token").unwrap();
        let mut bytes = hex::decode(&sealed).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        assert!(open(&key, &hex::encode(bytes)).is_err());
    }

    #[test]
    fn a_truncated_record_is_rejected_not_panicked_on() {
        let err = open(&test_key(), "abcd").unwrap_err();
        assert!(err.to_string().contains("too short"));
    }

    #[test]
    fn a_non_hex_record_is_rejected() {
        assert!(open(&test_key(), "not hex at all").is_err());
    }

    #[test]
    fn a_valid_hex_key_parses() {
        let key = parse_key(&"ab".repeat(KEY_BYTES)).unwrap();
        assert_eq!(key.len(), KEY_BYTES);
    }

    #[test]
    fn a_short_key_is_rejected_with_the_command_that_makes_a_good_one() {
        let err = parse_key("abcd").unwrap_err();
        assert!(err.remedy().contains("openssl rand -hex 32"));
    }

    #[test]
    fn a_non_hex_key_is_rejected() {
        let err = parse_key("zz".repeat(KEY_BYTES).as_str()).unwrap_err();
        assert!(err.to_string().contains("not valid hex"));
    }

    #[test]
    fn surrounding_whitespace_in_a_key_is_tolerated() {
        // Pasting a key out of a terminal routinely brings a newline;
        // refusing that would be a needless configuration trap.
        let key = parse_key(&format!("  {}\n", "ab".repeat(KEY_BYTES))).unwrap();
        assert_eq!(key.len(), KEY_BYTES);
    }
}
