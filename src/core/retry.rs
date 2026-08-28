//! Classifies a Google API failure as transient (worth retrying, per
//! M3) or permanent (retrying won't help). Kept as pure logic — no
//! network call needed to test it — specifically because the
//! architecture-critic's Phase 4 objection was concrete: a wrong
//! classification either hammers a permanently-broken call pointlessly
//! or gives up on a passing blip too early. AR14 requires this table
//! to be exhaustively tested against every documented status code, not
//! just a couple of examples.
//!
//! Google overloads HTTP 403 for both transient conditions (rate/quota
//! limits) and permanent ones (no permission) — the status code alone
//! cannot tell them apart, so this also inspects the parsed error
//! `reason` Google's Calendar/OAuth2 APIs put in the response body,
//! when one was present.

/// Classifies a Google API error response as transient or permanent.
pub fn is_transient(status: u16, reason: Option<&str>) -> bool {
    match status {
        429 => true,
        500..=599 => true,
        403 => matches!(
            reason,
            Some("rateLimitExceeded") | Some("userRateLimitExceeded") | Some("quotaExceeded")
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_and_quota_limited_403s_are_transient() {
        assert!(is_transient(403, Some("rateLimitExceeded")));
        assert!(is_transient(403, Some("userRateLimitExceeded")));
        assert!(is_transient(403, Some("quotaExceeded")));
    }

    #[test]
    fn permission_denied_403_is_permanent_even_though_same_status_code() {
        assert!(!is_transient(403, Some("forbidden")));
        assert!(!is_transient(403, Some("insufficientPermissions")));
    }

    #[test]
    fn a_403_with_no_reason_body_defaults_to_permanent() {
        // Fail closed (standing rule 12): an unparseable/absent reason
        // must not be assumed transient, or a genuine permission error
        // would retry forever.
        assert!(!is_transient(403, None));
    }

    #[test]
    fn too_many_requests_is_transient() {
        assert!(is_transient(429, None));
    }

    #[test]
    fn every_5xx_is_transient() {
        for status in [500, 502, 503, 504, 599] {
            assert!(
                is_transient(status, None),
                "expected {status} to be transient"
            );
        }
    }

    #[test]
    fn client_errors_other_than_403_and_429_are_permanent() {
        for status in [400, 401, 404, 409, 410, 422] {
            assert!(
                !is_transient(status, None),
                "expected {status} to be permanent"
            );
        }
    }

    #[test]
    fn success_and_redirect_codes_are_permanent_not_that_it_matters() {
        // Never actually consulted on a 2xx/3xx (callers only classify
        // failures), but the function must not panic or misbehave if
        // it is.
        assert!(!is_transient(200, None));
        assert!(!is_transient(304, None));
    }
}
