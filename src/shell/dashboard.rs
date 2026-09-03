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

    let loaded = state.profiles();
    let mut profiles: Vec<_> = loaded.values().collect();
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

/// The dropdown value that means "make a new one". Not a calendar id
/// Google could ever issue, so it cannot collide with a real choice.
const NEW_CALENDAR: &str = "__new__";

async fn sources_page(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !is_logged_in(&state, &headers).await {
        return Redirect::to("/login").into_response();
    }
    render_sources(&state, None, None).await
}

/// Renders the page, optionally with an error and the values that
/// caused it. Keeping what was typed is the whole point of re-rendering
/// rather than redirecting: a rejected form that empties itself costs
/// the typing as well as the mistake.
async fn render_sources(
    state: &AppState,
    error: Option<&str>,
    draft: Option<(&str, &str, &str)>,
) -> Response {
    let issued: HashMap<String, String> = state.tokens.list().await.into_iter().collect();
    let loaded = state.profiles();
    let mut profiles: Vec<_> = loaded.values().collect();
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
    <button class="btn btn-sm btn-outline-secondary" type="submit">Revoke token</button>
  </form>
  <form method="post" action="/dashboard/sources/{id}/retire" class="d-inline">
    <button class="btn btn-sm btn-outline-danger" type="submit">Retire</button>
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
  <form method="post" action="/dashboard/sources/{id}/retire" class="d-inline">
    <button class="btn btn-sm btn-outline-danger" type="submit">Retire</button>
  </form>
</td></tr>"#
                ),
            }
        })
        .collect();

    // Retired sources keep their row, the way kyu keeps a revoked
    // app's: a source that vanished without trace is indistinguishable
    // from one that was never there, and the question three months
    // later is always "did we have one of these?".
    let retired_rows: String = crate::shell::profiles::retired(&state.profiles_dir)
        .into_iter()
        .map(|id| {
            let id = escape(&id);
            format!(
                r#"<tr class="text-secondary">
<td><code>{id}</code></td>
<td><span class="badge text-bg-secondary">retired</span></td>
<td class="text-end"><span class="small">profile kept on disk</span></td>
</tr>"#
            )
        })
        .collect();
    let rows = format!("{rows}{retired_rows}");

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
// Reveals the name box only when "+ New calendar" is chosen, and runs
// once on load so a re-rendered form after an error comes back with the
// box already open.
function toggleNewCalendar() {
  const select = document.getElementById('calendar');
  const row = document.getElementById('new-calendar-row');
  if (!select || !row) { return; }
  row.classList.toggle('d-none', select.value !== '__new__');
}
document.addEventListener('DOMContentLoaded', toggleNewCalendar);
function selectAll(node) {
  const range = document.createRange();
  range.selectNodeContents(node);
  const sel = window.getSelection();
  sel.removeAllRanges();
  sel.addRange(range);
}
// Copies without assuming the page is in a secure context.
//
// navigator.clipboard exists ONLY in a secure context — https, or
// localhost. This dashboard is served over plain HTTP on the LAN, which
// is neither, so the object is simply absent and the button used to die
// with "navigator.clipboard is undefined". It could never have worked in
// the only way this page is ever opened, and nothing said so: the error
// appeared in the browser console, not on the page.
//
// So: the modern API when it is really there, the deprecated but
// http-friendly execCommand next, and failing both, put the command on
// screen already selected so it can be copied by hand.
function copyText(text) {
  if (window.isSecureContext && navigator.clipboard) {
    return navigator.clipboard.writeText(text).then(() => true, () => false);
  }
  const ta = document.createElement('textarea');
  ta.value = text;
  ta.setAttribute('readonly', '');
  ta.style.position = 'fixed';
  ta.style.opacity = '0';
  document.body.appendChild(ta);
  ta.select();
  let ok = false;
  try { ok = document.execCommand('copy'); } catch (e) { ok = false; }
  document.body.removeChild(ta);
  return Promise.resolve(ok);
}
async function copyCmd(id) {
  const pre = document.getElementById(`pre-${id}`);
  const row = document.getElementById(`out-${id}`);
  let hideAfter = 4000;
  try {
    const token = await fetchToken(id);
    const cmd = `curl -X POST ${location.origin}/v1/ingest/${id} \\\n  -H 'Authorization: Bearer ${token}' \\\n  -H 'Content-Type: application/json' \\\n  -d '{"title":"test","start":"2026-01-01T09:00:00+00:00"}'`;
    if (await copyText(cmd)) {
      pre.textContent = 'Command copied to the clipboard (token not shown).';
    } else {
      // The token is on screen now, so it gets the same treatment as
      // Reveal: visible long enough to use, then gone.
      pre.textContent = cmd;
      row.classList.remove('d-none');
      selectAll(pre);
      hideAfter = 20000;
    }
  } catch (e) { pre.textContent = e.message; }
  row.classList.remove('d-none');
  setTimeout(() => { pre.textContent = ''; row.classList.add('d-none'); }, hideAfter);
}
</script>"#;

    let alert = error
        .map(|e| {
            format!(
                r#"<div class="alert alert-danger" role="alert"><pre class="mb-0 small">{}</pre></div>"#,
                escape(e)
            )
        })
        .unwrap_or_default();

    let (draft_source, draft_calendar, draft_new) = draft.unwrap_or(("", "", ""));
    let draft_source = escape(draft_source);
    let draft_calendar = escape(draft_calendar);
    let draft_new_calendar = escape(draft_new);
    let profiles_dir = escape(&state.profiles_dir.display().to_string());

    // Without an owner a created calendar would belong to the service
    // account and be visible to nobody, so the form says so rather than
    // offering to make one.
    let can_create = state.calendar_owner.is_some();

    // Fetched on render so the dropdown shows what actually exists. A
    // failure here must not take the page down with it: the token
    // controls below are what someone came for when Google is
    // unreachable.
    let (calendars, calendar_error) = match state.client.list_calendars().await {
        Ok(calendars) => (calendars, None),
        Err(e) => (Vec::new(), Some(e.to_string())),
    };

    let options: String = calendars
        .iter()
        .map(|(id, name)| {
            let selected = if *id == *draft_calendar {
                " selected"
            } else {
                ""
            };
            format!(
                r#"<option value="{}"{selected}>{}</option>"#,
                escape(id),
                escape(name)
            )
        })
        .collect();
    let new_option = if can_create {
        let selected = if draft_calendar == NEW_CALENDAR {
            " selected"
        } else {
            ""
        };
        format!(r#"<option value="{NEW_CALENDAR}"{selected}>+ New calendar…</option>"#)
    } else {
        String::new()
    };
    let calendar_note = match (&calendar_error, can_create) {
        (Some(e), _) => format!(
            r#"<div class="form-text text-danger">Could not read the calendar list: {}</div>"#,
            escape(e)
        ),
        (None, true) => String::from(
            r#"<div class="form-text">A new one is created and shared with you straight away.</div>"#,
        ),
        (None, false) => String::from(
            r#"<div class="form-text">ALMANAC_CALENDAR_OWNER is not set, so no new calendar can be created — one nobody can see is worse than none.</div>"#,
        ),
    };

    page(
        "Sources",
        "sources",
        &format!(
            r#"<h1 class="h4 mb-1">Sources</h1>
<p class="text-secondary">
  A source is two things: a <em>profile</em> saying which calendar its events land on and
  which part of its payload means what, and a token it identifies itself with. Each token
  opens only its own source, so revoking one leaves the others working.
</p>

{alert}

<div class="card mb-4"><div class="card-body">
  <h2 class="h6 card-title">Add a source</h2>
  <p class="text-secondary small">
    Two things: what the source is called, and which calendar its events land on.
    Everything else gets Almanac's plain shape — a payload carrying
    <code>title</code>, <code>start</code> and <code>external_id</code>, and optionally
    <code>description</code> and <code>location</code>. That third one is what makes
    resending update an event instead of adding a second, and it is the only handle the
    delete endpoint has. It takes effect immediately; no restart.
  </p>
  <form method="post" action="/dashboard/sources" class="row g-2 align-items-end">
    <div class="col-sm-4">
      <label class="form-label" for="source_id">Source name</label>
      <input type="text" class="form-control" id="source_id" name="source_id"
             value="{draft_source}" placeholder="kobo" required>
      <div class="form-text">Letters, digits, dots, hyphens, underscores.</div>
    </div>
    <div class="col-sm-5">
      <label class="form-label" for="calendar">Calendar</label>
      <select class="form-select" id="calendar" name="calendar" onchange="toggleNewCalendar()" required>
        {options}{new_option}
      </select>
      {calendar_note}
      <div id="new-calendar-row" class="mt-2 d-none">
        <label class="form-label" for="new_calendar">Name for the new calendar</label>
        <input type="text" class="form-control" id="new_calendar" name="new_calendar"
               value="{draft_new_calendar}" placeholder="Almanac · Huishouden">
      </div>
    </div>
    <div class="col-sm-auto">
      <button class="btn btn-primary" type="submit">Add source</button>
    </div>
  </form>
  <p class="text-secondary small mt-3 mb-0">
    Written to <code>{profiles_dir}/&lt;source name&gt;.toml</code>. Anything the plain shape
    cannot express — a webhook that sends <code>monitor.name</code> instead of
    <code>title</code>, a colour per severity, an all-day event — is a line in that file;
    edit it there and press <em>Reload profiles from disk</em>.
  </p>
</div></div>

<h2 class="h5">Registered</h2>
<div class="card mb-4"><div class="card-body">
<div class="table-responsive"><table class="table table-sm align-middle mb-0">
<thead><tr><th>Source</th><th>Token issued</th><th class="text-end">Actions</th></tr></thead>
<tbody>{rows}</tbody></table></div>
</div></div>

<form method="post" action="/dashboard/sources/reload" class="mb-4">
  <button class="btn btn-sm btn-outline-secondary" type="submit">Reload profiles from disk</button>
  <span class="text-secondary small ms-2">
    for a profile placed on the machine by hand, rather than through the box above
  </span>
</form>

<div class="alert alert-secondary" role="alert">
  <h2 class="h6">Where these live, and what survives</h2>
  <p class="mb-0">
    <b>Revoke token</b> takes a source's key away and leaves everything else; issue it a new
    one and it works again. <b>Retire</b> ends the source: its token is revoked and its profile
    is renamed out of the loaded set, keeping the file — and the row above — as the record that
    it existed. Neither touches events already on the calendar; those are the calendar's now.
  </p>
  <p class="mb-0">
    Profiles are plain files in <code>{profiles_dir}</code>, which is the directory the
    homelab declares as almanac's data and backs up nightly. An update replaces the binary
    somewhere else entirely and never touches them. Adding a source here is the same act as
    writing the file by hand — this page just saves you the trip and the restart.
  </p>
</div>
{script}"#
        ),
    )
    .into_response()
}

