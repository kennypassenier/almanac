//! The Google Calendar v3 HTTP client (K1) — together with
//! `shell::auth`, the only place allowed to talk to Google. Every call
//! goes through `send_with_retry`, which classifies failures via
//! `core::retry::is_transient` and retries transient ones with
//! exponential backoff (M3/AR9) before giving up; the token itself is
//! obtained through `TokenManager`, which handles its own refresh
//! (AR18) transparently.

use std::sync::Arc;

use backoff::ExponentialBackoff;
use backoff::future::retry;
use reqwest::{Client, RequestBuilder, Response};
use serde::Deserialize;

use crate::core::calendar::{EventListResponse, GoogleEvent};
use crate::core::error::AlmanacError;
use crate::core::retry::is_transient;
use crate::shell::auth::TokenManager;

const CALENDAR_EVENTS_BASE: &str = "https://www.googleapis.com/calendar/v3/calendars";

pub struct GoogleCalendarClient {
    http: Client,
    tokens: Arc<TokenManager>,
}

/// Google's documented error response shape — only the `reason` field
/// `core::retry::is_transient` needs to disambiguate an overloaded
/// HTTP 403 is extracted; everything else is ignored.
#[derive(Debug, Deserialize)]
struct GoogleErrorBody {
    error: GoogleErrorDetail,
}

#[derive(Debug, Deserialize)]
struct GoogleErrorDetail {
    #[serde(default)]
    errors: Vec<GoogleErrorItem>,
}

#[derive(Debug, Deserialize)]
struct GoogleErrorItem {
    #[serde(default)]
    reason: Option<String>,
}

fn extract_reason(body: &str) -> Option<String> {
    serde_json::from_str::<GoogleErrorBody>(body)
        .ok()?
        .error
        .errors
        .into_iter()
        .next()?
        .reason
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
    async fn send_with_retry<F>(&self, build: F) -> Result<Response, AlmanacError>
    where
        F: Fn(&Client, &str) -> RequestBuilder,
    {
        retry(ExponentialBackoff::default(), || async {
            let token = self
                .tokens
                .token()
                .await
                .map_err(backoff::Error::Permanent)?;

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_reason_google_puts_on_a_rate_limited_403() {
        let body = r#"{
            "error": {
                "errors": [{"domain": "usageLimits", "reason": "rateLimitExceeded", "message": "Rate Limit Exceeded"}],
                "code": 403,
                "message": "Rate Limit Exceeded"
            }
        }"#;
        assert_eq!(extract_reason(body), Some("rateLimitExceeded".to_string()));
    }

    #[test]
    fn returns_none_for_a_body_that_is_not_googles_error_shape() {
        assert_eq!(extract_reason("not json"), None);
        assert_eq!(extract_reason("{}"), None);
        assert_eq!(extract_reason(r#"{"error": {"errors": []}}"#), None);
    }
}
