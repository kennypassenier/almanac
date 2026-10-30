//! Checks and preparation applied to a payload before it is journalled:
//! the identity requirement, the mappability check, the journal entry,
//! and the durable acceptance that combines them.

use axum::http::{HeaderMap, StatusCode};
use serde_json::Value;

use super::{AppState, IDEMPOTENCY_HEADER, Reply, error};
use crate::core::journal::Entry;
use crate::core::profile::Profile;

/// Refuses a call Almanac would not be able to find again.
///
/// Every event Almanac creates carries a marker built from the source's
/// own id, and that marker is the only handle the upsert and the delete
/// endpoint have. A call with neither an `external_id` in the payload
/// nor an `Idempotency-Key` header produces an event Almanac can never
/// update or remove — it duplicates on every resend and answers 404 on
/// every delete.
///
/// Refused at the door rather than left to a profile default. The
/// JobTracker session hit exactly this on 2026-09-03 against the live
/// service, hours after the dashboard started writing profiles: two
/// identical posts, two events, and a delete answering 404. A default
/// in a template fixes the next source; a refusal here fixes all of
/// them.
pub(crate) fn requires_an_id(payload: &Value, headers: &HeaderMap) -> Result<(), Reply> {
    let has_external_id = payload
        .get("external_id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.trim().is_empty());
    let has_header = headers
        .get(IDEMPOTENCY_HEADER)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| !s.trim().is_empty());

    if has_external_id || has_header {
        return Ok(());
    }
    Err(error(
        StatusCode::UNPROCESSABLE_ENTITY,
        "the payload has no \"external_id\" and no Idempotency-Key header",
        "send \"external_id\" with the source's own id for this thing, or an \
         Idempotency-Key header — without one of the two, almanac cannot find the event \
         again to update or delete it, and every resend creates another",
    ))
}

/// Refuses a payload Almanac could never turn into an event.
///
/// Checked at the door, before the journal, so a body that can only
/// ever fail is answered with 422 instead of being stored and retried
/// until it dead-letters. Two things were wrong without this, and they
/// were the same thing seen from two sides (Kenny, 2026-09-03):
///
/// - the asynchronous post answered a reassuring 202 to a payload with
///   a misspelled field, and the mistake only surfaced later, in a list
///   nobody watches;
/// - the synchronous post answered 502 both when Google had hiccuped
///   and when the body was unusable, so a caller could not tell "wait,
///   almanac is retrying" from "retrying will never help". The
///   JobTracker session was showing "almanac will try again" for a date
///   sent without `all_day`, which is a sentence nobody could disprove.
///
/// With this, 502 means Google and only Google, and 422 means the
/// source has to send something else. Neither needs a new field:
/// HTTP already has this distinction and we were not using it.
///
/// The cost, accepted: a payload almanac cannot map is refused rather
/// than kept. It was lost either way — this way the sender hears it
/// while it can still act.
fn must_be_mappable(payload: &Value, profile: &Profile) -> Result<(), Reply> {
    match crate::core::mapping::map_payload(
        payload,
        profile,
        &format!("profile {}", profile.source_id),
    ) {
        Ok(_) => Ok(()),
        Err(e) => Err(error(
            StatusCode::UNPROCESSABLE_ENTITY,
            &e.to_string(),
            e.remedy(),
        )),
    }
}

pub(super) fn build_entry(
    state: &AppState,
    source_id: &str,
    headers: &HeaderMap,
    payload: Value,
) -> Entry {
    let idempotency_key = headers
        .get(IDEMPOTENCY_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    Entry {
        id: uuid::Uuid::new_v4().to_string(),
        source_id: source_id.to_string(),
        received_at: (state.now)(),
        payload,
        idempotency_key,
    }
}

/// Validates a payload, journals it durably and counts it accepted: the
/// steps both POST endpoints share, in the order that keeps a doomed
/// body out of the journal (identity, then mappability, then fsync).
///
/// On a journal write failure the caller gets a 500 and nothing was
/// accepted, so it knows to resend.
pub(super) async fn accept_payload(
    state: &AppState,
    profile: &Profile,
    headers: &HeaderMap,
    payload: Value,
) -> Result<Entry, Reply> {
    requires_an_id(&payload, headers)?;
    // On the synchronous path the payload is mapped a second time inside
    // `deliver`. It is a pure function over the payload and the profile,
    // so the second run is cheap, and the alternative is a journalled
    // entry that exists only to fail.
    must_be_mappable(&payload, profile)?;

    let entry = build_entry(state, &profile.source_id, headers, payload);

    if let Err(e) = state.journal.accept(&entry).await {
        tracing::error!(
            source_id = %profile.source_id, error = %e,
            "failed to journal an accepted payload"
        );
        return Err(error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &e.to_string(),
            e.remedy(),
        ));
    }

    state.metrics.accepted();
    Ok(entry)
}
