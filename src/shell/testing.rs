//! Test-only stubs for the two systems Almanac talks to (T0).
//!
//! Before this existed there was no way to stand between Almanac and
//! Google, so the retry loop, the per-profile calendar routing and a
//! successful token refresh were all unreachable for any test that did
//! not use Kenny's real calendar. That is why three Essential features
//! leaned entirely on two `#[ignore]`d live tests — and why a
//! connection failure sat misclassified inside the retry loop until an
//! audit read it line by line.
//!
//! Both stubs are real HTTP servers on a loopback port, so the code
//! under test performs genuine requests: no mocked client, no
//! behaviour that exists only in tests.

#![cfg(test)]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::{Value, json};
use tokio::sync::Mutex;

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
    /// Events the stub already holds, keyed by calendar id, returned
    /// from the list endpoint so an upsert can find one.
    pub existing: Mutex<HashMap<String, Vec<Value>>>,
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
}

/// A stand-in for the Google Calendar API.
pub struct CalendarStub {
    pub base_url: String,
    pub state: Arc<CalendarState>,
}

/// The 503 body Google actually sends, so `extract_reason` sees the
/// shape it parses in production rather than an empty string.
fn transient_body() -> Value {
    json!({"error": {"errors": [{"reason": "backendError"}], "message": "backend error"}})
}

/// Google's permission-denied 403 — the same status code it uses for
/// rate limiting, which is exactly why the reason string matters.
fn permanent_body() -> Value {
    json!({"error": {"errors": [{"reason": "permissionDenied"}],
                     "message": "the service account cannot access this calendar"}})
}

async fn list_events(
    State(state): State<Arc<CalendarState>>,
    Path(calendar_id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<HashMap<String, String>>,
) -> (StatusCode, axum::Json<Value>) {
    state.record("GET", &calendar_id).await;
    if state.take_rejection() {
        return (StatusCode::FORBIDDEN, axum::Json(permanent_body()));
    }
    if state.take_failure() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(transient_body()),
        );
    }

    // Google filters on `privateExtendedProperty=key=value`, and so
    // must this: a stub that returns everything regardless of the
    // query would make an upsert lookup — and the delete that shares
    // it — appear to work while matching the wrong event, which is
    // precisely the cross-source bug the tests are meant to catch.
    let filter = query.get("privateExtendedProperty").cloned();
    let existing = state.existing.lock().await;
    let items: Vec<Value> = existing
        .get(&calendar_id)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|event| match &filter {
            None => true,
            Some(filter) => {
                let Some((key, value)) = filter.split_once('=') else {
                    return false;
                };
                event["extendedProperties"]["private"][key] == json!(value)
            }
        })
        .collect();

    (StatusCode::OK, axum::Json(json!({"items": items})))
}

async fn create_event(
    State(state): State<Arc<CalendarState>>,
    Path(calendar_id): Path<String>,
    axum::Json(body): axum::Json<Value>,
) -> (StatusCode, axum::Json<Value>) {
    state.record("POST", &calendar_id).await;
    if state.take_rejection() {
        return (StatusCode::FORBIDDEN, axum::Json(permanent_body()));
    }
    if state.take_failure() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(transient_body()),
        );
    }

    let id = format!("event-{}", state.next_id.fetch_add(1, Ordering::SeqCst));
    state.created.lock().await.push((calendar_id, body.clone()));

    let mut created = body;
    created["id"] = json!(id);
    (StatusCode::OK, axum::Json(created))
}

async fn update_event(
    State(state): State<Arc<CalendarState>>,
    Path((calendar_id, event_id)): Path<(String, String)>,
    axum::Json(body): axum::Json<Value>,
) -> (StatusCode, axum::Json<Value>) {
    state.record("PUT", &calendar_id).await;
    if state.take_rejection() {
        return (StatusCode::FORBIDDEN, axum::Json(permanent_body()));
    }
    if state.take_failure() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(transient_body()),
        );
    }

    let mut updated = body;
    updated["id"] = json!(event_id);
    (StatusCode::OK, axum::Json(updated))
}

