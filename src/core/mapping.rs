//! Turning an accepted payload into the event Almanac will write.
//!
//! Since 2.0.0 this is a thin door rather than a translation layer. The
//! payload IS Almanac's event shape (`core::request`), so all this does
//! is parse it and hand it over — the profile contributes the calendar
//! and two defaults.
//!
//! The function keeps its name and signature on purpose: the dry-run
//! surface, the delivery worker and the regression test all call it,
//! and none of them cares that what happens behind it got smaller.

use crate::core::calendar::GoogleEvent;
use crate::core::error::AlmanacError;
use crate::core::profile::Profile;
use crate::core::request::{EventRequest, to_google_event};

/// Maps one accepted payload onto a `GoogleEvent`.
///
/// `origin` names the profile for error messages (standing rule 11).
pub fn map_payload(
    payload: &serde_json::Value,
    profile: &Profile,
    origin: &str,
) -> Result<GoogleEvent, AlmanacError> {
    let request = EventRequest::parse(payload, origin)?;
    to_google_event(&request, profile, origin)
}
