//! Typed errors (AR14). Every variant carries an explicit `remedy`
//! field — a required struct field, not an implicit message template —
//! so a remedy can never be forgotten at a construction site (standing
//! rule 11: every error message carries a remedy). `GoogleApi` also
//! classifies itself as transient or permanent so the shell's retry
//! logic (M3/AR9) never has to sniff a message string to decide
//! whether to try again; see `retry::is_transient` for how that
//! classification is derived.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AlmanacError {
    #[error("{message} — {remedy}")]
    Config { message: String, remedy: String },

    #[error("{message} — {remedy}")]
    ProfileValidation { message: String, remedy: String },

    #[error("{message} — {remedy}")]
    Auth { message: String, remedy: String },

    #[error("{message} — {remedy}")]
    GoogleApi {
        message: String,
        remedy: String,
        transient: bool,
    },
}

impl AlmanacError {
    /// The remedy text every variant is required to carry.
    pub fn remedy(&self) -> &str {
        match self {
            AlmanacError::Config { remedy, .. }
            | AlmanacError::ProfileValidation { remedy, .. }
            | AlmanacError::Auth { remedy, .. }
            | AlmanacError::GoogleApi { remedy, .. } => remedy,
        }
    }

    /// Whether M3's retry-with-backoff should retry this error.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            AlmanacError::GoogleApi {
                transient: true,
                ..
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_carries_its_remedy_through_display() {
        let err = AlmanacError::Config {
            message: "missing default_calendar_id".to_string(),
            remedy: "add default_calendar_id to config.toml".to_string(),
        };
        assert!(err.to_string().contains("add default_calendar_id"));
        assert_eq!(err.remedy(), "add default_calendar_id to config.toml");
    }

    #[test]
    fn only_transient_google_api_errors_report_as_transient() {
        let transient = AlmanacError::GoogleApi {
            message: "rate limited".to_string(),
            remedy: "retrying automatically".to_string(),
            transient: true,
        };
        let permanent = AlmanacError::GoogleApi {
            message: "not found".to_string(),
            remedy: "check the event id".to_string(),
            transient: false,
        };
        assert!(transient.is_transient());
        assert!(!permanent.is_transient());
    }

    #[test]
    fn non_google_variants_are_never_transient() {
        let err = AlmanacError::Auth {
            message: "bad key".to_string(),
            remedy: "check PRIVATE_KEY".to_string(),
        };
        assert!(!err.is_transient());
    }
}