/// What the add-a-source form sends (K21).
///
/// Two fields rather than a profile, after Kenny opened the first
/// version and asked for "enkel een naam van de bron en de naam van de
/// target kalender" — then refined it once more: the calendar is a
/// dropdown of what exists, with one entry that means "make a new one"
/// and reveals a box for its name. Picking a calendar should not
/// require knowing an id, and adding one should not require leaving
/// the page.
#[derive(Deserialize)]
struct NewSource {
    source_id: String,
    /// A calendar id from the dropdown, or `NEW_CALENDAR`.
    calendar: String,
    /// The name to create, when `calendar` says to make one.
    #[serde(default)]
    new_calendar: String,
}

/// `POST /dashboard/sources` — resolve or create the calendar, write
/// the profile, reload (K21).
async fn create_source(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(form): Form<NewSource>,
) -> Response {
    if !is_logged_in(&state, &headers).await {
        return Redirect::to("/login").into_response();
    }

    let source_id = form.source_id.trim();
    let chosen = form.calendar.trim();
    let new_name = form.new_calendar.trim();
    let draft = Some((source_id, chosen, new_name));

    // Checked here as well as in the parser: this one names the file
    // AND becomes a URL segment, and the message should point at the
    // field the person just typed rather than at a TOML they never saw.
    if !crate::core::profile::source_id_is_safe(source_id) {
        return render_sources(
            &state,
            Some(&format!(
                "\"{source_id}\" cannot be a source name — use letters, digits, '.', '-' and '_', and do not start with a dot."
            )),
            draft,
        )
        .await;
    }

    // Resolved before anything is written: a profile pointing at a
    // calendar that does not exist accepts payloads and fails every
    // delivery, which looks like Google being down.
    let calendar_id = if chosen == NEW_CALENDAR {
        let Some(owner) = state.calendar_owner.as_deref() else {
            return render_sources(
                &state,
                Some(
                    "ALMANAC_CALENDAR_OWNER is not set — without an owner to share it with, a calendar \
                     Almanac creates belongs to the service account and is visible to nobody. Set that \
                     variable, or pick a calendar that already exists.",
                ),
                draft,
            )
            .await;
        };
        if new_name.is_empty() {
            return render_sources(&state, Some("Name the new calendar."), draft).await;
        }
        // Still find-or-create rather than a bare create: two tabs, or
        // a second source added to a calendar made a minute ago, must
        // not each get their own. A duplicate calendar is close to
        // invisible — events land, nothing errors, and half of them are
        // on a calendar nobody has open.
        match state.client.ensure_calendar(new_name, owner).await {
            Ok((id, created)) => {
                if created {
                    tracing::info!(calendar = %new_name, id = %id, "created a calendar from the dashboard");
                }
                id
            }
            Err(e) => return render_sources(&state, Some(&e.to_string()), draft).await,
        }
    } else if chosen.is_empty() {
        return render_sources(&state, Some("Choose a calendar for this source."), draft).await;
    } else {
        chosen.to_string()
    };

    let toml = crate::core::profile::default_profile_toml(source_id, &calendar_id);
    if let Err(e) = crate::shell::profiles::save_new(&state.profiles_dir, &toml) {
        return render_sources(&state, Some(&e.to_string()), draft).await;
    }

    // Reload from disk rather than inserting the parsed profile: the
    // set is what has to stay valid, and reading it back is also the
    // only proof that what was written can be read again.
    match crate::shell::profiles::load_map(&state.profiles_dir) {
        Ok(profiles) => {
            state.set_profiles(profiles);
            tracing::info!(source_id = %source_id, "added a source from the dashboard");
            Redirect::to("/dashboard/sources").into_response()
        }
        Err(e) => render_sources(&state, Some(&e.to_string()), draft).await,
    }
}

