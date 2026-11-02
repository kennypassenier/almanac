//! Where the Calendar API lives: Google's address, or a stub on this
//! machine (Habits integration test, 2026-11-02). `ALMANAC_GOOGLE_API_BASE`
//! points a release binary at a fake Calendar API for an end-to-end check
//! without real Google; anything but a loopback address is refused, so the
//! setting can never send events or the service account's bearer token to
//! another host.

use crate::core::error::AlmanacError;

/// The environment variable that overrides the Calendar API root.
pub const GOOGLE_API_BASE_ENV: &str = "ALMANAC_GOOGLE_API_BASE";

/// Google's own root: `{root}/calendars` and `{root}/users/me/calendarList`.
pub const GOOGLE_API_ROOT: &str = "https://www.googleapis.com/calendar/v3";

/// The Calendar API root to use: Google's when `value` is unset or empty,
/// else `value` without a trailing slash, provided it is http(s) on
/// 127.0.0.1, `localhost` or `[::1]`.
pub fn api_root(value: Option<&str>) -> Result<String, AlmanacError> {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(GOOGLE_API_ROOT.to_string());
    };
    let root = value.trim_end_matches('/');
    if is_loopback_url(root) {
        Ok(root.to_string())
    } else {
        Err(AlmanacError::Config {
            message: format!("{GOOGLE_API_BASE_ENV} is \"{value}\", which is not on this machine"),
            remedy: format!(
                "point {GOOGLE_API_BASE_ENV} at a stub on 127.0.0.1, localhost or [::1] \
                 (such as http://127.0.0.1:9100/calendar/v3), or unset it to use Google"
            ),
        })
    }
}

fn is_loopback_url(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
    else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.contains('@') {
        return false;
    }
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next().map(|h| format!("[{h}]"))
    } else {
        authority.split(':').next().map(str::to_string)
    };
    matches!(host.as_deref(), Some("127.0.0.1" | "localhost" | "[::1]"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_or_empty_means_google() {
        assert_eq!(api_root(None).unwrap(), GOOGLE_API_ROOT);
        assert_eq!(api_root(Some("  ")).unwrap(), GOOGLE_API_ROOT);
    }

    #[test]
    fn a_loopback_stub_is_taken_without_its_trailing_slash() {
        for url in [
            "http://127.0.0.1:9100/calendar/v3/",
            "http://localhost:9100",
            "https://[::1]:8443/v3",
        ] {
            assert_eq!(api_root(Some(url)).unwrap(), url.trim_end_matches('/'));
        }
    }

    #[test]
    fn any_other_host_is_refused_with_a_remedy() {
        for url in [
            "https://www.googleapis.com/calendar/v3",
            "http://10.10.10.12:9100",
            "http://127.0.0.1.evil.example/v3",
            "http://user@127.0.0.1@evil.example/",
            "ftp://127.0.0.1/",
            "127.0.0.1:9100",
        ] {
            let err = api_root(Some(url)).expect_err(url);
            assert!(err.remedy().contains("127.0.0.1"), "{url}: {err}");
        }
    }
}
