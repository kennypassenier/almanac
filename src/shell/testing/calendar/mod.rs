//! The Google Calendar API stand-in: what it observed, and how a test
//! scripts it.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use axum::Router;
use serde_json::Value;
use tokio::sync::Mutex;

mod handlers;

use handlers::{
    create_calendar, create_event, delete_calendar, delete_event, insert_acl, list_calendars,
    list_events, update_event,
};

/// What a stub observed, so a test can assert on how it was called
/// rather than only on what came back.
#[derive(Default)]
pub struct CalendarState {
    /// Every request, as `(method, calendar_id)`. The calendar id is
    /// the point: it is how K3's routing is checked.
    pub requests: Mutex<Vec<(String, String)>>,
    /// Events created, as `(calendar_id, body)`.
    pub created: Mutex<Vec<(String, Value)>>,
    /// Answer this many requests with 503 before behaving. Lets a test
    /// script a transient outage and assert the retry loop rides it
    /// out.
    pub fail_times: AtomicUsize,
    /// Answer this many requests with a permanent 403. Scripted
    /// separately from `fail_times` because the whole point of the
    /// classification is that these two are treated differently.
    pub reject_times: AtomicUsize,
    /// Calendars created through this stub, keyed by id.
    pub calendars: Mutex<HashMap<String, Created>>,
    /// Models Google's eventually-consistent calendar list: while this
    /// is set, a calendar the stub creates is held back from
    /// `calendarList` until `catch_up` is called.
    ///
    /// Without it the stub answers instantly and consistently, and that
    /// assumption is what let a real bug through — the dashboard's
    /// find-or-create guard reads that list, so during the lag it finds
    /// nothing and makes a second calendar.
    pub lag_new_calendars: AtomicBool,
    /// Events the stub already holds, keyed by calendar id, returned
    /// from the list endpoint so an upsert can find one.
    pub existing: Mutex<HashMap<String, Vec<Value>>>,
    /// How many items one list page holds; 0 means everything on one
    /// page. Set through `page_by` so a test can make Google page.
    pub page_size: AtomicUsize,
    next_id: AtomicUsize,
}

impl CalendarState {
    /// Whether this request should fail, consuming one scripted
    /// failure if so.
    fn take_failure(&self) -> bool {
        self.fail_times
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                if n == 0 { None } else { Some(n - 1) }
            })
            .is_ok()
    }

    /// Whether this request should be permanently rejected.
    fn take_rejection(&self) -> bool {
        self.reject_times
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                if n == 0 { None } else { Some(n - 1) }
            })
            .is_ok()
    }

    async fn record(&self, method: &str, calendar_id: &str) {
        self.requests
            .lock()
            .await
            .push((method.to_string(), calendar_id.to_string()));
    }

    /// Which calendars this stub was asked to write to, in order.
    pub async fn calendars_written(&self) -> Vec<String> {
        self.requests
            .lock()
            .await
            .iter()
            .filter(|(method, _)| method == "POST" || method == "PUT")
            .map(|(_, calendar)| calendar.clone())
            .collect()
    }

    pub async fn request_count(&self) -> usize {
        self.requests.lock().await.len()
    }
}

/// Calendars the stub was asked to create, and who each was shared
/// with — the pair that must never come apart.
#[derive(Default)]
pub struct Created {
    pub name: String,
    pub shared_with: Vec<String>,
    /// Whether `calendarList` admits this one exists yet.
    pub listed: bool,
}

/// A stand-in for the Google Calendar API.
pub struct CalendarStub {
    pub base_url: String,
    pub state: Arc<CalendarState>,
}

impl CalendarStub {
    /// Hold every calendar created from now on out of `calendarList`,
    /// the way Google does for the first seconds.
    pub fn lag_new_calendars(&self) {
        self.state.lag_new_calendars.store(true, Ordering::SeqCst);
    }

    /// Google has caught up: everything created is listed again.
    pub async fn catch_up(&self) {
        self.state.lag_new_calendars.store(false, Ordering::SeqCst);
        for created in self.state.calendars.lock().await.values_mut() {
            created.listed = true;
        }
    }

    /// Who a created calendar ended up shared with.
    pub async fn shared_with(&self, name: &str) -> Vec<String> {
        self.state
            .calendars
            .lock()
            .await
            .values()
            .find(|c| c.name == name)
            .map(|c| c.shared_with.clone())
            .unwrap_or_default()
    }

    /// Starts the stub on a loopback port.
    pub async fn start() -> Self {
        let state = Arc::new(CalendarState::default());

        let router = Router::new()
            .route(
                "/{calendar_id}/events",
                axum::routing::get(list_events).post(create_event),
            )
            .route(
                "/{calendar_id}/events/{event_id}",
                axum::routing::put(update_event).delete(delete_event),
            )
            .route("/", axum::routing::post(create_calendar))
            .route("/calendarList", axum::routing::get(list_calendars))
            .route("/{calendar_id}/acl", axum::routing::post(insert_acl))
            .route("/{calendar_id}", axum::routing::delete(delete_calendar))
            .with_state(Arc::clone(&state));

        let address = super::serve_loopback(router).await;

        Self {
            base_url: format!("http://{address}"),
            state,
        }
    }

    /// Serve list calls `n` items per page, with a `nextPageToken`
    /// while more follow, the way Google does past its page size.
    pub fn page_by(&self, n: usize) {
        self.state.page_size.store(n, Ordering::SeqCst);
    }

    /// Script the next `n` requests to fail with a transient error.
    pub fn fail_next(&self, n: usize) {
        self.state.fail_times.store(n, Ordering::SeqCst);
    }

    /// Script the next `n` requests to be permanently rejected.
    pub fn reject_next(&self, n: usize) {
        self.state.reject_times.store(n, Ordering::SeqCst);
    }

    /// Seed an event so an upsert lookup finds one and updates instead
    /// of creating.
    pub async fn seed(&self, calendar_id: &str, event: Value) {
        self.state
            .existing
            .lock()
            .await
            .entry(calendar_id.to_string())
            .or_default()
            .push(event);
    }
}
