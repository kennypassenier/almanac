//! The Google Calendar v3 HTTP client (K1) — together with
//! `shell::auth`, the only place allowed to talk to Google. Every call
//! goes through `send_with_retry`, which classifies failures via
//! `core::retry::is_transient` and retries transient ones with
//! exponential backoff (M3/AR9) before giving up; the token itself is
//! obtained through `TokenManager`, which handles its own refresh
//! (AR18) transparently — including retrying its own transient
//! failures within this same loop (see `send_with_retry`).

use std::sync::Arc;

use backoff::ExponentialBackoff;
use backoff::future::retry;
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

pub struct GoogleCalendarClient {
    http: Client,
    tokens: Arc<TokenManager>,
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
        Self { http, tokens }
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
        retry(ExponentialBackoff::default(), || async {
            let token = self.tokens.token().await.map_err(|e| {
                if e.is_transient() {
                    backoff::Error::transient(e)
                } else {
                    backoff::Error::Permanent(e)
                }
            })?;

            let response = build(&self.http, &token).send().await.map_err(|e| {
                backoff::Error::Permanent(AlmanacError::GoogleApi {
                    message: format!("request to the Calendar API failed: {e}"),
                    remedy: "check network connectivity to www.googleapis.com".to_string(),
                    transient: false,
                })
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

            if transient {
                Err(backoff::Error::transient(err))
            } else {
                Err(backoff::Error::Permanent(err))
            }
        })
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
        let url = format!("{CALENDAR_EVENTS_BASE}/{calendar_id}/events");
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
        let url = format!("{CALENDAR_EVENTS_BASE}/{calendar_id}/events/{event_id}");
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
        let url = format!("{CALENDAR_EVENTS_BASE}/{calendar_id}/events/{event_id}");
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
        let url = format!("{CALENDAR_EVENTS_BASE}/{calendar_id}/events/{event_id}");
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
        let url = format!("{CALENDAR_EVENTS_BASE}/{calendar_id}/events");
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

        Ok(list.items.into_iter().flatten().next())
    }

    /// Creates a new secondary calendar owned by the authenticated
    /// service account and returns its id. A small forward-borrow from
    /// K3 (multiple calendars, landing properly in L2): used here as a
    /// one-off setup step (see `examples/create_test_calendar.rs`) so
    /// standing rule 14's scratch calendar can be provisioned entirely
    /// through the API — no manual Google Calendar UI step, no sharing
    /// step, since a calendar the service account creates is already
    /// its own.
    pub async fn create_calendar(&self, summary: &str) -> Result<String, AlmanacError> {
        let body = NewCalendar { summary };
        let response = self
            .send_with_retry(|http, token| {
                http.post(CALENDAR_EVENTS_BASE)
                    .bearer_auth(token)
                    .json(&body)
            })
            .await?;

        let parsed: CalendarResource =
            response.json().await.map_err(|e| AlmanacError::GoogleApi {
                message: format!("failed to parse the created calendar response: {e}"),
                remedy: "check Google's Calendar API response format hasn't changed".to_string(),
                transient: false,
            })?;

        Ok(parsed.id)
    }
}
