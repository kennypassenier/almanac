//! Event calls: create, read, update, delete and the property lookups
//! the upsert relies on.

use crate::core::calendar::GoogleEvent;
use crate::core::error::AlmanacError;

use super::GoogleCalendarClient;

/// The most events Google returns on one page, so a week usually comes
/// back in one request; `get_all_pages` follows the rest when it does not.
const MAX_EVENTS_PER_PAGE: &str = "2500";

impl GoogleCalendarClient {
    pub async fn create_event(
        &self,
        calendar_id: &str,
        event: &GoogleEvent,
    ) -> Result<GoogleEvent, AlmanacError> {
        let url = format!("{}/{calendar_id}/events", self.base_url);
        let response = self
            .send_with_retry(|http, token| http.post(&url).bearer_auth(token).json(event))
            .await?;
        Self::parse_event(response).await
    }

    pub async fn get_event(
        &self,
        calendar_id: &str,
        event_id: &str,
    ) -> Result<GoogleEvent, AlmanacError> {
        let url = format!("{}/{calendar_id}/events/{event_id}", self.base_url);
        let response = self
            .send_with_retry(|http, token| http.get(&url).bearer_auth(token))
            .await?;
        Self::parse_event(response).await
    }

    /// Full resource replacement (Google's PUT semantics): the caller
    /// must resend every field it wants kept, not only what changed.
    pub async fn update_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        event: &GoogleEvent,
    ) -> Result<GoogleEvent, AlmanacError> {
        let url = format!("{}/{calendar_id}/events/{event_id}", self.base_url);
        let response = self
            .send_with_retry(|http, token| http.put(&url).bearer_auth(token).json(event))
            .await?;
        Self::parse_event(response).await
    }

    pub async fn delete_event(
        &self,
        calendar_id: &str,
        event_id: &str,
    ) -> Result<(), AlmanacError> {
        let url = format!("{}/{calendar_id}/events/{event_id}", self.base_url);
        self.send_with_retry(|http, token| http.delete(&url).bearer_auth(token))
            .await?;
        Ok(())
    }

    /// Finds an event by a private extended-property key/value pair —
    /// the mechanism K2's upsert lookup and AR15's `source_id` tagging
    /// depend on. Returns the first match, or `None`.
    pub async fn find_event_by_property(
        &self,
        calendar_id: &str,
        key: &str,
        value: &str,
    ) -> Result<Option<GoogleEvent>, AlmanacError> {
        Ok(self
            .list_events_by_property(calendar_id, key, value)
            .await?
            .into_iter()
            .next())
    }

    /// Every event carrying the given private extended property.
    /// `find_event_by_property` is the upsert path and only needs the
    /// first; this exists so a test can assert how many events a
    /// redelivery actually left behind, rather than inferring it.
    pub async fn list_events_by_property(
        &self,
        calendar_id: &str,
        key: &str,
        value: &str,
    ) -> Result<Vec<GoogleEvent>, AlmanacError> {
        let url = format!("{}/{calendar_id}/events", self.base_url);
        let filter = format!("{key}={value}");
        self.get_all_pages(
            &url,
            &[("privateExtendedProperty", filter.as_str())],
            "event list",
        )
        .await
    }

    /// Every event that starts in `[time_min, time_max)` (RFC 3339), repeats
    /// expanded, oldest first. Read-only: the History page's week agenda
    /// filters these to the ones carrying Almanac's own property.
    pub async fn list_events_between(
        &self,
        calendar_id: &str,
        time_min: &str,
        time_max: &str,
    ) -> Result<Vec<GoogleEvent>, AlmanacError> {
        let url = format!("{}/{calendar_id}/events", self.base_url);
        let query = [
            ("timeMin", time_min),
            ("timeMax", time_max),
            ("singleEvents", "true"),
            ("orderBy", "startTime"),
            ("maxResults", MAX_EVENTS_PER_PAGE),
        ];
        self.get_all_pages(&url, &query, "event list").await
    }
}
