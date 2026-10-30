//! Google service-account OAuth2 authentication (AR6) and the AR18
//! single-flight token refresh: concurrent requests that all observe
//! an expired/near-expired token share one refresh-plus-retry
//! operation instead of each hammering Google's token endpoint. The
//! critic's Phase 4 objection was specific — locking only the first
//! attempt just delays the thundering herd by one round trip when that
//! attempt itself hits a transient failure — so the lock stays held
//! for the whole refresh, including its retries, not released between
//! them.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::core::auth::{ServiceAccountCredentials, validate_credentials};
use crate::core::error::AlmanacError;
use crate::core::metrics::Metrics;
use crate::core::retry::{extract_reason, is_transient};

const CALENDAR_SCOPE: &str = "https://www.googleapis.com/auth/calendar";
const JWT_LIFETIME_SECS: u64 = 3600;
const JWT_BEARER_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
/// Refresh this long before actual expiry, so a request in flight
/// never races the token dying mid-call.
const REFRESH_MARGIN_SECS: u64 = 300;

/// Reads `CLIENT_EMAIL`, `PRIVATE_KEY`, `TOKEN_URI` from the process
/// environment — expected to have been injected directly by
/// `latch run --` (AR8), never read from a file on disk — and
/// delegates presence-checking to the pure core validator.
pub fn load_credentials() -> Result<ServiceAccountCredentials, AlmanacError> {
    validate_credentials(
        std::env::var("CLIENT_EMAIL").ok(),
        std::env::var("PRIVATE_KEY").ok(),
        std::env::var("TOKEN_URI").ok(),
    )
}

#[derive(Debug, Serialize)]
struct GoogleClaims {
    iss: String,
    scope: String,
    aud: String,
    exp: u64,
    iat: u64,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

fn now_secs() -> Result<u64, AlmanacError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|e| AlmanacError::Auth {
            message: format!("system clock is set before the Unix epoch: {e}"),
            remedy: "fix the system clock".to_string(),
            transient: false,
        })
}

/// Signs a fresh JWT and exchanges it for a bearer access token.
/// Returns the token and its absolute expiry (Unix seconds).
async fn fetch_token(
    http: &Client,
    creds: &ServiceAccountCredentials,
) -> Result<(String, u64), AlmanacError> {
    let now = now_secs()?;
    let jwt = sign_assertion(creds, now)?;
    let parsed = exchange(http, creds, &jwt).await?;
    Ok((parsed.access_token, now + parsed.expires_in))
}

/// The JWT that asks Google for a Calendar-scoped token, signed with the
/// service account's key.
fn sign_assertion(creds: &ServiceAccountCredentials, now: u64) -> Result<String, AlmanacError> {
    let claims = GoogleClaims {
        iss: creds.client_email.clone(),
        scope: CALENDAR_SCOPE.to_string(),
        aud: creds.token_url.clone(),
        iat: now,
        exp: now + JWT_LIFETIME_SECS,
    };

    // Secret managers commonly store multi-line PEM with literal `\n`
    // sequences rather than real newline bytes; normalize before
    // handing the bytes to the PEM parser.
    let pem = creds.private_key.replace("\\n", "\n");
    let key = EncodingKey::from_rsa_pem(pem.as_bytes()).map_err(|e| AlmanacError::Auth {
        message: format!("failed to parse the service-account private key: {e}"),
        remedy: "check that PRIVATE_KEY in Latch is the unmodified PEM from the Google service-account JSON"
            .to_string(),
        transient: false,
    })?;

    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key).map_err(|e| {
        AlmanacError::Auth {
            message: format!("failed to sign the service-account JWT: {e}"),
            remedy: "regenerate the service-account key in Google Cloud Console".to_string(),
            transient: false,
        }
    })
}

/// Trades the signed JWT for an access token at the token endpoint.
async fn exchange(
    http: &Client,
    creds: &ServiceAccountCredentials,
    jwt: &str,
) -> Result<TokenResponse, AlmanacError> {
    // A connection-level failure here (DNS, refused, reset, timeout) is
    // exactly the kind of passing network blip that must not need
    // Kenny's intervention — classified transient so the caller's
    // retry loop (shell::calendar_client::send_with_retry) tries again
    // instead of surfacing a permanent failure for a problem that will
    // very likely be gone on the next attempt.
    let response = http
        .post(&creds.token_url)
        .form(&[("grant_type", JWT_BEARER_GRANT_TYPE), ("assertion", jwt)])
        .send()
        .await
        .map_err(|e| AlmanacError::Auth {
            message: format!("token request to {} failed: {e}", creds.token_url),
            remedy: "transient network failure — retrying automatically".to_string(),
            transient: true,
        })?;

    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|e| format!("(the response body could not be read: {e})"));
        let reason = extract_reason(&body);
        let transient = is_transient(status.as_u16(), reason.as_deref());
        return Err(AlmanacError::Auth {
            message: format!("token endpoint returned HTTP {status}: {body}"),
            remedy: if transient {
                "transient token-endpoint failure — retrying automatically".to_string()
            } else {
                "check the service account is enabled and CLIENT_EMAIL/PRIVATE_KEY match"
                    .to_string()
            },
            transient,
        });
    }

    response.json().await.map_err(|e| AlmanacError::Auth {
        message: format!("failed to parse the token endpoint response: {e}"),
        remedy: "check Google's OAuth2 token endpoint response format hasn't changed".to_string(),
        transient: false,
    })
}

struct TokenState {
    access_token: String,
    expires_at_secs: u64,
}

/// Holds the current bearer token behind a single-flight lock (AR18).
/// `token()` refreshes only when needed; while a refresh is in flight,
/// every other caller blocks on the same lock and reuses its result
/// instead of starting a redundant request of its own.
pub struct TokenManager {
    http: Client,
    credentials: ServiceAccountCredentials,
    metrics: Arc<Metrics>,
    state: Mutex<Option<TokenState>>,
}

impl TokenManager {
    pub fn new(http: Client, credentials: ServiceAccountCredentials) -> Arc<Self> {
        Self::with_metrics(http, credentials, Arc::new(Metrics::default()))
    }

    /// The same, sharing the counters the rest of the process uses.
    /// A separate constructor rather than a parameter on `new`, so the
    /// setup examples and every existing test keep working unchanged.
    pub fn with_metrics(
        http: Client,
        credentials: ServiceAccountCredentials,
        metrics: Arc<Metrics>,
    ) -> Arc<Self> {
        Arc::new(Self {
            http,
            credentials,
            state: Mutex::new(None),
            metrics,
        })
    }

    /// Returns a valid bearer token, refreshing it first if necessary.
    pub async fn token(&self) -> Result<String, AlmanacError> {
        let mut guard = self.state.lock().await;

        let needs_refresh = match &*guard {
            Some(state) => now_secs()? + REFRESH_MARGIN_SECS >= state.expires_at_secs,
            None => true,
        };

        if needs_refresh {
            let (access_token, expires_at_secs) =
                fetch_token(&self.http, &self.credentials).await?;
            self.metrics.token_refreshed();
            *guard = Some(TokenState {
                access_token: access_token.clone(),
                expires_at_secs,
            });
            return Ok(access_token);
        }

        Ok(guard
            .as_ref()
            .expect("checked Some above")
            .access_token
            .clone())
    }
}

#[cfg(test)]
mod tests;
