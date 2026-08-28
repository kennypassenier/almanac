//! The upsert decision (K2): given whether a matching event already
//! exists, decide whether to create or update it. This is the pure
//! branch at the heart of the pattern the architecture-critic called
//! "the core value" of the Vikunja integration to recycle (VIK-5) —
//! deliberately factored out on its own so `shell` can serialize it
//! per external id (AR16's in-memory lock, L3) without duplicating the
//! decision logic itself.

/// What to do with a mapped event, given the result of looking it up
/// by its `almanac_source_id` extended property.
#[derive(Debug, Clone, PartialEq)]
pub enum UpsertAction {
    Create,
    /// Update the event with this Google-assigned id.
    Update(String),
}

/// Decides create-vs-update from an upsert lookup's result. `existing`
/// is `None` when nothing matched (including when the mapping has no
/// external id at all — see `core::mapping`), and `Some(google_id)`
/// when a prior event with the same `almanac_source_id` was found.
pub fn decide(existing: Option<String>) -> UpsertAction {
    match existing {
        Some(google_id) => UpsertAction::Update(google_id),
        None => UpsertAction::Create,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_existing_match_means_create() {
        assert_eq!(decide(None), UpsertAction::Create);
    }

    #[test]
    fn an_existing_match_means_update_with_its_id() {
        assert_eq!(
            decide(Some("evt123".to_string())),
            UpsertAction::Update("evt123".to_string())
        );
    }
}