async fn delete_event(
    State(state): State<Arc<CalendarState>>,
    Path((calendar_id, _event_id)): Path<(String, String)>,
) -> StatusCode {
    state.record("DELETE", &calendar_id).await;
    if state.take_rejection() {
        return StatusCode::FORBIDDEN;
    }
    if state.take_failure() {
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    StatusCode::NO_CONTENT
}

async fn create_calendar(
    State(state): State<Arc<CalendarState>>,
    axum::Json(body): axum::Json<Value>,
) -> (StatusCode, axum::Json<Value>) {
    state.record("POST", "<new calendar>").await;
    let id = format!("calendar-{}", state.next_id.fetch_add(1, Ordering::SeqCst));
    state.calendars.lock().await.insert(
        id.clone(),
        Created {
            name: body["summary"].as_str().unwrap_or_default().to_string(),
            shared_with: Vec::new(),
        },
    );
    (StatusCode::OK, axum::Json(json!({"id": id})))
}

async fn insert_acl(
    State(state): State<Arc<CalendarState>>,
    Path(calendar_id): Path<String>,
    axum::Json(rule): axum::Json<Value>,
) -> (StatusCode, axum::Json<Value>) {
    state.record("ACL", &calendar_id).await;
    if let Some(created) = state.calendars.lock().await.get_mut(&calendar_id)
        && let Some(who) = rule["scope"]["value"].as_str()
    {
        created.shared_with.push(who.to_string());
    }
    (StatusCode::OK, axum::Json(rule))
}

impl CalendarStub {
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
            .route("/{calendar_id}/acl", axum::routing::post(insert_acl))
            .with_state(Arc::clone(&state));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.ok();
        });

        Self {
            base_url: format!("http://{address}"),
            state,
        }
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

/// A stand-in for Google's OAuth2 token endpoint.
pub struct TokenStub {
    pub url: String,
    pub state: Arc<TokenStubState>,
}

#[derive(Default)]
pub struct TokenStubState {
    /// How many token requests arrived. The number that proves AR18:
    /// twenty concurrent callers must produce one.
    pub hits: AtomicUsize,
    /// Seconds each issued token is valid for.
    pub expires_in: AtomicUsize,
}

impl TokenStubState {
    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

async fn issue_token(State(state): State<Arc<TokenStubState>>) -> axum::Json<Value> {
    let n = state.hits.fetch_add(1, Ordering::SeqCst);
    let expires_in = state.expires_in.load(Ordering::SeqCst);
    // A different token each time, so a test can tell a reused token
    // from a refreshed one.
    axum::Json(json!({"access_token": format!("stub-token-{n}"), "expires_in": expires_in}))
}

impl TokenStub {
    /// Starts the stub. `expires_in` is what it reports for each
    /// issued token — pass something small to make a token expire
    /// without waiting.
    pub async fn start(expires_in: usize) -> Self {
        let state = Arc::new(TokenStubState::default());
        state.expires_in.store(expires_in, Ordering::SeqCst);

        let router = Router::new()
            .route("/token", axum::routing::post(issue_token))
            .with_state(Arc::clone(&state));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.ok();
        });

        Self {
            url: format!("http://{address}/token"),
            state,
        }
    }
}

/// A throwaway RSA key generated for this test suite alone. It signs
/// JWTs that only ever reach the local stub above, and grants access to
/// nothing. Kept here rather than duplicated per test file.
pub const TEST_ONLY_THROWAWAY_KEY: &str = crate::shell::auth::tests::TEST_ONLY_THROWAWAY_KEY;

/// Credentials pointing at a token stub, signed with that key.
pub fn stub_credentials(token_url: &str) -> crate::core::auth::ServiceAccountCredentials {
    crate::core::auth::ServiceAccountCredentials {
        client_email: "stub@example.iam.gserviceaccount.com".to_string(),
        private_key: TEST_ONLY_THROWAWAY_KEY.to_string(),
        token_url: token_url.to_string(),
    }
}

