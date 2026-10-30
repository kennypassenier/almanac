//! The per-field checks on a source's event, each refusal naming the
//! field and what to send instead.

use crate::core::calendar::ExtendedProperties;
use crate::core::error::AlmanacError;
use crate::core::profile::Profile;

use super::EventRequest;

/// The three statuses Google accepts, refused here rather than at
/// Google — a rejected status arrives as a 400 with no field name.
const STATUSES: [&str; 3] = ["confirmed", "tentative", "cancelled"];

/// A refusal of what the source sent, named after where it came from.
fn invalid(origin: &str, message: String, remedy: String) -> AlmanacError {
    AlmanacError::ProfileValidation {
        message: format!("{origin}: {message}"),
        remedy,
    }
}

/// A title, and an external id that is not blank when it is sent.
pub(super) fn check_identity(request: &EventRequest, origin: &str) -> Result<(), AlmanacError> {
    if request.title.trim().is_empty() {
        return Err(invalid(
            origin,
            "\"title\" is empty".to_string(),
            "send a title — an untitled calendar entry tells nobody anything".to_string(),
        ));
    }
    if request
        .external_id
        .as_deref()
        .is_some_and(|id| id.trim().is_empty())
    {
        return Err(invalid(
            origin,
            "\"external_id\" is empty".to_string(),
            "send a stable id for this thing, or leave the field out and send an \
             Idempotency-Key header instead"
                .to_string(),
        ));
    }
    Ok(())
}

/// Google's colour id for a colour name or id the source sent.
pub(super) fn resolve_color(requested: &str, origin: &str) -> Result<String, AlmanacError> {
    crate::core::colors::resolve(requested)
        .map(|id| id.to_string())
        .ok_or_else(|| {
            invalid(
                origin,
                format!("\"color\": {requested:?} is not a Google colour"),
                format!(
                    "use one of: {}, or a colour id 1-11",
                    crate::core::colors::names()
                ),
            )
        })
}

pub(super) fn check_status(request: &EventRequest, origin: &str) -> Result<(), AlmanacError> {
    match &request.status {
        Some(status) if !STATUSES.contains(&status.as_str()) => Err(invalid(
            origin,
            format!("\"status\": {status:?} is not a Google event status"),
            format!("use one of: {}", STATUSES.join(", ")),
        )),
        _ => Ok(()),
    }
}

/// AR15's pinned upsert-key format: "<source_id>:<external-id>".
/// Absent when the source sent no external_id — the delivery layer then
/// fills the marker from the Idempotency-Key header (M7), and ingest has
/// already refused a call that offered neither.
pub(super) fn upsert_marker(
    request: &EventRequest,
    profile: &Profile,
) -> Option<ExtendedProperties> {
    request.external_id.as_deref().map(|external_id| {
        let mut private = std::collections::HashMap::new();
        private.insert(
            "almanac_source_id".to_string(),
            format!("{}:{external_id}", profile.source_id),
        );
        ExtendedProperties { private }
    })
}
