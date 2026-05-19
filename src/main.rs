//! cal-stacean — Universal Google Calendar API Gateway
//!
//! Step 1: Configuration layer  — parse `config.toml`, initialise tracing.
//! Step 2: Authentication layer — Service Account JWT (RS256) via discrete
//!         `.env` variables loaded from Doppler.
//! Step 3: Calendar client      — [`GoogleCalendarClient`] with connectivity probe.
//! Step 4: CRUD operations      — `create_event` and `get_event` with a live
//!         round-trip validation test.
//! Step 5: Full lifecycle         — `update_event`, `delete_event`, and
//!         `find_event_by_property` with an end-to-end lifecycle test.
//! Step 6: HTTP server            — Axum web server exposing the CRUD core via
//!         REST endpoints at `/api/v1/events/*`.
//!
//! Execution order in `main`:
//!   1.  Load `.env` into the process environment (must be first).
//!   2.  Read and deserialize `config.toml`.
//!   3.  Initialise the `tracing` subscriber.
//!   4.  Build a shared `Arc<reqwest::Client>`.
//!   5.  Load Service Account credentials from environment variables.
//!   6.  Obtain a Google OAuth2 bearer token.
//!   7.  Instantiate [`GoogleCalendarClient`].
//!   8.  Wrap the client in [`AppState`] and share it via `Arc`.
//!   9.  Build the Axum [`Router`] with all API route handlers.
//!   10. Bind a [`tokio::net::TcpListener`] on `0.0.0.0:8080`.
//!   11. Serve the application continuously until interrupted.

use std::collections::HashMap;
use std::fs;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::post,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::Level;
use tracing_subscriber::EnvFilter;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// OAuth2 scope that grants full read/write access to Google Calendar.
const CALENDAR_SCOPE: &str = "https://www.googleapis.com/auth/calendar";

/// Requested JWT lifetime in seconds.  Google enforces a maximum of 3600 (1 h).
const JWT_LIFETIME_SECS: u64 = 3600;

/// `grant_type` value required by RFC 7523 / the Google token endpoint when
/// presenting a self-signed JWT instead of an authorisation code.
const JWT_BEARER_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";

/// Base URL for the Google Calendar v3 events collection.
/// Full event URLs are constructed as:
///   `{CALENDAR_EVENTS_BASE}/{calendarId}/events`
///   `{CALENDAR_EVENTS_BASE}/{calendarId}/events/{eventId}`
const CALENDAR_EVENTS_BASE: &str = "https://www.googleapis.com/calendar/v3/calendars";

// ---------------------------------------------------------------------------
// Google Calendar event types
// ---------------------------------------------------------------------------

/// Date/time boundary for a calendar event (start or end).
///
/// Google's API represents this as a JSON object with camelCase keys.
/// The `rename_all` attribute handles the snake_case → camelCase mapping so
/// the Rust field names remain idiomatic.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct EventDateTime {
    /// RFC3339 timestamp, e.g. `"2026-05-19T09:00:00+00:00"`.
    date_time: String,

    /// IANA time zone name, e.g. `"UTC"` or `"America/New_York"`.
    time_zone: String,
}

/// Extended properties attached to a calendar event.
///
/// Google supports two buckets — `private` (visible only to the creating
/// application) and `shared` (visible to all Calendar applications).  Only
/// `private` is used by this gateway; `shared` is omitted intentionally.
#[derive(Debug, Serialize, Deserialize, Clone)]
struct ExtendedProperties {
    /// Application-private key/value metadata.  Keys and values are
    /// arbitrary UTF-8 strings; Google imposes a 1 024-byte limit per value.
    private: HashMap<String, String>,
}

