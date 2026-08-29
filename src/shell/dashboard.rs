//! The management dashboard (M12), modelled on `kyu`'s W2 so both
//! services are administered the same way.
//!
//! Login uses the bootstrap token from the environment and a session
//! cookie — not browser basic auth, which has no way to log out of
//! short of clearing browser data. Sessions live in the encrypted
//! store (AR25) so a restart or a self-update does not log Kenny out,
//! while logout stays a real server-side removal: a copied cookie
//! stops working, which a self-validating cookie could not offer.
//!
//! The UI is English per standing rule 1 (Dutch is for conversation and
//! for dashboards meant for Kenny's parents; this is an operator tool).

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::{Form, Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::core::html::escape;
use crate::core::token::verify_token;
use crate::shell::admin::CAPTURE_TTL_SECS;
use crate::shell::ingest::AppState;

const SESSION_COOKIE: &str = "almanac_session";
/// A generous session life: this is a LAN tool behind a token, and
/// being logged out mid-debugging helps nobody.
const SESSION_TTL_SECS: u64 = 7 * 24 * 3600;

/// Reads the session cookie and says whether it names a live session.
async fn is_logged_in(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(cookie_header) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) else {
        return false;
    };

    let Some(presented) = cookie_header.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == SESSION_COOKIE).then_some(value)
    }) else {
        return false;
    };

    state
        .tokens
        .session_is_live(presented, (state.now_unix)())
        .await
}

fn page(title: &str, active: &str, body: &str) -> Html<String> {
    let nav_item = |href: &str, label: &str, key: &str| {
        let class = if key == active {
            "nav-link active"
        } else {
            "nav-link"
        };
        format!(r#"<a class="{class}" href="{href}">{label}</a>"#)
    };

    Html(format!(
        r#"<!doctype html>
<html lang="en" data-bs-theme="dark">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} — Almanac</title>
<link rel="stylesheet" href="/static/bootstrap.min.css">
</head>
<body class="bg-body">
<nav class="navbar navbar-expand bg-body-tertiary border-bottom mb-4">
  <div class="container">
    <span class="navbar-brand fw-semibold">Almanac</span>
    <div class="navbar-nav me-auto">
      {status}
      {sources}
      {captures}
    </div>
    <form method="post" action="/logout" class="m-0">
      <button class="btn btn-sm btn-outline-secondary" type="submit">Log out</button>
    </form>
  </div>
</nav>
<main class="container pb-5">
{body}
</main>
</body>
</html>"#,
        title = escape(title),
        status = nav_item("/dashboard", "Status", "status"),
        sources = nav_item("/dashboard/sources", "Sources", "sources"),
        captures = nav_item("/dashboard/captures", "Captures", "captures"),
        body = body,
    ))
}

fn login_page(error: Option<&str>) -> Html<String> {
    let alert = error
        .map(|e| format!(r#"<div class="alert alert-danger">{}</div>"#, escape(e)))
        .unwrap_or_default();

    Html(format!(
        r#"<!doctype html>
<html lang="en" data-bs-theme="dark">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Log in — Almanac</title>
<link rel="stylesheet" href="/static/bootstrap.min.css">
</head>
<body class="bg-body">
<main class="container" style="max-width: 26rem; margin-top: 6rem;">
  <h1 class="h4 mb-3">Almanac</h1>
  {alert}
  <form method="post" action="/login">
    <div class="mb-3">
      <label class="form-label" for="token">Bootstrap token</label>
      <input class="form-control" type="password" id="token" name="token" autofocus
             autocomplete="current-password">
      <div class="form-text">The value of ALMANAC_BOOTSTRAP_TOKEN.</div>
    </div>
    <button class="btn btn-primary w-100" type="submit">Log in</button>
  </form>
</main>
</body>
</html>"#
    ))
}

async fn root(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if is_logged_in(&state, &headers).await {
        Redirect::to("/dashboard").into_response()
    } else {
        Redirect::to("/login").into_response()
    }
}

async fn login_form() -> Response {
    login_page(None).into_response()
}

#[derive(Deserialize)]
pub struct LoginBody {
    token: String,
}

async fn login_submit(State(state): State<Arc<AppState>>, Form(body): Form<LoginBody>) -> Response {
    let Some(expected) = &state.bootstrap_token_hash else {
        return login_page(Some(
            "No bootstrap token is configured. Set ALMANAC_BOOTSTRAP_TOKEN and restart.",
        ))
        .into_response();
    };

    if !verify_token(body.token.trim(), expected) {
        tracing::warn!("a dashboard login attempt used the wrong token");
        return login_page(Some("That token is not correct.")).into_response();
    }

    let mut id_bytes = [0u8; 32];
    if getrandom::fill(&mut id_bytes).is_err() {
        return login_page(Some(
            "Could not start a session; the OS refused randomness.",
        ))
        .into_response();
    }
    let session_id = hex::encode(id_bytes);

    let now = (state.now_unix)();
    if let Err(e) = state
        .tokens
        .start_session(&session_id, now + SESSION_TTL_SECS, now)
        .await
    {
        tracing::error!(error = %e, "failed to persist a dashboard session");
        return login_page(Some("Could not start a session; see the logs.")).into_response();
    }

    // HttpOnly so script cannot read it; SameSite=Strict so another
    // site cannot ride the session. Not Secure: the default deployment
    // terminates TLS at Traefik, and the documented fallback is plain
    // HTTP direct to the LXC (AR17) — marking it Secure would silently
    // break login exactly when the fallback is needed.
    let cookie = format!(
        "{SESSION_COOKIE}={session_id}; Path=/; HttpOnly; SameSite=Strict; Max-Age={SESSION_TTL_SECS}"
    );

    (
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, cookie),
            (header::LOCATION, "/dashboard".to_string()),
        ],
    )
        .into_response()
}

