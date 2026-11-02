//! Calendar calls: create and share, list, find, ensure and delete.

use crate::core::error::AlmanacError;

use super::{CalendarResource, GoogleCalendarClient, NewCalendar};

impl GoogleCalendarClient {
    /// Creates a calendar and immediately gives `share_with` ownership
    /// of it.
    ///
    /// The two steps are one method on purpose. A calendar this service
    /// account creates is owned by the service account and invisible to
    /// every other human being, and that is easy not to notice — the
    /// `almanac-test` calendar from L1 sat that way for months while
    /// the live tests wrote into it, and the first two real calendars
    /// were created the same way an hour before this was written.
    /// Making sharing a separate call is how that keeps happening, so
    /// the caller has to name an owner to create anything at all.
    pub async fn create_calendar(
        &self,
        summary: &str,
        share_with: &str,
    ) -> Result<String, AlmanacError> {
        let body = NewCalendar { summary };
        let response = self
            .send_with_retry(|http, token| http.post(&self.base_url).bearer_auth(token).json(&body))
            .await?;

        let parsed: CalendarResource =
            response.json().await.map_err(|e| AlmanacError::GoogleApi {
                message: format!("failed to parse the created calendar response: {e}"),
                remedy: "check Google's Calendar API response format hasn't changed".to_string(),
                transient: false,
            })?;

        self.share_calendar(&parsed.id, share_with).await?;
        Ok(parsed.id)
    }

    /// Every calendar this account can see, as `(id, display name)`,
    /// sorted by name.
    ///
    /// The dashboard offers these in a dropdown (K21): a calendar id is
    /// an opaque string nobody types on purpose, and having to go and
    /// find one before adding a source was most of the chore.
    pub async fn list_calendars(&self) -> Result<Vec<(String, String)>, AlmanacError> {
        let entries: Vec<serde_json::Value> = self
            .get_all_pages(&self.calendar_list_url, &[], "calendar list")
            .await?;

        let mut calendars: Vec<(String, String)> = entries
            .iter()
            .filter_map(|c| {
                Some((
                    c["id"].as_str()?.to_string(),
                    c["summary"].as_str()?.to_string(),
                ))
            })
            .collect();
        calendars.sort_by(|a, b| a.1.cmp(&b.1));
        Ok(calendars)
    }

    /// Finds a calendar this account can see by its display name, or
    /// `None`.
    pub async fn find_calendar_by_summary(
        &self,
        summary: &str,
    ) -> Result<Option<String>, AlmanacError> {
        Ok(self
            .list_calendars()
            .await?
            .into_iter()
            .find(|(_, name)| name == summary)
            .map(|(id, _)| id))
    }

    /// The id of the calendar called `summary`, creating and sharing it
    /// if this account cannot see one (K21).
    ///
    /// Matching on the name before creating is what stops a second
    /// "Almanac · Huishouden" appearing every time someone adds a
    /// source to it — and a duplicate calendar is close to invisible:
    /// events land, nothing errors, and half of them are on a calendar
    /// nobody has open.
    pub async fn ensure_calendar(
        &self,
        summary: &str,
        share_with: &str,
    ) -> Result<(String, bool), AlmanacError> {
        if let Some(id) = self.find_calendar_by_summary(summary).await? {
            return Ok((id, false));
        }
        let id = self.create_calendar(summary, share_with).await?;
        Ok((id, true))
    }

    /// Deletes a calendar and everything on it (K24).
    ///
    /// Irreversible at Google's end — this is not "remove it from my
    /// list", it is the calendar and every event ever put on it. The
    /// dashboard only offers it for a calendar no source writes to, and
    /// that guard is the reason this is safe to expose at all.
    pub async fn delete_calendar(&self, calendar_id: &str) -> Result<(), AlmanacError> {
        let calendar_id = super::segment(calendar_id);
        let url = format!("{}/{calendar_id}", self.base_url);
        self.send_with_retry(|http, token| http.delete(&url).bearer_auth(token))
            .await?;
        Ok(())
    }

    /// Gives `user` ownership of a calendar. Idempotent at Google's end:
    /// re-granting what someone already has is not an error.
    ///
    /// Owner rather than writer, so losing the service account never
    /// strands a calendar somebody depends on — the grantee can rename,
    /// share onward and delete it without us.
    pub async fn share_calendar(&self, calendar_id: &str, user: &str) -> Result<(), AlmanacError> {
        let calendar_id = super::segment(calendar_id);
        let url = format!("{}/{calendar_id}/acl", self.base_url);
        let rule = serde_json::json!({
            "role": "owner",
            "scope": {"type": "user", "value": user}
        });

        self.send_with_retry(|http, token| http.post(&url).bearer_auth(token).json(&rule))
            .await?;
        Ok(())
    }
}