/// A Google Calendar Event resource.
///
/// Mirrors the subset of fields defined in the official v3 API schema:
/// <https://developers.google.com/calendar/api/v3/reference/events>
///
/// `rename_all = "camelCase"` converts every snake_case Rust field name to its
/// camelCase JSON counterpart (e.g. `color_id` → `"colorId"`).
///
/// `Option` fields are skipped during serialization when `None` so the JSON
/// body sent to Google never contains explicit `null` values for absent fields.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct GoogleEvent {
    /// Unique event identifier assigned by Google.
    /// Set to `None` when constructing a new event; populated in the response
    /// after a successful `create_event` call.
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,

    /// One-line title displayed in calendar views.
    summary: String,

    /// Optional multi-line body text shown in the event detail pane.
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,

    /// Optional free-text location string (address, room name, URL, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<String>,

    /// Google Calendar colour ID (`"1"` – `"11"`).  When absent, the
    /// calendar's default colour is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    color_id: Option<String>,

    /// Inclusive start date/time of the event.
    start: EventDateTime,

    /// Exclusive end date/time of the event.
    end: EventDateTime,

    /// Application-private metadata attached to this event.
    #[serde(skip_serializing_if = "Option::is_none")]
    extended_properties: Option<ExtendedProperties>,
}

/// Wrapper around the `items` array returned by the Google Calendar v3
/// events list endpoint.
///
/// The `items` key is absent (not just empty) when no events match the
/// query, so the field is typed as `Option<Vec<GoogleEvent>>` to satisfy
/// the deserializer in both cases.
#[derive(Debug, Deserialize)]
struct EventListResponse {
    /// Zero or more event resources matching the list query.
    items: Option<Vec<GoogleEvent>>,
}

// ---------------------------------------------------------------------------
// Configuration types
// ---------------------------------------------------------------------------

/// Top-level application configuration deserialized from `config.toml`.
/// Every field maps to a TOML key of the same name.
#[derive(Debug, Deserialize)]
struct Config {
    /// Google Calendar ID used when the caller does not specify one
    /// (e.g. `"primary"` or a full address such as `"user@example.com"`).
    default_calendar_id: String,

    /// Google Calendar colour ID applied to events when the caller omits one.
    /// Google's API accepts integer strings `"1"` – `"11"`.
    default_color_id: String,

    /// Minimum tracing level.  Accepted values (case-insensitive):
    /// `"trace"`, `"debug"`, `"info"`, `"warn"`, `"error"`.
    log_level: String,

    /// Human-readable colour names mapped to Google Calendar colour IDs.
    standard_colors: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// Service Account credential types
// ---------------------------------------------------------------------------

/// Service Account credentials loaded from discrete environment variables.
///
/// Doppler automatically creates these variable names when a Google Service
/// Account JSON key file is imported into a project, preserving the original
/// field names from the JSON (e.g. `client_email` becomes `CLIENT_EMAIL`).
struct ServiceAccountCredentials {
    /// Service account email address; becomes the JWT `iss` (issuer) claim.
    /// Loaded from `CLIENT_EMAIL`.
    client_email: String,

    /// RSA private key in PEM format (PKCS#8, header `-----BEGIN PRIVATE KEY-----`).
    /// Loaded from `PRIVATE_KEY`.
    ///
    /// Secret managers often encode newlines as the two-character sequence `\n`.
    /// These are normalized to real newlines before the bytes are passed to
    /// the PEM parser.
    private_key: String,

    /// Google OAuth2 token endpoint URL.
    /// Loaded from `TOKEN_URI` (always `"https://oauth2.googleapis.com/token"`).
    token_url: String,
}

// ---------------------------------------------------------------------------
// JWT and token types
// ---------------------------------------------------------------------------

/// Claims embedded in the JWT posted to the Google OAuth2 token endpoint.
///
/// Field names are defined by Google and must not be changed.
/// Reference: <https://developers.google.com/identity/protocols/oauth2/service-account>
#[derive(Debug, Serialize)]
struct GoogleClaims {
    /// Issuer — the service account email address.
    iss: String,

    /// OAuth2 scope(s) requested (space-separated list for multiple scopes).
    scope: String,

    /// Audience — the URL of the token endpoint that will validate the JWT.
    aud: String,

    /// Expiry time as a Unix timestamp (seconds since the epoch).
    exp: u64,