/// `POST /dashboard/sources/{source_id}/retire` — end a source (K21).
///
/// Revokes its token and renames its profile out of the loaded set,
/// keeping the file. Modelled on kyu's app revocation, which keeps the
/// row rather than erasing it.
async fn retire_source(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !is_logged_in(&state, &headers).await {
        return Redirect::to("/login").into_response();
    }
    if !state.profiles().contains_key(&source_id) {
        return (StatusCode::NOT_FOUND, "no such source").into_response();
    }

    // Refused while anything of this source's is still waiting to be
    // delivered. The worker needs the profile to know which calendar
    // an entry belongs to; retiring it first would leave those entries
    // in the journal forever, logging an error on every pass and
    // deliverable by nothing. The journal never drops an entry, so the
    // only honest answer is "not yet".
    let waiting = match state.journal.pending() {
        Ok(pending) => pending
            .iter()
            .filter(|entry| entry.source_id == source_id)
            .count(),
        Err(e) => {
            return render_sources(&state, Some(&e.to_string()), None).await;
        }
    };
    if waiting > 0 {
        let message = format!(
            "{source_id} still has {waiting} event(s) waiting to be delivered. \
             Retiring it now would leave them in the journal with no profile to deliver them by. \
             Wait for the queue to drain, or fix whatever is blocking delivery, and retire it then."
        );
        return render_sources(&state, Some(&message), None).await;
    }

    if let Err(e) = state.tokens.revoke(&source_id).await {
        return render_sources(&state, Some(&e.to_string()), None).await;
    }

    let kept = match crate::shell::profiles::retire(&state.profiles_dir, &source_id) {
        Ok(path) => path,
        Err(e) => return render_sources(&state, Some(&e.to_string()), None).await,
    };

    match crate::shell::profiles::load_map(&state.profiles_dir) {
        Ok(profiles) => {
            state.set_profiles(profiles);
            tracing::info!(
                source_id = %source_id,
                kept = %kept.display(),
                "retired a source from the dashboard"
            );
            Redirect::to("/dashboard/sources").into_response()
        }
        Err(e) => render_sources(&state, Some(&e.to_string()), None).await,
    }
}

/// `POST /dashboard/sources/reload` — re-read the profiles directory
/// (K21). Free once profiles are swappable, and it is what makes a
/// profile placed by hand usable without a restart.
async fn reload_profiles(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !is_logged_in(&state, &headers).await {
        return Redirect::to("/login").into_response();
    }

    match crate::shell::profiles::load_map(&state.profiles_dir) {
        Ok(profiles) => {
            tracing::info!(
                count = profiles.len(),
                "reloaded profiles from the dashboard"
            );
            state.set_profiles(profiles);
            Redirect::to("/dashboard/sources").into_response()
        }
        Err(e) => render_sources(&state, Some(&e.to_string()), None).await,
    }
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
    if !state.profiles().contains_key(&source_id) {
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
        .route(
            "/dashboard/sources",
            axum::routing::get(sources_page).post(create_source),
        )
        .route(
            "/dashboard/sources/reload",
            axum::routing::post(reload_profiles),
        )
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
            "/dashboard/sources/{source_id}/retire",
            axum::routing::post(retire_source),
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