async fn logout(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Some(cookie_header) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok())
        && let Some(presented) = cookie_header.split(';').find_map(|part| {
            let (name, value) = part.trim().split_once('=')?;
            (name == SESSION_COOKIE).then_some(value.to_string())
        })
        && let Err(e) = state.tokens.end_session(&presented).await
    {
        tracing::warn!(error = %e, "failed to remove a session from the store");
    }

    (
        StatusCode::SEE_OTHER,
        [
            (
                header::SET_COOKIE,
                format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
            ),
            (header::LOCATION, "/login".to_string()),
        ],
    )
        .into_response()
}

async fn status_page(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !is_logged_in(&state, &headers).await {
        return Redirect::to("/login").into_response();
    }

    let mut profiles: Vec<_> = state.profiles.values().collect();
    profiles.sort_by(|a, b| a.source_id.cmp(&b.source_id));

    let profile_rows: String = profiles
        .iter()
        .map(|p| {
            format!(
                "<tr><td><code>{}</code></td><td>{}</td><td>{}</td></tr>",
                escape(&p.source_id),
                escape(&p.target_calendar_id),
                p.schema_version
            )
        })
        .collect();

    let journal = match state.journal.pending() {
        Ok(pending) => format!(
            r#"<p class="mb-0">{} entr{} waiting to be delivered.</p>"#,
            pending.len(),
            if pending.len() == 1 { "y" } else { "ies" }
        ),
        Err(e) => format!(
            r#"<div class="alert alert-danger mb-0"><strong>{}</strong><br>{}</div>"#,
            escape(&e.to_string()),
            escape(e.remedy())
        ),
    };

    let route_rows: String = state
        .routes
        .lock()
        .await
        .iter()
        .map(|r| {
            let (badge, detail) = match &r.outcome {
                crate::core::observability::RouteOutcome::Created { event_id } => (
                    r#"<span class="badge text-bg-success">created</span>"#.to_string(),
                    escape(event_id),
                ),
                crate::core::observability::RouteOutcome::Updated { event_id } => (
                    r#"<span class="badge text-bg-primary">updated</span>"#.to_string(),
                    escape(event_id),
                ),
                crate::core::observability::RouteOutcome::Failed { message, remedy } => (
                    r#"<span class="badge text-bg-danger">failed</span>"#.to_string(),
                    format!("{}<br><em>{}</em>", escape(message), escape(remedy)),
                ),
            };
            format!(
                "<tr><td class=\"text-nowrap\">{}</td><td><code>{}</code></td><td>{}</td><td>{}</td></tr>",
                escape(&r.at),
                escape(&r.source_id),
                badge,
                detail
            )
        })
        .collect();

    let route_table = if route_rows.is_empty() {
        r#"<p class="text-secondary mb-0">Nothing delivered yet this run.</p>"#.to_string()
    } else {
        format!(
            r#"<div class="table-responsive"><table class="table table-sm align-middle">
<thead><tr><th>When</th><th>Source</th><th>Result</th><th>Detail</th></tr></thead>
<tbody>{route_rows}</tbody></table></div>"#
        )
    };

    page(
        "Status",
        "status",
        &format!(
            r#"<h1 class="h4 mb-4">Status</h1>
<div class="card mb-4"><div class="card-body">
  <h2 class="h6 text-secondary">Journal</h2>{journal}
</div></div>
<div class="card mb-4"><div class="card-body">
  <h2 class="h6 text-secondary">Loaded profiles</h2>
  <div class="table-responsive"><table class="table table-sm align-middle mb-0">
  <thead><tr><th>Source</th><th>Target calendar</th><th>Schema</th></tr></thead>
  <tbody>{profile_rows}</tbody></table></div>
</div></div>
<div class="card"><div class="card-body">
  <h2 class="h6 text-secondary">Recent deliveries</h2>{route_table}
</div></div>"#
        ),
    )
    .into_response()
}