    /// Issued-at time as a Unix timestamp.
    iat: u64,
}

/// Successful response body from the Google OAuth2 token endpoint.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    /// Short-lived bearer token to be sent in `Authorization: Bearer <token>`
    /// headers on subsequent Google API requests.
    access_token: String,

    /// Token lifetime in seconds from the moment of issuance (typically 3600).
    expires_in: u64,
}

// ---------------------------------------------------------------------------
// Google Calendar client
// ---------------------------------------------------------------------------

/// High-level Google Calendar API client.
///
/// Bundles a shared HTTP connection pool ([`Arc<Client>`]), the active OAuth2
/// bearer token, and the application configuration.  Wrapping the client in
/// [`Arc`] allows it to be cloned cheaply across async tasks without
/// duplicating the underlying connection pool.
struct GoogleCalendarClient {
    /// Shared, thread-safe HTTP client.
    http_client: Arc<Client>,

    /// Active OAuth2 bearer access token.  Obtained during startup and valid
    /// for the duration returned by the token endpoint (typically 1 hour).
    access_token: String,

    /// Application configuration parsed from `config.toml`.
    /// Provides `default_calendar_id` and `default_color_id` to all API methods.
    config: Config,
}

impl GoogleCalendarClient {
    /// Construct a new [`GoogleCalendarClient`].
    fn new(http_client: Arc<Client>, access_token: String, config: Config) -> Self {
        Self {
            http_client,
            access_token,
            config,
        }
    }

    /// Create a new event in the calendar identified by `config.default_calendar_id`.
    ///
    /// Serializes `event` to JSON and POSTs it to:
    /// `https://www.googleapis.com/calendar/v3/calendars/{calendarId}/events`
    ///
    /// Google assigns a unique `id` to the new event and returns the full
    /// event resource in the response body.  The caller should inspect
    /// `created.id` to obtain the assigned identifier.
    ///
    /// # Errors
    /// Returns an error if the HTTP request fails, if Google returns a non-2xx
    /// status (the response body is included in the error message), or if the
    /// response JSON cannot be deserialized into a [`GoogleEvent`].
    pub async fn create_event(
        &self,
        event: &GoogleEvent,
    ) -> Result<GoogleEvent, Box<dyn std::error::Error>> {
        let url = format!(
            "{}/{}/events",
            CALENDAR_EVENTS_BASE, self.config.default_calendar_id
        );

        tracing::debug!(calendar_id = %self.config.default_calendar_id, "creating calendar event");

        let response = self
            .http_client
            .post(&url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.access_token),
            )
            // reqwest's `.json()` serializes the body and sets Content-Type
            // to application/json automatically.
            .json(event)
            .send()
            .await
            .map_err(|e| format!("POST {url} failed: {e}"))?;

        let status = response.status();

        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable response body>".to_string());

