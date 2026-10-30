//! Following Google's page tokens, so a list call returns every item
//! rather than the first page only.

use serde::de::DeserializeOwned;

use crate::core::calendar::ListPage;
use crate::core::error::AlmanacError;

use super::GoogleCalendarClient;

/// The most pages one list call follows. Google ends a listing by
/// leaving `nextPageToken` out; a token that never ends would loop
/// forever, so past this many pages the call fails instead.
const MAX_PAGES: usize = 100;

impl GoogleCalendarClient {
    /// GETs `url` with `query`, follows `nextPageToken` until Google
    /// stops sending one, and returns the items of every page in order.
    /// `what` names the listing in an error ("event list").
    pub(super) async fn get_all_pages<T: DeserializeOwned>(
        &self,
        url: &str,
        query: &[(&str, &str)],
        what: &str,
    ) -> Result<Vec<T>, AlmanacError> {
        let mut items = Vec::new();
        let mut page_token: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let response = self
                .send_with_retry(|http, token| {
                    let request = http.get(url).bearer_auth(token).query(query);
                    match &page_token {
                        Some(next) => request.query(&[("pageToken", next.as_str())]),
                        None => request,
                    }
                })
                .await?;
            let page: ListPage<T> = response.json().await.map_err(|e| AlmanacError::GoogleApi {
                message: format!("failed to parse the Calendar API's {what} response: {e}"),
                remedy: "check Google's Calendar API response format hasn't changed".to_string(),
                transient: false,
            })?;
            items.extend(page.items.unwrap_or_default());
            match page.next_page_token {
                Some(next) => page_token = Some(next),
                None => return Ok(items),
            }
        }
        Err(AlmanacError::GoogleApi {
            message: format!("the Calendar API's {what} did not end after {MAX_PAGES} pages"),
            remedy: "check whether Google keeps sending the same page token".to_string(),
            transient: false,
        })
    }
}