async fn sources_page(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !is_logged_in(&state, &headers).await {
        return Redirect::to("/login").into_response();
    }

    let issued: HashMap<String, String> = state.tokens.list().await.into_iter().collect();
    let mut profiles: Vec<_> = state.profiles.values().collect();
    profiles.sort_by(|a, b| a.source_id.cmp(&b.source_id));

    let rows: String = profiles
        .iter()
        .map(|p| {
            let id = escape(&p.source_id);
            match issued.get(&p.source_id) {
                Some(when) => format!(
                    r#"<tr>
<td><code>{id}</code></td>
<td class="text-nowrap">{when}</td>
<td class="text-end">
  <button class="btn btn-sm btn-outline-secondary" onclick="reveal('{id}')">Reveal 10s</button>
  <button class="btn btn-sm btn-outline-secondary" onclick="copyCmd('{id}')">Copy command</button>
  <form method="post" action="/dashboard/sources/{id}/issue" class="d-inline">
    <button class="btn btn-sm btn-outline-warning" type="submit">Re-issue</button>
  </form>
  <form method="post" action="/dashboard/sources/{id}/revoke" class="d-inline">
    <button class="btn btn-sm btn-outline-danger" type="submit">Revoke</button>
  </form>
</td></tr>
<tr id="out-{id}" class="d-none"><td colspan="3"><pre class="mb-0 small" id="pre-{id}"></pre></td></tr>"#,
                    when = escape(when)
                ),
                None => format!(
                    r#"<tr>
<td><code>{id}</code></td>
<td class="text-secondary">no token</td>
<td class="text-end">
  <form method="post" action="/dashboard/sources/{id}/issue" class="d-inline">
    <button class="btn btn-sm btn-primary" type="submit">Issue token</button>
  </form>
</td></tr>"#
                ),
            }
        })
        .collect();

    // The reveal and copy controls fetch the token only when clicked,
    // so a token never sits in the page source waiting to be read over
    // someone's shoulder or scraped out of a cached page.
    let script = r#"<script>
async function fetchToken(id) {
  const r = await fetch(`/dashboard/sources/${encodeURIComponent(id)}/token`);
  if (!r.ok) { throw new Error('could not fetch the token'); }
  return (await r.json()).token;
}
async function reveal(id) {
  const row = document.getElementById(`out-${id}`);
  const pre = document.getElementById(`pre-${id}`);
  try {
    pre.textContent = await fetchToken(id);
    row.classList.remove('d-none');
    setTimeout(() => { pre.textContent = ''; row.classList.add('d-none'); }, 10000);
  } catch (e) { pre.textContent = e.message; row.classList.remove('d-none'); }
}
async function copyCmd(id) {
  const pre = document.getElementById(`pre-${id}`);
  const row = document.getElementById(`out-${id}`);
  try {
    const token = await fetchToken(id);
    const cmd = `curl -X POST ${location.origin}/v1/ingest/${id} \\\n  -H 'Authorization: Bearer ${token}' \\\n  -H 'Content-Type: application/json' \\\n  -d '{"title":"test","start":"2026-01-01T09:00:00+00:00"}'`;
    await navigator.clipboard.writeText(cmd);
    pre.textContent = 'Command copied to the clipboard (token not shown).';
  } catch (e) { pre.textContent = e.message; }
  row.classList.remove('d-none');
  setTimeout(() => { pre.textContent = ''; row.classList.add('d-none'); }, 4000);
}
</script>"#;

    page(
        "Sources",
        "sources",
        &format!(
            r#"<h1 class="h4 mb-1">Sources</h1>
<p class="text-secondary">One bearer token per source. Revoking one leaves the others working.</p>
<div class="card"><div class="card-body">
<div class="table-responsive"><table class="table table-sm align-middle mb-0">
<thead><tr><th>Source</th><th>Token issued</th><th class="text-end">Actions</th></tr></thead>
<tbody>{rows}</tbody></table></div>
</div></div>
{script}"#
        ),
    )
    .into_response()
}