            return Err(format!("create_event returned HTTP {status}: {body}").into());
        }

        let created_event: GoogleEvent = response
            .json()
            .await
            .map_err(|e| format!("failed to deserialize created event response: {e}"))?;

        Ok(created_event)
    }

    /// Fetch a single event by its ID from `config.default_calendar_id`.
    ///
    /// Sends a GET request to:
    /// `https://www.googleapis.com/calendar/v3/calendars/{calendarId}/events/{eventId}`
    ///
    /// # Errors
    /// Returns an error if the HTTP request fails, if Google returns a non-2xx
    /// status, or if the response JSON cannot be deserialized into a [`GoogleEvent`].
    #[allow(dead_code)]
    pub async fn get_event(
        &self,
        event_id: &str,
    ) -> Result<GoogleEvent, Box<dyn std::error::Error>> {
        let url = format!(
            "{}/{}/events/{}",
            CALENDAR_EVENTS_BASE, self.config.default_calendar_id, event_id
        );

        tracing::debug!(event_id = %event_id, "fetching calendar event");

        let response = self
            .http_client
            .get(&url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.access_token),
            )
            .send()
            .await
            .map_err(|e| format!("GET {url} failed: {e}"))?;

        let status = response.status();

        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable response body>".to_string());

            return Err(format!("get_event returned HTTP {status}: {body}").into());
        }

        let event: GoogleEvent = response
            .json()
            .await
            .map_err(|e| format!("failed to deserialize fetched event response: {e}"))?;

        Ok(event)
    }

    /// Replace an existing event with the provided [`GoogleEvent`] value.
    ///
    /// Sends a PUT request to:
    /// `https://www.googleapis.com/calendar/v3/calendars/{calendarId}/events/{eventId}`
    ///
    /// A PUT overwrites the entire event resource.  The caller must therefore
    /// supply all fields that should be retained, not only the changed ones.
    /// The `event` parameter is typically a clone of the previously fetched
    /// event with the desired fields mutated.
    ///
    /// Returns the updated event resource as confirmed by Google.
    ///
    /// # Errors
    /// Returns an error if the HTTP request fails, if Google returns a non-2xx
    /// status, or if the response JSON cannot be deserialized.
    pub async fn update_event(
        &self,
        event_id: &str,
        event: &GoogleEvent,
    ) -> Result<GoogleEvent, Box<dyn std::error::Error>> {
        let url = format!(
            "{}/{}/events/{}",
            CALENDAR_EVENTS_BASE, self.config.default_calendar_id, event_id
        );

        tracing::debug!(event_id = %event_id, "updating calendar event");

        let response = self
            .http_client
            .put(&url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.access_token),
            )
            .json(event)
            .send()
            .await
            .map_err(|e| format!("PUT {url} failed: {e}"))?;

        let status = response.status();

        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable response body>".to_string());

            return Err(format!("update_event returned HTTP {status}: {body}").into());
        }

        let updated_event: GoogleEvent = response
            .json()
            .await
            .map_err(|e| format!("failed to deserialize updated event response: {e}"))?;

        Ok(updated_event)
    }

    /// Permanently delete an event from the calendar.
    ///
    /// Sends a DELETE request to:
    /// `https://www.googleapis.com/calendar/v3/calendars/{calendarId}/events/{eventId}`
    ///
    /// Google returns HTTP 204 No Content on success; there is no response body.
    /// Deleting an already-deleted event returns HTTP 410 Gone, which is also
    /// treated as an error by this method.
    ///
    /// # Errors
    /// Returns an error if the HTTP request fails or if Google returns any
    /// non-2xx status (404 Not Found, 410 Gone, etc.).
    pub async fn delete_event(&self, event_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "{}/{}/events/{}",
            CALENDAR_EVENTS_BASE, self.config.default_calendar_id, event_id
        );

        tracing::debug!(event_id = %event_id, "deleting calendar event");

        let response = self
            .http_client
            .delete(&url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.access_token),
            )
            .send()
            .await
            .map_err(|e| format!("DELETE {url} failed: {e}"))?;

        let status = response.status();

        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable response body>".to_string());

            return Err(format!("delete_event returned HTTP {status}: {body}").into());
        }

        // HTTP 204 No Content — no body to read.
        Ok(())
    }

    /// Search the calendar for events whose private extended properties contain
    /// the supplied key/value pair.
    ///
    /// Queries the events list endpoint with the `privateExtendedProperty`
    /// filter parameter:
    /// `https://www.googleapis.com/calendar/v3/calendars/{calendarId}/events`
    ///   `?privateExtendedProperty={key}={value}`
    ///
    /// Google returns only events that were created by the same application
    /// (same OAuth2 client) and have a matching private property.
    ///
    /// Returns the **first** matching event wrapped in `Some`, or `None` if no
    /// events match.  The returned event includes its Google-assigned `id`.
    ///
    /// # Errors
    /// Returns an error if the HTTP request fails, if Google returns a non-2xx
    /// status, or if the response JSON cannot be deserialized.
    #[allow(dead_code)]
    pub async fn find_event_by_property(
        &self,
        key: &str,
        value: &str,
    ) -> Result<Option<GoogleEvent>, Box<dyn std::error::Error>> {
        let url = format!(
            "{}/{}/events",
            CALENDAR_EVENTS_BASE, self.config.default_calendar_id
        );

        // The `privateExtendedProperty` filter takes the form `key=value`
        // as a single query parameter value.  reqwest percent-encodes it.
        let filter = format!("{key}={value}");

        tracing::debug!(
            filter = %filter,
            "searching events by private extended property",
        );

        let response = self
            .http_client
            .get(&url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.access_token),
            )
            .query(&[("privateExtendedProperty", filter.as_str())])
            .send()
            .await
            .map_err(|e| format!("GET {url} (property search) failed: {e}"))?;

        let status = response.status();

        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable response body>".to_string());

            return Err(format!("find_event_by_property returned HTTP {status}: {body}").into());
        }

        let list: EventListResponse = response
            .json()
            .await
            .map_err(|e| format!("failed to deserialize event list response: {e}"))?;

        // Return the first event from the list, or None if the result set is
        // empty.  `into_iter().flatten()` handles both the None case (no
        // `items` key) and the Some(vec) case uniformly.
        let first = list.items.into_iter().flatten().next();

        Ok(first)
    }
}

