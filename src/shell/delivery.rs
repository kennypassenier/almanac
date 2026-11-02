//! Delivers one journal entry to Google: map the payload through its
//! profile (K5), look for an existing event by its upsert key
//! (K2/AR15), then create or update accordingly. Shared by both the
//! synchronous ingest path (K8) and the background worker, so there is
//! exactly one implementation of the create-vs-update behaviour.
//!
//! AR16's serialization lives here too: deliveries that target the
//! same upsert key are serialized against each other, because
//! search-then-write is not atomic at Google. Two near-simultaneous
//! deliveries of the same logical event would otherwise both find
//! nothing and both create — the exact duplicate K2 exists to prevent
//! (the architecture-critic's Phase 4 objection). Different keys still
//! proceed concurrently.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::core::calendar::{ExtendedProperties, GoogleEvent};
use crate::core::error::AlmanacError;
use crate::core::journal::Entry;
use crate::core::mapping::map_payload;
use crate::core::profile::Profile;
use crate::core::upsert::{UpsertAction, decide};
use crate::shell::calendar_client::GoogleCalendarClient;

/// The extended-property key every Almanac-created event carries
/// (AR15). Pinned: changing it orphans every event already out there.
pub const UPSERT_PROPERTY: &str = "almanac_source_id";

/// Hands out one lock per upsert key, so same-key deliveries queue and
/// different-key deliveries don't. In memory only — this is transient
/// coordination, not state, so it does not compromise AR16's "the
/// journal and Google hold everything durable" position.
#[derive(Default)]
pub struct KeyLocks {
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Entries a delete cancelled while a worker pass may already hold
    /// them in its snapshot, waiting on the key lock the delete held.
    /// Bounded: the oldest ids drop out, by then no pass still holds them.
    cancelled: std::sync::Mutex<std::collections::VecDeque<String>>,
}

/// Ends the message of a delivery a delete cancelled; [`is_cancelled`] keys on it.
const CANCELLED_SUFFIX: &str = " was deleted by its source while this delivery waited";

/// Whether `e` is a delivery a delete cancelled, not a failure.
pub fn is_cancelled(e: &AlmanacError) -> bool {
    e.message().ends_with(CANCELLED_SUFFIX)
}

/// How many cancelled ids are remembered at most.
const CANCELLED_KEPT: usize = 256;

impl KeyLocks {
    pub fn new() -> Self {
        Self::default()
    }

    /// The lock for one upsert key. Public because the delete path
    /// (K8) has to take the same lock as delivery: a delete
    /// interleaving with an upsert of the same event would leave a
    /// recreated copy behind.
    pub async fn for_key(&self, key: &str) -> Arc<Mutex<()>> {
        let mut map = self.locks.lock().await;
        Arc::clone(map.entry(key.to_string()).or_default())
    }

    /// Records that a delete cancelled this entry (call with its key lock held).
    pub fn cancel(&self, entry_id: &str) {
        let mut ids = self.cancelled.lock().unwrap_or_else(|e| e.into_inner());
        if ids.len() == CANCELLED_KEPT {
            ids.pop_front();
        }
        ids.push_back(entry_id.to_string());
    }

    /// Whether a delete cancelled this entry; forgets it once asked.
    fn take_cancelled(&self, entry_id: &str) -> bool {
        let mut ids = self.cancelled.lock().unwrap_or_else(|e| e.into_inner());
        let found = ids.iter().position(|id| id == entry_id);
        found.map(|i| ids.remove(i)).is_some()
    }
}

/// What a delivery did, for the caller's response and the K11 debug
/// surface.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivered {
    pub event_id: String,
    pub created: bool,
    /// The key this delivery deduplicated against, or `None` when the
    /// source offered neither an external id nor an idempotency key.
    ///
    /// Carried back out so the debug surface can show it. It used to be
    /// hardcoded to `None` there, which meant every routing decision
    /// claimed there had been no upsert key — including the ones that
    /// plainly had, since they converged on the same event. Someone
    /// chasing a duplicate would read that, conclude the profile had no
    /// `external_id_field`, and go and edit a profile that was already
    /// correct.
    pub upsert_key: Option<String>,
}

/// Resolves the upsert key for an entry: the mapping's own
/// `almanac_source_id` extended property when the profile names an
/// external id field, or the source's supplied idempotency key (M7)
/// when it does not. `None` means the source offered neither, and the
/// event is created unconditionally.
pub(crate) fn upsert_key(entry: &Entry, event: &GoogleEvent) -> Option<String> {
    if let Some(props) = &event.extended_properties
        && let Some(key) = props.private.get(UPSERT_PROPERTY)
    {
        return Some(key.clone());
    }
    entry
        .idempotency_key
        .as_ref()
        .map(|k| crate::core::journal::idempotency_upsert_key(&entry.source_id, k))
}

