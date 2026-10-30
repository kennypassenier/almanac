//! The Google Calendar v3 HTTP client (K1) — together with
//! `shell::auth`, the only place allowed to talk to Google. Every call
//! goes through `send_with_retry`, which classifies failures via
//! `core::retry::is_transient` and retries transient ones with
//! exponential backoff (M3/AR9) before giving up; the token itself is
//! obtained through `TokenManager`, which handles its own refresh
//! (AR18) transparently — including retrying its own transient
//! failures within this same loop (see `send_with_retry`).

use std::sync::Arc;

use backon::{ExponentialBuilder, Retryable};
use reqwest::{Client, RequestBuilder, Response};
use serde::{Deserialize, Serialize};

use crate::core::calendar::GoogleEvent;
use crate::core::error::AlmanacError;
use crate::core::retry::{extract_reason, is_transient};
use crate::shell::auth::TokenManager;

mod calendars;
mod events;
mod paging;

/// Also the calendars *collection* endpoint (`POST` here creates a
/// calendar) — `{CALENDAR_EVENTS_BASE}/{calendar_id}/events` is the
/// events collection under one specific calendar.
const CALENDAR_EVENTS_BASE: &str = "https://www.googleapis.com/calendar/v3/calendars";
/// Google keeps "which calendars can this account see" on a different
/// path from "do something to a calendar", so it cannot be derived from
/// the events base at call time.
const CALENDAR_LIST_URL: &str = "https://www.googleapis.com/calendar/v3/users/me/calendarList";

/// How long one call may spend retrying before it gives up.
///
/// A long retry budget is wrong in both directions here. A synchronous
/// caller (K8 — a Claude session waiting on `/sync`) would hang for
/// minutes, and the worker would sit inside one delivery while every
/// other pending entry waited behind it.
///
/// A minute is the right shape because retrying here is only meant to
/// absorb blips. Anything longer is what the journal is for: the entry
/// stays pending, the worker backs off (AR26), and it goes out when
/// Google comes back — durably, and without holding anything open.
///
/// The ladder keeps the shape the unmaintained `backoff` crate used to
/// give (RUSTSEC-2025-0012): half a second, growing by 1.5, jittered.
/// `backon`'s budget counts the time spent sleeping between attempts,
/// not the attempts themselves, so a call whose every request times
/// out can run past the minute by the requests' own timeouts.
fn in_call_backoff() -> ExponentialBuilder {
    ExponentialBuilder::default()
        .with_min_delay(std::time::Duration::from_millis(500))
        .with_factor(1.5)
        .with_max_delay(std::time::Duration::from_secs(60))
        .with_jitter()
        .without_max_times()
        .with_total_delay(Some(std::time::Duration::from_secs(60)))
}

pub struct GoogleCalendarClient {
    http: Client,
    tokens: Arc<TokenManager>,
    /// Where the Calendar API lives. A field rather than a constant so
    /// a test can point the client at a local stub (T0).
    ///
    /// Without this the retry loop, the create-vs-update decision and
    /// the per-profile calendar routing were unreachable for anything
    /// but a live test against Kenny's real calendar — which is why
    /// all three went untested, and why a misclassified connection
    /// failure survived in the retry loop until an audit read it.
    /// Production never sets it; the default is the real endpoint.
    base_url: String,
    /// Where "which calendars can this account see" lives. Derived
    /// from `base_url` in tests so a stub serves both from one port.
    calendar_list_url: String,
}

#[derive(Serialize)]
struct NewCalendar<'a> {
    summary: &'a str,
}

#[derive(Deserialize)]
struct CalendarResource {
    id: String,
}

impl GoogleCalendarClient {
    pub fn new(http: Client, tokens: Arc<TokenManager>) -> Self {
        Self {
            http,
            tokens,
            base_url: CALENDAR_EVENTS_BASE.to_string(),
            calendar_list_url: CALENDAR_LIST_URL.to_string(),
        }
    }

    /// The same client pointed at somewhere else — a local stub in
    /// tests. Never used in production.
    pub fn with_base_url(
        http: Client,
        tokens: Arc<TokenManager>,
        base_url: impl Into<String>,
    ) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        Self {
            calendar_list_url: format!("{base_url}/calendarList"),
            http,
            tokens,
            base_url,
        }
    }

    /// Runs `build` (given the HTTP client and a fresh bearer token) —
    /// a synchronous step, since assembling a `RequestBuilder` never
    /// itself performs I/O — and retries transient failures with
    /// exponential backoff. `build` must be callable more than once:
    /// the retry loop rebuilds and resends the whole request on each
    /// attempt, never reusing a partially-consumed one.
    ///
    /// A token-refresh failure is retried in this same loop rather
    /// than bailing immediately: `TokenManager::token()` itself only
    /// classifies genuinely permanent problems (a malformed key, a
    /// clock error) as such; a passing network blip while talking to
    /// Google's token endpoint is `transient`, and must recover here
    /// without waiting for some later, unrelated request to try again
    /// — Kenny should never need to restart the process by hand for a
    /// problem that fixes itself in a few seconds.
    async fn send_with_retry<F>(&self, build: F) -> Result<Response, AlmanacError>
    where
        F: Fn(&Client, &str) -> RequestBuilder,
    {
        let attempt = || async {
            let token = self.tokens.token().await?;

            let response = build(&self.http, &token).send().await.map_err(|e| {
                // A connection that never completed has no status code,
                // so `is_transient` never sees it — it has to be
                // classified here. Transient, because that is what it
                // is: a DNS blip, a dropped TLS handshake, a router
                // rebooting. Calling it permanent meant a two-second
                // hiccup surfaced to a synchronous caller as a failure
                // while `shell::auth` retried the very same class of
                // failure against the token endpoint. The two now
                // agree.
                AlmanacError::GoogleApi {
                    message: format!("request to the Calendar API failed: {e}"),
                    remedy: "transient network failure — retrying automatically".to_string(),
                    transient: true,
                }
            })?;

            let status = response.status();
            if status.is_success() {
                return Ok(response);
            }

            let body = response.text().await.unwrap_or_default();
            let reason = extract_reason(&body);
            let transient = is_transient(status.as_u16(), reason.as_deref());
            let err = AlmanacError::GoogleApi {
                message: format!("Calendar API returned HTTP {status}: {body}"),
                remedy: if transient {
                    "transient Google API failure — retrying automatically".to_string()
                } else {
                    "check the request payload, event id and calendar id".to_string()
                },
                transient,
            };

            Err(err)
        };

        attempt
            .retry(in_call_backoff())
            .when(AlmanacError::is_transient)
            .await
    }

    async fn parse_event(response: Response) -> Result<GoogleEvent, AlmanacError> {
        response.json().await.map_err(|e| AlmanacError::GoogleApi {
            message: format!("failed to parse the Calendar API's event response: {e}"),
            remedy: "check Google's Calendar API response format hasn't changed".to_string(),
            transient: false,
        })
    }
}

#[cfg(test)]
mod tests;