/// A stand-in for Home Assistant's homelab-ops webhook, so a test can
/// assert what Almanac actually reported rather than only that it did
/// not crash.
pub struct NotifyStub {
    pub url: String,
    pub events: Arc<Mutex<Vec<Value>>>,
}

async fn record_event(
    State(events): State<Arc<Mutex<Vec<Value>>>>,
    axum::Json(body): axum::Json<Value>,
) -> StatusCode {
    events.lock().await.push(body);
    StatusCode::OK
}

impl NotifyStub {
    pub async fn start() -> Self {
        let events = Arc::new(Mutex::new(Vec::new()));

        let router = Router::new()
            .route("/webhook", axum::routing::post(record_event))
            .with_state(Arc::clone(&events));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.ok();
        });

        Self {
            url: format!("http://{address}/webhook"),
            events,
        }
    }

    pub async fn count(&self) -> usize {
        self.events.lock().await.len()
    }

    /// The `op` field of every event received, in order.
    pub async fn ops(&self) -> Vec<String> {
        self.events
            .lock()
            .await
            .iter()
            .filter_map(|e| e["op"].as_str().map(str::to_string))
            .collect()
    }
}

/// The public half of a throwaway minisign key. Its secret half was
/// generated once, used to sign the fixtures in `tests/self_update.rs`,
/// and never kept. It is not the release key and grants nothing.
pub const THROWAWAY_RELEASE_PUBKEY: &str =
    "RWSD7EDZN4XNRaGibu+cfLqrMzCOC0pAyW/CCeNTg5A1BcZMfHalxTx4";

/// A stand-in for the release host.
pub struct ReleaseStub {
    pub base_url: String,
    /// How many times a running instance asked what the latest version
    /// is — the signal that a check actually happened.
    pub version_requests: Arc<AtomicUsize>,
}

impl ReleaseStub {
    pub fn checks(&self) -> usize {
        self.version_requests.load(Ordering::SeqCst)
    }
}

impl ReleaseStub {
    /// Serves a release whose manifest does not match its signature —
    /// the shape of a compromised release host, and what the
    /// verification-failure threshold counts.
    pub async fn start_unverifiable() -> Self {
        async fn version(State(count): State<Arc<AtomicUsize>>) -> &'static str {
            count.fetch_add(1, Ordering::SeqCst);
            "0.2.0"
        }
        async fn manifest() -> &'static str {
            // One byte different from what was signed.
            "306d6ca7407560340797866e077e053627ad409277d1b9da58106fce4cf717cb  almanac\n"
        }
        async fn signature() -> &'static str {
            "untrusted comment: signature from minisign secret key\nRUSD7EDZN4XNRWzTd/nVYZPGWVHFhAdBbUgEuUgnG8PNvVN24ZFFhkXVQRLam6HmQw9bcUwAMGbUi7Rgew5LWC0DGlLbmbcy9g8=\ntrusted comment: timestamp:1787940761\tfile:SHA256SUMS\thashed\nsR1goffxRR5oeoS6no0+GjFuKrSt4UannaugGwWSe5Ahv3f5bAmd7nCCRA/a/sYW5TO00kNiMYb2yXKigUm5CA==\n"
        }

        let count = Arc::new(AtomicUsize::new(0));
        let router = Router::new()
            .route("/download/v0.2.0/SHA256SUMS", axum::routing::get(manifest))
            .route(
                "/download/v0.2.0/SHA256SUMS.minisig",
                axum::routing::get(signature),
            )
            .route(
                "/latest/download/VERSION",
                axum::routing::get(version).with_state(Arc::clone(&count)),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.ok();
        });

        Self {
            base_url: format!("http://{address}"),
            version_requests: count,
        }
    }
}