// ---------------------------------------------------------------------------
// Application state
// ---------------------------------------------------------------------------

/// Shared application state accessible from every Axum request handler.
///
/// Constructed once during application startup and then wrapped in [`Arc`] so
/// that Axum can clone the cheap pointer into each spawned request task
/// without copying the underlying data.
struct AppState {
    /// Authenticated Google Calendar API client.
    ///
    /// Encapsulates the active OAuth2 bearer token, the shared HTTP connection
    /// pool, and the loaded application configuration.
    calendar_client: GoogleCalendarClient,
}

// ---------------------------------------------------------------------------
// Configuration loading
// ---------------------------------------------------------------------------

/// Read `config.toml` from the current working directory and deserialize it
/// into a [`Config`] value.
///
/// # Errors
/// Returns an error if the file cannot be read or if the TOML does not
/// conform to the expected schema.
fn load_config() -> Result<Config, Box<dyn std::error::Error>> {
    let raw = fs::read_to_string("config.toml")
        .map_err(|e| format!("failed to read config.toml: {e}"))?;

    let config: Config =
        toml::from_str(&raw).map_err(|e| format!("failed to parse config.toml: {e}"))?;

    Ok(config)
}

// ---------------------------------------------------------------------------
// Tracing initialisation
// ---------------------------------------------------------------------------