async fn captures_page(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !is_logged_in(&state, &headers).await {
        return Redirect::to("/login").into_response();
    }

    let captures = state.captures_after_expiry().await;

    let cards: String = captures
        .iter()
        .map(|c| {
            let headers_rows: String = c
                .headers
                .iter()
                .map(|(k, v)| {
                    format!(
                        "<tr><td class=\"text-secondary\">{}</td><td><code>{}</code></td></tr>",
                        escape(k),
                        escape(v)
                    )
                })
                .collect();
            let truncated = c
                .truncated_from_bytes
                .map(|n| {
                    format!(
                        r#"<div class="alert alert-warning py-1 px-2 small">Body shortened for display; the original was {n} bytes.</div>"#
                    )
                })
                .unwrap_or_default();

            format!(
                r#"<div class="card mb-3"><div class="card-body">
<div class="d-flex justify-content-between">
  <h2 class="h6 mb-2"><code>{label}</code></h2>
  <span class="text-secondary small">{at}</span>
</div>
{truncated}
<table class="table table-sm mb-2"><tbody>{headers_rows}</tbody></table>
<pre class="mb-0 small bg-body-tertiary p-2 rounded">{body}</pre>
</div></div>"#,
                label = escape(&c.label),
                at = escape(&c.at),
                headers_rows = headers_rows,
                body = escape(&c.body),
            )
        })
        .collect();

    let content = if cards.is_empty() {
        format!(
            r#"<p class="text-secondary">Nothing captured. Point an undocumented webhook at
<code>POST /v1/debug/capture/&lt;label&gt;</code> to see exactly what it sends.
Captures are kept for {} minutes.</p>
<p class="text-secondary"><b>That endpoint needs the admin token</b>, which is
also the token that logs in here and can reveal every source's token. Do not
paste it into a third-party system to make a capture work — use
<code>curl</code> from a machine you control, or replay the request yourself.</p>"#,
            CAPTURE_TTL_SECS / 60
        )
    } else {
        cards
    };

    page(
        "Captures",
        "captures",
        &format!(r#"<h1 class="h4 mb-3">Captured requests</h1>{content}"#),
    )
    .into_response()
}

async fn issue_token(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !is_logged_in(&state, &headers).await {
        return Redirect::to("/login").into_response();
    }
    if !state.profiles.contains_key(&source_id) {
        return (StatusCode::NOT_FOUND, "no such source").into_response();
    }

    let mut bytes = [0u8; 32];
    if getrandom::fill(&mut bytes).is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "no randomness available").into_response();
    }
    let token = hex::encode(bytes);

    match state.tokens.issue(&source_id, &token, &(state.now)()).await {
        Ok(()) => {
            tracing::info!(source_id = %source_id, "issued a token from the dashboard");
            Redirect::to("/dashboard/sources").into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to store an issued token");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

async fn revoke_token(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !is_logged_in(&state, &headers).await {
        return Redirect::to("/login").into_response();
    }

    match state.tokens.revoke(&source_id).await {
        Ok(existed) => {
            tracing::info!(source_id = %source_id, existed, "revoked a token from the dashboard");
            Redirect::to("/dashboard/sources").into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Hands the plaintext token to the logged-in page, on demand only.
async fn token_json(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !is_logged_in(&state, &headers).await {
        return (StatusCode::UNAUTHORIZED, "not logged in").into_response();
    }

    match state.tokens.reveal(&source_id).await {
        Ok(Some(token)) => Json(json!({"token": token})).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "no token for that source").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Serves the vendored Bootstrap CSS. Compiled into the binary so a
/// LAN-only service never needs the internet to render its own pages
/// (Kenny's choice, 2026-08-28) and the file cannot go missing from a
/// deployment.
async fn bootstrap_css() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../../static/bootstrap.min.css"),
    )
        .into_response()
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", axum::routing::get(root))
        .route("/login", axum::routing::get(login_form).post(login_submit))
        .route("/logout", axum::routing::post(logout))
        .route("/dashboard", axum::routing::get(status_page))
        .route("/dashboard/sources", axum::routing::get(sources_page))
        .route("/dashboard/captures", axum::routing::get(captures_page))
        .route(
            "/dashboard/sources/{source_id}/issue",
            axum::routing::post(issue_token),
        )
        .route(
            "/dashboard/sources/{source_id}/revoke",
            axum::routing::post(revoke_token),
        )
        .route(
            "/dashboard/sources/{source_id}/token",
            axum::routing::get(token_json),
        )
        .route(
            "/static/bootstrap.min.css",
            axum::routing::get(bootstrap_css),
        )
}
