//! The HTTP side every Google call shares: one client with timeouts, and
//! ids turned into single URL path segments.

use reqwest::Client;

/// Longest wait for a connection to Google, and for one whole request.
/// Without them a half-open connection never returns: the worker is
/// serial, so one hung call would hold every pending entry behind it and
/// keep a stop waiting until systemd kills the process.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// A calendar or event id as one URL path segment. Calendar ids carry
/// `@` and, for Google's own calendars, `#` (`en.be#holiday@group.v.calendar.google.com`),
/// which would end the path; anything outside the unreserved set and `@`
/// is percent-encoded, so no id can reach another API path either.
pub(super) fn segment(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~@".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The HTTP client every Google call (token exchange included) shares.
pub fn http_client() -> Client {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .expect("a reqwest client with timeouts always builds")
}
