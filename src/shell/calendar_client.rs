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

use crate::core::calendar::{EventListResponse, GoogleEvent};
use crate::core::error::AlmanacError;
use crate::core::retry::{extract_reason, is_transient};
use crate::shell::auth::TokenManager;

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

    pub async fn create_event(
        &self,
        calendar_id: &str,
        event: &GoogleEvent,
    ) -> Result<GoogleEvent, AlmanacError> {
        let url = format!("{}/{calendar_id}/events", self.base_url);
        let response = self
            .send_with_retry(|http, token| http.post(&url).bearer_auth(token).json(event))
            .await?;
        Self::parse_event(response).await
    }

    pub async fn get_event(
        &self,
        calendar_id: &str,
        event_id: &str,
    ) -> Result<GoogleEvent, AlmanacError> {
        let url = format!("{}/{calendar_id}/events/{event_id}", self.base_url);
        let response = self
            .send_with_retry(|http, token| http.get(&url).bearer_auth(token))
            .await?;
        Self::parse_event(response).await
    }

    /// Full resource replacement (Google's PUT semantics): the caller
    /// must resend every field it wants kept, not only what changed.
    pub async fn update_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        event: &GoogleEvent,
    ) -> Result<GoogleEvent, AlmanacError> {
        let url = format!("{}/{calendar_id}/events/{event_id}", self.base_url);
        let response = self
            .send_with_retry(|http, token| http.put(&url).bearer_auth(token).json(event))
            .await?;
        Self::parse_event(response).await
    }

    pub async fn delete_event(
        &self,
        calendar_id: &str,
        event_id: &str,
    ) -> Result<(), AlmanacError> {
        let url = format!("{}/{calendar_id}/events/{event_id}", self.base_url);
        self.send_with_retry(|http, token| http.delete(&url).bearer_auth(token))
            .await?;
        Ok(())
    }

    /// Finds an event by a private extended-property key/value pair —
    /// the mechanism K2's upsert lookup and AR15's `source_id` tagging
    /// depend on. Returns the first match, or `None`.
    pub async fn find_event_by_property(
        &self,
        calendar_id: &str,
        key: &str,
        value: &str,
    ) -> Result<Option<GoogleEvent>, AlmanacError> {
        Ok(self
            .list_events_by_property(calendar_id, key, value)
            .await?
            .into_iter()
            .next())
    }

    /// Every event carrying the given private extended property.
    /// `find_event_by_property` is the upsert path and only needs the
    /// first; this exists so a test can assert how many events a
    /// redelivery actually left behind, rather than inferring it.
    pub async fn list_events_by_property(
        &self,
        calendar_id: &str,
        key: &str,
        value: &str,
    ) -> Result<Vec<GoogleEvent>, AlmanacError> {
        let url = format!("{}/{calendar_id}/events", self.base_url);
        let filter = format!("{key}={value}");
        let response = self
            .send_with_retry(|http, token| {
                http.get(&url)
                    .bearer_auth(token)
                    .query(&[("privateExtendedProperty", filter.as_str())])
            })
            .await?;

        let list: EventListResponse =
            response.json().await.map_err(|e| AlmanacError::GoogleApi {
                message: format!("failed to parse the Calendar API's event list response: {e}"),
                remedy: "check Google's Calendar API response format hasn't changed".to_string(),
                transient: false,
            })?;

        Ok(list.items.unwrap_or_default())
    }

    /// Every event that starts in `[time_min, time_max)` (RFC 3339), repeats
    /// expanded, oldest first. Read-only: the History page's week agenda
    /// filters these to the ones carrying Almanac's own property.
    pub async fn list_events_between(
        &self,
        calendar_id: &str,
        time_min: &str,
        time_max: &str,
    ) -> Result<Vec<GoogleEvent>, AlmanacError> {
        let url = format!("{}/{calendar_id}/events", self.base_url);
        let response = self
            .send_with_retry(|http, token| {
                http.get(&url).bearer_auth(token).query(&[
                    ("timeMin", time_min),
                    ("timeMax", time_max),
                    ("singleEvents", "true"),
                    ("orderBy", "startTime"),
                    ("maxResults", "2500"),
                ])
            })
            .await?;

        let list: EventListResponse =
            response.json().await.map_err(|e| AlmanacError::GoogleApi {
                message: format!("failed to parse the Calendar API's event list response: {e}"),
                remedy: "check Google's Calendar API response format hasn't changed".to_string(),
                transient: false,
            })?;

        Ok(list.items.unwrap_or_default())
    }

    /// Creates a new secondary calendar owned by the authenticated
    /// service account and returns its id. A small forward-borrow from
    /// K3 (multiple calendars, landing properly in L2): used here as a
    /// one-off setup step (see `examples/create_test_calendar.rs`) so
    /// standing rule 14's scratch calendar can be provisioned entirely
    /// through the API — no manual Google Calendar UI step, no sharing
    /// step, since a calendar the service account creates is already
    /// its own.
    /// Creates a calendar and immediately gives `share_with` ownership
    /// of it.
    ///
    /// The two steps are one method on purpose. A calendar this service
    /// account creates is owned by the service account and invisible to
    /// every other human being, and that is easy not to notice — the
    /// `almanac-test` calendar from L1 sat that way for months while
    /// the live tests wrote into it, and the first two real calendars
    /// were created the same way an hour before this was written.
    /// Making sharing a separate call is how that keeps happening, so
    /// the caller has to name an owner to create anything at all.
    pub async fn create_calendar(
        &self,
        summary: &str,
        share_with: &str,
    ) -> Result<String, AlmanacError> {
        let body = NewCalendar { summary };
        let response = self
            .send_with_retry(|http, token| http.post(&self.base_url).bearer_auth(token).json(&body))
            .await?;

        let parsed: CalendarResource =
            response.json().await.map_err(|e| AlmanacError::GoogleApi {
                message: format!("failed to parse the created calendar response: {e}"),
                remedy: "check Google's Calendar API response format hasn't changed".to_string(),
                transient: false,
            })?;

        self.share_calendar(&parsed.id, share_with).await?;
        Ok(parsed.id)
    }

    /// Every calendar this account can see, as `(id, display name)`,
    /// sorted by name.
    ///
    /// The dashboard offers these in a dropdown (K21): a calendar id is
    /// an opaque string nobody types on purpose, and having to go and
    /// find one before adding a source was most of the chore.
    pub async fn list_calendars(&self) -> Result<Vec<(String, String)>, AlmanacError> {
        let response = self
            .send_with_retry(|http, token| http.get(&self.calendar_list_url).bearer_auth(token))
            .await?;

        let parsed: serde_json::Value =
            response.json().await.map_err(|e| AlmanacError::GoogleApi {
                message: format!("failed to parse the calendar list: {e}"),
                remedy: "check Google's Calendar API response format hasn't changed".to_string(),
                transient: false,
            })?;

        let mut calendars: Vec<(String, String)> = parsed["items"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .filter_map(|c| {
                Some((
                    c["id"].as_str()?.to_string(),
                    c["summary"].as_str()?.to_string(),
                ))
            })
            .collect();
        calendars.sort_by(|a, b| a.1.cmp(&b.1));
        Ok(calendars)
    }

    /// Finds a calendar this account can see by its display name, or
    /// `None`.
    pub async fn find_calendar_by_summary(
        &self,
        summary: &str,
    ) -> Result<Option<String>, AlmanacError> {
        Ok(self
            .list_calendars()
            .await?
            .into_iter()
            .find(|(_, name)| name == summary)
            .map(|(id, _)| id))
    }

    /// The id of the calendar called `summary`, creating and sharing it
    /// if this account cannot see one (K21).
    ///
    /// Matching on the name before creating is what stops a second
    /// "Almanac · Huishouden" appearing every time someone adds a
    /// source to it — and a duplicate calendar is close to invisible:
    /// events land, nothing errors, and half of them are on a calendar
    /// nobody has open.
    pub async fn ensure_calendar(
        &self,
        summary: &str,
        share_with: &str,
    ) -> Result<(String, bool), AlmanacError> {
        if let Some(id) = self.find_calendar_by_summary(summary).await? {
            return Ok((id, false));
        }
        let id = self.create_calendar(summary, share_with).await?;
        Ok((id, true))
    }

    /// Deletes a calendar and everything on it (K24).
    ///
    /// Irreversible at Google's end — this is not "remove it from my
    /// list", it is the calendar and every event ever put on it. The
    /// dashboard only offers it for a calendar no source writes to, and
    /// that guard is the reason this is safe to expose at all.
    pub async fn delete_calendar(&self, calendar_id: &str) -> Result<(), AlmanacError> {
        let url = format!("{}/{calendar_id}", self.base_url);
        self.send_with_retry(|http, token| http.delete(&url).bearer_auth(token))
            .await?;
        Ok(())
    }

    /// Gives `user` ownership of a calendar. Idempotent at Google's end:
    /// re-granting what someone already has is not an error.
    ///
    /// Owner rather than writer, so losing the service account never
    /// strands a calendar somebody depends on — the grantee can rename,
    /// share onward and delete it without us.
    pub async fn share_calendar(&self, calendar_id: &str, user: &str) -> Result<(), AlmanacError> {
        let url = format!("{}/{calendar_id}/acl", self.base_url);
        let rule = serde_json::json!({
            "role": "owner",
            "scope": {"type": "user", "value": user}
        });

        self.send_with_retry(|http, token| http.post(&url).bearer_auth(token).json(&rule))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