/// Initialise the global `tracing` subscriber using the log level from the
/// application configuration.
///
/// The `RUST_LOG` environment variable takes precedence over the config file
/// value, allowing operators to adjust verbosity at runtime.
///
/// # Errors
/// Returns an error if `log_level` is not a recognised tracing level string,
/// or if the subscriber could not be registered as the global default.
fn init_tracing(log_level: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Validate the string up-front so the error message names the bad value
    // rather than failing deep inside the filter parser.
    Level::from_str(log_level).map_err(|_| {
        format!(
            "invalid log_level '{log_level}' in config.toml; \
             expected one of: trace, debug, info, warn, error"
        )
    })?;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_file(true)
        .with_line_number(true)
        .try_init()
        .map_err(|e| format!("failed to initialise tracing subscriber: {e}"))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Credential loading
// ---------------------------------------------------------------------------

/// Load Service Account credentials from discrete environment variables.
///
/// The three expected variables are created automatically by Doppler when a
/// Google Service Account JSON key file is imported into a Doppler project:
///
/// | Variable       | Service Account JSON field | Content                        |
/// |----------------|----------------------------|--------------------------------|
/// | `CLIENT_EMAIL` | `client_email`             | Service account email address  |
/// | `PRIVATE_KEY`  | `private_key`              | RSA private key in PEM format  |
/// | `TOKEN_URI`    | `token_uri`                | Google OAuth2 token endpoint   |
///
/// # Errors
/// Returns a descriptive error naming whichever variable is absent.
fn load_service_account_credentials()
-> Result<ServiceAccountCredentials, Box<dyn std::error::Error>> {
    // Variable names match the keys Doppler creates when a Google Service
    // Account JSON key file is imported directly into a Doppler project.
    let client_email = std::env::var("CLIENT_EMAIL").map_err(|_| {
        "environment variable CLIENT_EMAIL is not set; \
         ensure the Google Service Account JSON has been imported into Doppler"
    })?;

    let private_key = std::env::var("PRIVATE_KEY").map_err(|_| {
        "environment variable PRIVATE_KEY is not set; \
         ensure the Google Service Account JSON has been imported into Doppler"
    })?;

    let token_url = std::env::var("TOKEN_URI").map_err(|_| {
        "environment variable TOKEN_URI is not set; \
         expected value: https://oauth2.googleapis.com/token"
    })?;

    Ok(ServiceAccountCredentials {
        client_email,
        private_key,
        token_url,
    })
}

// ---------------------------------------------------------------------------
// Google OAuth2 authentication
// ---------------------------------------------------------------------------

/// Obtain a short-lived Google OAuth2 bearer access token using the provided
/// Service Account credentials.
///
/// ### Flow
/// 1. Compute the current Unix timestamp for the JWT `iat` and `exp` claims.
/// 2. Build [`GoogleClaims`] and sign them as an RS256 JWT.
/// 3. POST the JWT to the token endpoint using the `jwt-bearer` grant type.
/// 4. Deserialize the response and return the `access_token` string.
///
/// The returned token is valid for [`TokenResponse::expires_in`] seconds
/// (typically 3600 / 1 hour).
///
/// # Errors
/// Returns an error for: signing failure, network failure, non-2xx token
/// endpoint response, or JSON deserialization failure.
async fn get_google_auth_token(
    client: &Client,
    credentials: &ServiceAccountCredentials,
) -> Result<String, Box<dyn std::error::Error>> {
    // --- Build JWT claims --------------------------------------------------
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("system clock is set before the Unix epoch: {e}"))?
        .as_secs();

    let claims = GoogleClaims {
        iss: credentials.client_email.clone(),
        scope: CALENDAR_SCOPE.to_string(),
        // The audience claim must equal the token endpoint URL exactly.
        aud: credentials.token_url.clone(),
        iat: now_secs,
        exp: now_secs + JWT_LIFETIME_SECS,
    };

    // --- Sign the JWT (RS256) ----------------------------------------------
    // Secret managers such as Doppler store multi-line PEM strings with
    // literal two-character `\n` sequences rather than real newline bytes.
    // Expand them so that the PEM parser receives structurally valid input.
    let pem_normalized = credentials.private_key.replace("\\n", "\n");

    let encoding_key = EncodingKey::from_rsa_pem(pem_normalized.as_bytes()).map_err(|e| {
        format!("failed to load RSA private key from GOOGLE_SERVICE_ACCOUNT_KEY: {e}")
    })?;

    let jwt = jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &encoding_key)
        .map_err(|e| format!("failed to sign JWT: {e}"))?;

    tracing::debug!("JWT signed with RS256 successfully");

    // --- Exchange JWT for an access token ----------------------------------
    let response = client
        .post(&credentials.token_url)
        // Google's token endpoint requires application/x-www-form-urlencoded,
        // not JSON.  reqwest's `.form()` sets the correct Content-Type.
        .form(&[
            ("grant_type", JWT_BEARER_GRANT_TYPE),
            ("assertion", jwt.as_str()),
        ])
        .send()
        .await
        .map_err(|e| {
            format!(
                "HTTP POST to token endpoint '{}' failed: {e}",
                credentials.token_url
            )
        })?;

    let status = response.status();

    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<unreadable response body>".to_string());

        return Err(format!("token endpoint returned HTTP {status}: {body}").into());
    }

    let token_response: TokenResponse = response
        .json()
        .await
        .map_err(|e| format!("failed to deserialize token endpoint response: {e}"))?;

    tracing::debug!(
        expires_in = token_response.expires_in,
        "access token obtained from Google token endpoint",
    );

    Ok(token_response.access_token)
}

// ---------------------------------------------------------------------------
// HTTP request handlers
// ---------------------------------------------------------------------------