/// For a header-keyed entry whose event carries no `external_id`, the key
/// such an event had before the namespaces split; looked up only when the
/// current key finds nothing.
fn legacy_key(entry: &Entry, event: &GoogleEvent, key: &str) -> Option<String> {
    let header = entry.idempotency_key.as_deref()?;
    let current = crate::core::journal::idempotency_upsert_key(&entry.source_id, header);
    let mapped_own = event
        .extended_properties
        .as_ref()
        .is_some_and(|p| p.private.contains_key(UPSERT_PROPERTY));
    (key == current && !mapped_own)
        .then(|| crate::core::journal::legacy_idempotency_key(&entry.source_id, header))
}

/// Puts the upsert key on the event itself when the mapping did not: a
/// source with no natural external id still needs its idempotency key on
/// the event, or a redelivery could not find it.
fn ensure_upsert_property(event: &mut GoogleEvent, key: &str) {
    if event.extended_properties.is_none() {
        let mut private = HashMap::new();
        private.insert(UPSERT_PROPERTY.to_string(), key.to_string());
        event.extended_properties = Some(ExtendedProperties { private });
    }
}

/// The id Google gave an event. An answer without one is a transient
/// failure (Kenny, 2026-10-30): the retry's upsert lookup then finds the
/// event that was written and updates it, instead of storing an empty id
/// or creating a second event.
fn google_id(id: Option<String>, action: &str) -> Result<String, AlmanacError> {
    id.ok_or_else(|| {
        tracing::warn!(action, "Google's answer carried no event id; retrying");
        AlmanacError::GoogleApi {
            message: format!("Google's answer to the {action} carried no event id"),
            remedy: "retrying automatically; the next try finds the event by its upsert key"
                .to_string(),
            transient: true,
        }
    })
}

/// Creates the event and reports it, with the key it was filed under.
async fn create(
    client: &GoogleCalendarClient,
    calendar_id: &str,
    event: &GoogleEvent,
    upsert_key: Option<String>,
) -> Result<Delivered, AlmanacError> {
    let created = client.create_event(calendar_id, event).await?;
    Ok(Delivered {
        event_id: google_id(created.id, "create")?,
        created: true,
        upsert_key,
    })
}

/// Delivers one entry. Idempotent with respect to the upsert key: a
/// redelivery after a crash updates the event it already created
/// rather than making a second one.
pub async fn deliver(
    entry: &Entry,
    profile: &Profile,
    client: &GoogleCalendarClient,
    locks: &KeyLocks,
) -> Result<Delivered, AlmanacError> {
    let origin = format!("profile {}", profile.source_id);
    let mut event = map_payload(&entry.payload, profile, &origin)?;
    let key = upsert_key(entry, &event);
    let legacy = key.as_deref().and_then(|k| legacy_key(entry, &event, k));
    if let Some(key) = &key {
        ensure_upsert_property(&mut event, key);
    }
    let calendar_id = &profile.target_calendar_id;

    let Some(key) = key else {
        // Nothing to deduplicate against — create and return.
        return create(client, calendar_id, &event, None).await;
    };

    let lock = locks.for_key(&key).await;
    let _guard = lock.lock().await;
    if locks.take_cancelled(&entry.id) {
        // Review 2026-11-02: a pass held this entry while a delete marked it
        // done and removed the event; writing it now would bring it back.
        // Transient, so the worker only leaves it: it is no longer pending.
        return Err(AlmanacError::GoogleApi {
            message: format!("{key}{CANCELLED_SUFFIX}"),
            remedy: "nothing to do: the event stays deleted".to_string(),
            transient: true,
        });
    }

    let mut found = client
        .find_event_by_property(calendar_id, UPSERT_PROPERTY, &key)
        .await?;
    if found.is_none()
        && let Some(legacy) = &legacy
    {
        // Written before the split: found once, and the update below puts
        // the current key on it.
        found = client
            .find_event_by_property(calendar_id, UPSERT_PROPERTY, legacy)
            .await?;
    }
    let existing = match found {
        // Found but without an id: retry rather than create a second one.
        Some(found) => Some(google_id(found.id, "lookup")?),
        None => None,
    };

    match decide(existing) {
        UpsertAction::Create => create(client, calendar_id, &event, Some(key)).await,
        UpsertAction::Update(google_id) => {
            let updated = client.update_event(calendar_id, &google_id, &event).await?;
            Ok(Delivered {
                event_id: updated.id.unwrap_or(google_id),
                created: false,
                upsert_key: Some(key),
            })
        }
    }
}

#[cfg(test)]
mod tests;
