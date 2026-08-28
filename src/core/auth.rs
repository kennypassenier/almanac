//! Pure validation of service-account credentials. Reading the actual
//! environment variables is I/O and belongs in `shell::auth`; this
//! module only checks that what was found is complete and says
//! exactly what is missing when it isn't (standing rule 11).

use crate::core::error::AlmanacError;

/// Service-account credentials for the Google OAuth2 JWT-bearer flow.
/// Loaded by `shell::auth` from environment variables Latch injects
/// directly into the process (AR8) — never read from a file.
///
/// No `Debug` derive: this struct holds `private_key` in plaintext,
/// and standing rule 10 (secrets never in logs) means an accidental
/// `{:?}` on it must be a compile error, not a leak.
#[derive(Clone)]
pub struct ServiceAccountCredentials {
    pub client_email: String,
    pub private_key: String,
    pub token_url: String,
}

/// Validates that all three required credential values were found.
/// This project only ever reads these three fields from the Google
/// service-account JSON (see INVENTORY.md AUTH-1 — the old
/// `.env.example` listed ten, unused ones were dead documentation).
pub fn validate_credentials(
    client_email: Option<String>,
    private_key: Option<String>,
    token_url: Option<String>,
) -> Result<ServiceAccountCredentials, AlmanacError> {
    let client_email = client_email.ok_or_else(|| AlmanacError::Auth {
        message: "CLIENT_EMAIL is not set".to_string(),
        remedy:
            "run this process via `latch run --` with the almanac service-account secrets loaded"
                .to_string(),
    })?;
    let private_key = private_key.ok_or_else(|| AlmanacError::Auth {
        message: "PRIVATE_KEY is not set".to_string(),
        remedy:
            "run this process via `latch run --` with the almanac service-account secrets loaded"
                .to_string(),
    })?;
    let token_url = token_url.ok_or_else(|| AlmanacError::Auth {
        message: "TOKEN_URI is not set".to_string(),
        remedy: "set TOKEN_URI, typically https://oauth2.googleapis.com/token".to_string(),
    })?;

    Ok(ServiceAccountCredentials {
        client_email,
        private_key,
        token_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ServiceAccountCredentials` deliberately has no `Debug` impl (it
    /// holds a plaintext private key — standing rule 10), so
    /// `Result::unwrap_err` isn't available; this does the same thing
    /// without requiring `T: Debug`.
    fn expect_err<T>(result: Result<T, AlmanacError>) -> AlmanacError {
        match result {
            Ok(_) => panic!("expected an error, got Ok"),
            Err(e) => e,
        }
    }

    #[test]
    fn all_present_succeeds() {
        let creds = validate_credentials(
            Some("sa@example.iam.gserviceaccount.com".to_string()),
            Some("-----BEGIN PRIVATE KEY-----\n...\n-----END PRIVATE KEY-----".to_string()),
            Some("https://oauth2.googleapis.com/token".to_string()),
        )
        .unwrap();
        assert_eq!(creds.client_email, "sa@example.iam.gserviceaccount.com");
    }

    #[test]
    fn missing_client_email_names_itself_and_points_at_latch() {
        let err = expect_err(validate_credentials(
            None,
            Some("key".to_string()),
            Some("url".to_string()),
        ));
        assert!(err.to_string().contains("CLIENT_EMAIL"));
        assert!(err.remedy().contains("latch run"));
    }

    #[test]
    fn missing_private_key_names_itself() {
        let err = expect_err(validate_credentials(
            Some("e".to_string()),
            None,
            Some("url".to_string()),
        ));
        assert!(err.to_string().contains("PRIVATE_KEY"));
    }

    #[test]
    fn missing_token_url_names_itself_with_the_typical_value() {
        let err = expect_err(validate_credentials(
            Some("e".to_string()),
            Some("k".to_string()),
            None,
        ));
        assert!(err.to_string().contains("TOKEN_URI"));
        assert!(err.remedy().contains("oauth2.googleapis.com"));
    }
}