/// Convenience type alias for all HTTP handler return types.
///
/// Both arms carry an HTTP status code and a JSON body, which lets the `?`
/// operator propagate calendar-client errors as structured JSON responses
/// without any additional conversion boilerplate.
type HandlerResult = Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)>;

/// Handler for `POST /api/v1/events/create`.
///
/// Deserializes the request body as a [`GoogleEvent`], delegates to
/// [`GoogleCalendarClient::create_event`], and returns the Google-assigned
/// event ID in a JSON success envelope.
///
/// # Responses
/// - `201 Created`                — `{ "status": "success", "event_id": "..." }`
/// - `500 Internal Server Error`  — `{ "status": "error",   "message": "..." }`
async fn create_event_handler(
    State(state): State<Arc<AppState>>,
    Json(event): Json<GoogleEvent>,
) -> HandlerResult {
    tracing::info!(
        summary = %event.summary,
        "POST /api/v1/events/create received",
    );

    // Delegate to the calendar client; map any error into a JSON error envelope.
    let created = state
        .calendar_client
        .create_event(&event)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "create_event failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "status": "error", "message": e.to_string() })),
            )
        })?;

    // Google guarantees an `id` field on every successful creation response;
    // treat its absence as an unexpected server-side error.
    let event_id = created.id.ok_or_else(|| {
        let msg = "Google API did not return an event ID in the creation response";
        tracing::error!(msg);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "status": "error", "message": msg })),
        )
    })?;

    tracing::info!(
        event_id = %event_id,
        summary  = %created.summary,
        "event created successfully",
    );

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "status":   "success",
            "event_id": event_id,
        })),
    ))
}

/// Handler for `POST /api/v1/events/update/:id`.
///
/// Accepts the target event ID as the `:id` URL path segment and a complete
/// [`GoogleEvent`] resource as the JSON request body, then delegates to
/// [`GoogleCalendarClient::update_event`] (HTTP PUT — full resource replacement).
///
/// # Responses
/// - `200 OK`                     — `{ "status": "success", "event_id": "...", "updated_summary": "..." }`
/// - `500 Internal Server Error`  — `{ "status": "error",   "message": "..." }`
async fn update_event_handler(
    State(state): State<Arc<AppState>>,
    Path(event_id): Path<String>,
    Json(event): Json<GoogleEvent>,
) -> HandlerResult {
    tracing::info!(
        event_id = %event_id,
        summary  = %event.summary,
        "POST /api/v1/events/update/:id received",
    );

    // Delegate to the calendar client; map any error into a JSON error envelope.
    let updated = state
        .calendar_client
        .update_event(&event_id, &event)
        .await
        .map_err(|e| {
            tracing::error!(event_id = %event_id, error = %e, "update_event failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "status": "error", "message": e.to_string() })),
            )
        })?;

    tracing::info!(
        event_id        = %event_id,
        updated_summary = %updated.summary,
        "event updated successfully",
    );

    Ok((
        StatusCode::OK,
        Json(json!({
            "status":          "success",
            "event_id":        event_id,
            "updated_summary": updated.summary,
        })),
    ))
}

/// Handler for `POST /api/v1/events/delete/:id`.
///
/// Accepts the target event ID as the `:id` URL path segment and delegates to
/// [`GoogleCalendarClient::delete_event`].  No request body is required.
///
/// # Responses
/// - `200 OK`                     — `{ "status": "success", "deleted_event_id": "..." }`
/// - `500 Internal Server Error`  — `{ "status": "error",   "message": "..." }`
async fn delete_event_handler(
    State(state): State<Arc<AppState>>,
    Path(event_id): Path<String>,
) -> HandlerResult {
    tracing::info!(
        event_id = %event_id,
        "POST /api/v1/events/delete/:id received",
    );

    // Delegate to the calendar client; map any error into a JSON error envelope.
    state
        .calendar_client
        .delete_event(&event_id)
        .await
        .map_err(|e| {
            tracing::error!(event_id = %event_id, error = %e, "delete_event failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "status": "error", "message": e.to_string() })),
            )
        })?;

    tracing::info!(
        event_id = %event_id,
        "event deleted successfully",
    );

    Ok((
        StatusCode::OK,
        Json(json!({
            "status":           "success",
            "deleted_event_id": event_id,
        })),
    ))
}

