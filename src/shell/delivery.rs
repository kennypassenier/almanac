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
}

impl KeyLocks {
    pub fn new() -> Self {
        Self::default()
    }

    async fn for_key(&self, key: &str) -> Arc<Mutex<()>> {
        let mut map = self.locks.lock().await;
        Arc::clone(map.entry(key.to_string()).or_default())
    }
}

/// What a delivery did, for the caller's response and the K11 debug
/// surface.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivered {
    pub event_id: String,
    pub created: bool,
}

/// Resolves the upsert key for an entry: the mapping's own
/// `almanac_source_id` extended property when the profile names an
/// external id field, or the source's supplied idempotency key (M7)
/// when it does not. `None` means the source offered neither, and the
/// event is created unconditionally.
fn upsert_key(entry: &Entry, event: &crate::core::calendar::GoogleEvent) -> Option<String> {
    if let Some(props) = &event.extended_properties
        && let Some(key) = props.private.get(UPSERT_PROPERTY)
    {
        return Some(key.clone());
    }
    entry
        .idempotency_key
        .as_ref()
        .map(|k| format!("{}:{k}", entry.source_id))
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

    // A source with no natural external id still needs its idempotency
    // key on the event itself, or a redelivery could not find it.
    if let Some(key) = &key
        && event.extended_properties.is_none()
    {
        let mut private = HashMap::new();
        private.insert(UPSERT_PROPERTY.to_string(), key.clone());
        event.extended_properties = Some(crate::core::calendar::ExtendedProperties { private });
    }

    let calendar_id = &profile.target_calendar_id;

    let Some(key) = key else {
        // Nothing to deduplicate against — create and return.
        let created = client.create_event(calendar_id, &event).await?;
        return Ok(Delivered {
            event_id: created.id.unwrap_or_default(),
            created: true,
        });
    };

    let lock = locks.for_key(&key).await;
    let _guard = lock.lock().await;

    let existing = client
        .find_event_by_property(calendar_id, UPSERT_PROPERTY, &key)
        .await?
        .and_then(|e| e.id);

    match decide(existing) {
        UpsertAction::Create => {
            let created = client.create_event(calendar_id, &event).await?;
            Ok(Delivered {
                event_id: created.id.unwrap_or_default(),
                created: true,
            })
        }
        UpsertAction::Update(google_id) => {
            let updated = client.update_event(calendar_id, &google_id, &event).await?;
            Ok(Delivered {
                event_id: updated.id.unwrap_or(google_id),
                created: false,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::calendar::{EventDateTime, ExtendedProperties, GoogleEvent};
    use serde_json::json;

    fn event_with_property(value: Option<&str>) -> GoogleEvent {
        let extended_properties = value.map(|v| {
            let mut private = HashMap::new();
            private.insert(UPSERT_PROPERTY.to_string(), v.to_string());
            ExtendedProperties { private }
        });
        GoogleEvent {
            id: None,
            summary: "t".to_string(),
            description: None,
            location: None,
            color_id: None,
            start: EventDateTime {
                date_time: "2026-08-28T09:00:00+00:00".to_string(),
                time_zone: "UTC".to_string(),
            },
            end: EventDateTime {
                date_time: "2026-08-28T10:00:00+00:00".to_string(),
                time_zone: "UTC".to_string(),
            },
            extended_properties,
        }
    }

    fn entry(idempotency_key: Option<&str>) -> Entry {
        Entry {
            id: "j1".to_string(),
            source_id: "home-assistant".to_string(),
            received_at: "2026-08-28T09:00:00+00:00".to_string(),
            payload: json!({}),
            idempotency_key: idempotency_key.map(|k| k.to_string()),
        }
    }

    #[test]
    fn the_mappings_own_property_is_the_upsert_key_when_present() {
        let event = event_with_property(Some("home-assistant:switch.wasmachine"));
        assert_eq!(
            upsert_key(&entry(None), &event),
            Some("home-assistant:switch.wasmachine".to_string())
        );
    }

    #[test]
    fn an_idempotency_key_is_used_when_the_mapping_has_no_external_id() {
        let event = event_with_property(None);
        assert_eq!(
            upsert_key(&entry(Some("abc123")), &event),
            Some("home-assistant:abc123".to_string())
        );
    }

    #[test]
    fn the_mappings_property_wins_over_an_idempotency_key() {
        // A source that has both a real external id and sends a key
        // should dedupe on the durable identity, not the per-request
        // one, or a retry with a fresh key would duplicate.
        let event = event_with_property(Some("home-assistant:switch.wasmachine"));
        assert_eq!(
            upsert_key(&entry(Some("abc123")), &event),
            Some("home-assistant:switch.wasmachine".to_string())
        );
    }

    #[test]
    fn neither_source_of_identity_means_no_key_and_an_unconditional_create() {
        let event = event_with_property(None);
        assert_eq!(upsert_key(&entry(None), &event), None);
    }

    #[tokio::test]
    async fn the_same_key_hands_back_the_same_lock_and_different_keys_do_not() {
        let locks = KeyLocks::new();
        let a1 = locks.for_key("k1").await;
        let a2 = locks.for_key("k1").await;
        let b = locks.for_key("k2").await;

        assert!(Arc::ptr_eq(&a1, &a2), "same key must share one lock");
        assert!(
            !Arc::ptr_eq(&a1, &b),
            "different keys must not share a lock"
        );
    }
}
