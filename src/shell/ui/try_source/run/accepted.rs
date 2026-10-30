//! An accepted call: the event Almanac would write, looked up against
//! what the calendar holds now, read only.

use serde_json::{Value, json};

use crate::core::calendar::{ExtendedProperties, GoogleEvent};
use crate::core::dry_run::{self, Outcome};
use crate::core::journal::Entry;
use crate::core::mapping::map_payload;
use crate::core::profile::Profile;
use crate::shell::delivery::{UPSERT_PROPERTY, upsert_key};
use crate::shell::state::AppState;
use chassis::Error;

use super::RunRequest;

/// An accepted call: the event Almanac would write, and what is in the
/// calendar under the same key now.
pub(super) struct Accepted {
    event: GoogleEvent,
    existing: Option<GoogleEvent>,
    lookup_failed: bool,
}

pub(super) async fn accepted(
    state: &AppState,
    request: &RunRequest,
    profile: &Profile,
    origin_label: &str,
) -> Result<Accepted, Error> {
    // Map again for the event itself (cheap and pure).
    let mut event: GoogleEvent = map_payload(&request.event, profile, origin_label)
        .map_err(|e| Error::internal(e.to_string(), e.remedy().to_string()))?;
    let entry = Entry {
        id: String::new(),
        source_id: profile.source_id.clone(),
        received_at: String::new(),
        payload: request.event.clone(),
        idempotency_key: request
            .idempotency_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_string),
    };
    let key = upsert_key(&entry, &event);
    // Delivery stamps the marker on an event whose call had no external_id.
    if let Some(key) = &key
        && event.extended_properties.is_none()
    {
        event.extended_properties = Some(ExtendedProperties {
            private: std::iter::once((UPSERT_PROPERTY.to_string(), key.clone())).collect(),
        });
    }
    let (existing, lookup_failed) = match &key {
        None => (None, false),
        Some(key) => lookup_existing(state, profile, key).await,
    };
    Ok(Accepted {
        event,
        existing,
        lookup_failed,
    })
}

/// Read only: the same lookup delivery does before it creates or updates.
async fn lookup_existing(
    state: &AppState,
    profile: &Profile,
    key: &str,
) -> (Option<GoogleEvent>, bool) {
    match state
        .client
        .find_event_by_property(&profile.target_calendar_id, UPSERT_PROPERTY, key)
        .await
    {
        Ok(found) => (found, false),
        Err(e) => {
            tracing::warn!(source_id = %profile.source_id, error = %e, "try: could not look up the existing event");
            (None, true)
        }
    }
}

impl Accepted {
    pub(super) fn into_json(
        self,
        source_id: &str,
        profile: &Profile,
        calendar_name: &str,
        urls: Value,
    ) -> Value {
        let verdict =
            (!self.lookup_failed).then(|| dry_run::verdict(self.existing.as_ref(), &self.event));
        let outcome = Outcome::Mapped {
            summary: &self.event.summary,
            all_day: self.event.start.is_all_day(),
            verdict,
        };
        let hops = dry_run::hops(source_id, calendar_name, &outcome);
        let existing = self.existing.map(|mut found| {
            found.id = None;
            found
        });
        json!({
            "http": 200,
            "answer": {
                "status": "ok",
                "would_write_to_calendar": profile.target_calendar_id,
                "event": self.event,
            },
            "hops": hops,
            "verdict": verdict,
            "existing": existing,
            "lookup_failed": self.lookup_failed,
            "calendar_name": calendar_name,
            "urls": urls,
        })
    }
}