// ---------------------------------------------------------------------------
// Async entry point
// ---------------------------------------------------------------------------

/// Application entry point.
///
/// Bootstraps all application layers in strict sequence and then hands control
/// to the Axum HTTP server, which runs indefinitely.  Any startup error
/// propagates back to the OS as a non-zero exit code via the `?` operator.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1 — Load the `.env` file FIRST, before any std::env::var() calls, so
    //     that all secrets injected by Doppler (or a manually maintained .env)
    //     are available to every subsequent step.
    //     A missing .env file is silently ignored; only an unreadable file
    //     (e.g. wrong permissions) will surface through the tracing output
    //     once the subscriber is initialised.
    dotenvy::dotenv().ok();

    // 2 — Parse `config.toml`.
    let config = load_config()?;

    // 3 — Initialise structured logging now that we have the log level.
    init_tracing(&config.log_level)?;

    tracing::info!(
        default_calendar_id  = %config.default_calendar_id,
        default_color_id     = %config.default_color_id,
        log_level            = %config.log_level,
        standard_color_count = config.standard_colors.len(),
        "configuration loaded successfully",
    );

    tracing::debug!(colors = ?config.standard_colors, "standard colour map");

    // 4 — Build a shared HTTP client and wrap it in Arc so it can be passed
    //     to both the auth function and the calendar client without cloning
    //     the underlying connection pool.
    let http_client = Arc::new(
        Client::builder()
            .build()
            .map_err(|e| format!("failed to build HTTP client: {e}"))?,
    );

    // 5 — Load Service Account credentials from environment variables.
    //     These are expected to have been injected via the `.env` file loaded
    //     in step 1 (or pre-existing in the shell environment).
    let credentials = load_service_account_credentials()?;

    tracing::debug!(
        client_email = %credentials.client_email,
        token_url    = %credentials.token_url,
        "service account credentials loaded from environment",
    );

    // 6 — Obtain a Google OAuth2 bearer access token.
    let access_token = get_google_auth_token(&http_client, &credentials).await?;

    tracing::info!("Google OAuth2 bearer token obtained successfully");

    // 7 — Instantiate the Calendar client.
    let calendar_client = GoogleCalendarClient::new(Arc::clone(&http_client), access_token, config);

    // 8 — Wrap the authenticated calendar client in AppState and share it
    //     across all Axum request handlers via Arc.  Each spawned request task
    //     receives a clone of the Arc (a cheap pointer copy), not a clone of
    //     the underlying data.
    let app_state = Arc::new(AppState { calendar_client });

    // 9 — Build the Axum router and register all API route handlers.
    //     Routes are scoped under /api/v1/events/ for versioned, namespaced
    //     API organisation.  `.with_state()` attaches the shared state so
    //     every handler can extract it with the State<Arc<AppState>> extractor.
    let router: Router = Router::new()
        .route("/api/v1/events/create", post(create_event_handler))
        .route("/api/v1/events/update/:id", post(update_event_handler))
        .route("/api/v1/events/delete/:id", post(delete_event_handler))
        .with_state(app_state);

    // 10 — Bind the TCP listener to all network interfaces on port 8080.
    //      0.0.0.0 means the server accepts connections on every available
    //      interface: loopback, LAN, container bridge, etc.
    let bind_address = "0.0.0.0:8080";
    let listener = tokio::net::TcpListener::bind(bind_address)
        .await
        .map_err(|e| format!("failed to bind TCP listener on {bind_address}: {e}"))?;

    tracing::info!(
        address = bind_address,
        "cal-stacean HTTP server listening — waiting for requests",
    );

    // 11 — Hand the listener and router to Axum and serve indefinitely.
    //      `axum::serve` drives the Tokio accept loop; it only returns when
    //      an unrecoverable server-level error occurs.
    axum::serve(listener, router)
        .await
        .map_err(|e| format!("axum server terminated unexpectedly: {e}"))?;

    Ok(())
}
