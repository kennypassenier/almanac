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

/// The seven kp-themes palettes, once (K25).
///
/// The eleven house themes. A name and the Dutch label almanac gives it
/// — the package's own `js/theme-registry.js` labels them in English
/// since v3.0.0, and README's own framing is why almanac does not follow
/// that here: "the theme names — Kenny's names for his themes, not
/// interface chrome" stay as they are on this English-UI page, the way
/// a product name does. Nothing else lives in this list.
///
/// Everything else that used to sit here was a copy. The two swatch
/// colours per theme went first — a swatch wears the theme now, reading
/// its live tokens through `data-theme` on the span itself. The
/// dark/light flag went with them: `dark_themes()` below reads it out of
/// the vendored registry instead, so a theme that changes side upstream
/// cannot leave a hardcoded answer behind. (The kyu session made that
/// mistake first, believing in a fourth dark theme; there were three,
/// and now there are four, which is the point.)
///
/// The names and labels remain because almanac renders its markup on
/// the server: a picker that only exists after JavaScript has run is an
/// empty box on first paint.
///
/// `(name, label)`.
const THEMES: [(&str, &str); 11] = [
    ("formal", "Formeel"),
    ("light", "Licht"),
    ("dark", "Donker"),
    ("cyberpunk", "Cyberpunk"),
    ("pastel", "Pastel"),
    ("terminal", "Terminal"),
    ("topo", "Topografisch"),
    ("high-contrast", "Hoog contrast"),
    ("sepia", "Sepia"),
    ("blueprint", "Blauwdruk"),
    ("solstice", "Zonnewende"),
];

/// Everything the browser needs before the first paint, and the three
/// stylesheets in the order they must load: Bootstrap first, then the
/// theme tokens, then the bridge that points Bootstrap at them.
fn head_assets() -> String {
    // 3.0.0: under the kit's CSP (script-src 'self') nothing inline runs,
    // so the two head scripts are files served from /static, and the display
    // faces come from the kit's vendored fonts instead of a CDN (font-src
    // 'self'; works offline too). Order still matters: the no-flash script
    // before any stylesheet, the Bootstrap bridge after the last one.
    String::from(
        r#"<script src="/static/theme-boot.js"></script>
<link rel="stylesheet" href="/static/fonts.css">
<link rel="stylesheet" href="/static/bootstrap.min.css">
<link rel="stylesheet" href="/static/themes.css">
<link rel="stylesheet" href="/static/kp-components.css">
<link rel="stylesheet" href="/static/theme-bridge.css">
<script src="/static/almanac-head.js"></script>"#,
    )
}

/// Which of the eleven themes are dark, read out of the vendored
/// registry rather than kept as a second Rust list.
///
/// kp-themes 3.0.0 groups its picker into light and dark sections by
/// default (TH63); almanac writes that grouping itself since it writes
/// the whole menu server-side, and grouping needs the split *before*
/// the page paints. A hand-typed list here would be exactly the mistake
/// K25 already made once — this reads the same generated file
/// `k25_the_rust_theme_list_matches_the_packages_own_registry` checks
/// almanac's names against, so a palette that changes side upstream
/// changes this too, the next time someone runs the vendor script.
fn dark_themes() -> std::collections::HashSet<&'static str> {
    const REGISTRY: &str = include_str!("../../static/theme-registry.js");
    REGISTRY
        .split("{ name: '")
        .skip(1)
        .filter_map(|entry| {
            let name_end = entry.find('\'')?;
            let close = entry.find('}')?;
            entry[..close]
                .contains("dark: true")
                .then_some(&entry[..name_end])
        })
        .collect()
}

/// The picker itself, rendered from `THEMES` so the list exists once.
///
/// The markup is the shape `@kp-soft/themes/js/theme-picker.js` attaches
/// to, and the classes are the ones `css/components.css` styles, so the
/// look and the behaviour both come from the package rather than from a
/// copy here. Written by the server rather than by the package's own
/// `themeMenuMarkup()`: almanac renders HTML from a Rust binary, and a
/// menu that only exists once a module has run is an empty box on first
/// paint. The light/dark grouping below mirrors that function's own
/// `themeOptionsMarkup()` — same two group headings, same order, same
/// wrapper elements — so the look does not depend on which side wrote
/// the HTML.
fn theme_picker() -> String {
    let dark = dark_themes();
    let option = |(name, label): &(&str, &str)| {
        format!(
            r#"<li><button type="button" data-kp-theme="{name}">
            <span class="kp-swatch" data-theme="{name}"></span>{label}</button></li>"#
        )
    };
    let group = |heading: &str, kind: &str, themes: &[(&str, &str)]| -> String {
        if themes.is_empty() {
            return String::new();
        }
        let options: String = themes.iter().map(option).collect();
        format!(
            r#"<li role="presentation" class="kp-theme-group" data-kp-theme-group="{kind}">
            <span class="kp-theme-group__label" aria-hidden="true">{heading}</span>
            <ul class="kp-theme-group__list" aria-label="{heading}">{options}</ul>
          </li>"#
        )
    };
    let (light_themes, dark_themes): (Vec<_>, Vec<_>) = THEMES
        .iter()
        .copied()
        .partition(|(name, _)| !dark.contains(name));
    let options = group("Light", "light", &light_themes) + &group("Dark", "dark", &dark_themes);

    format!(
        r#"<span class="kp-theme-menu">
        <button type="button" class="kp-icon-button" popovertarget="kp-theme-menu"
                aria-label="Choose a theme" style="anchor-name: --kp-theme-menu">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor"
               stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <circle cx="13.5" cy="6.5" r=".5" fill="currentColor"/>
            <circle cx="17.5" cy="10.5" r=".5" fill="currentColor"/>
            <circle cx="8.5" cy="7.5" r=".5" fill="currentColor"/>
            <circle cx="6.5" cy="12.5" r=".5" fill="currentColor"/>
            <path d="M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10c.9 0 1.6-.7 1.6-1.6 0-.4-.2-.8-.4-1.1-.3-.3-.4-.7-.4-1.1 0-.9.7-1.6 1.6-1.6H16c3.3 0 6-2.7 6-6 0-4.9-4.5-8.6-10-8.6z"/>
          </svg>
        </button>
        <div popover="auto" id="kp-theme-menu" class="kp-popover"
             style="position-anchor: --kp-theme-menu">
          <ul class="kp-menu" data-kp-theme-picker aria-label="Choose a theme">
            {options}
          </ul>
        </div>
        <p data-kp-theme-status hidden></p>
      </span>"#
    )
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
<html lang="en" data-theme="formal" data-bs-theme="light">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} — Almanac</title>
{assets}
<script type="module" src="/static/almanac-picker.js"></script>
<script src="/static/theme-bootstrap.js" defer></script>
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
    {picker}
    <form method="post" action="/logout" class="m-0 ms-2">
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
        assets = head_assets(),
        picker = theme_picker(),
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
<html lang="en" data-theme="formal" data-bs-theme="light">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Log in — Almanac</title>
{assets}
<script type="module" src="/static/almanac-picker.js"></script>
<script src="/static/theme-bootstrap.js" defer></script>
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
</html>"#,
        assets = head_assets()
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
    draft: Option<(&str, &str)>,
) -> Response {
    // Rendered for a person: "3 Sep 2026, 03:47" with "2 hours ago"
    // beside it, in Kenny's own zone. The stored value stays exactly as
    // issued — this is a reading, not a rewrite.
    let zone: chrono_tz::Tz = "Europe/Brussels".parse().unwrap_or(chrono_tz::UTC);
    let now = chrono::Utc::now();
    let issued: HashMap<String, String> = state
        .tokens
        .list()
        .await
        .into_iter()
        .map(|(source_id, raw)| {
            let when = crate::core::humanise::timestamp(&raw, zone);
            let rendered = match crate::core::humanise::how_long_ago(&raw, now) {
                Some(ago) => format!(
                    r#"{}<span class="text-secondary small ms-2">{}</span>"#,
                    escape(&when),
                    escape(&ago)
                ),
                None => escape(&when),
            };
            (source_id, rendered)
        })
        .collect();
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
  <button class="btn btn-sm btn-outline-secondary" data-reveal="{id}">Reveal 10s</button>
  <button class="btn btn-sm btn-outline-secondary" data-copy="{id}">Copy command</button>
  <form method="post" action="/dashboard/sources/{id}/issue" class="d-inline">
    <button class="btn btn-sm btn-outline-warning" type="submit">Re-issue</button>
  </form>
  <form method="post" action="/dashboard/sources/{id}/revoke" class="d-inline">
    <button class="btn btn-sm btn-outline-secondary" type="submit">Revoke token</button>
  </form>
  <form method="post" action="/dashboard/sources/{id}/delete" class="d-inline"
        data-confirm="Delete the source &quot;{id}&quot;? Its token and its profile both go. Events it already put on the calendar stay."
        data-busy="Deleting…">
    <button class="btn btn-sm btn-outline-danger" type="submit">
      <span class="spinner-border spinner-border-sm d-none" aria-hidden="true"></span>
      <span class="label">Delete</span>
    </button>
  </form>
</td></tr>
<tr id="out-{id}" class="d-none"><td colspan="3"><pre class="mb-0 small" id="pre-{id}"></pre></td></tr>"#,
                    when = when
                ),
                None => format!(
                    r#"<tr>
<td><code>{id}</code></td>
<td class="text-secondary">no token</td>
<td class="text-end">
  <form method="post" action="/dashboard/sources/{id}/issue" class="d-inline">
    <button class="btn btn-sm btn-primary" type="submit">Issue token</button>
  </form>
  <form method="post" action="/dashboard/sources/{id}/delete" class="d-inline"
        data-confirm="Delete the source &quot;{id}&quot;? Its token and its profile both go. Events it already put on the calendar stay."
        data-busy="Deleting…">
    <button class="btn btn-sm btn-outline-danger" type="submit">
      <span class="spinner-border spinner-border-sm d-none" aria-hidden="true"></span>
      <span class="label">Delete</span>
    </button>
  </form>
</td></tr>"#
                ),
            }
        })
        .collect();

    let loaded_unusable = crate::shell::profiles::load_all(&state.profiles_dir).unusable;

    // Files that are on disk and not being served. Shown rather than
    // only logged: a source that stopped working is invisible from
    // here otherwise, and the fix — delete it — belongs on the same
    // page as everything else about sources.
    let unusable_card = if loaded_unusable.is_empty() {
        String::new()
    } else {
        let rows: String = loaded_unusable
            .iter()
            .map(|u| {
                let name = escape(&u.file_name());
                format!(
                    r#"<tr>
<td><code>{name}</code></td>
<td class="small text-secondary">{reason}</td>
<td class="text-end">
  <form method="post" action="/dashboard/profiles/{name}/delete" class="d-inline"
        data-confirm="Delete the file {name}? It is not being served, and this removes it from disk."
        data-busy="Deleting…">
    <button class="btn btn-sm btn-outline-danger" type="submit">
      <span class="spinner-border spinner-border-sm d-none" aria-hidden="true"></span>
      <span class="label">Delete</span>
    </button>
  </form>
</td></tr>"#,
                    reason = escape(&u.reason)
                )
            })
            .collect();
        format!(
            r#"<h2 class="h5">Not being served</h2>
<div class="card mb-4 border-danger"><div class="card-body">
  <p class="text-secondary small">
    These files are in the profiles directory and Almanac cannot use them, so the sources they
    describe receive nothing — a post to one answers 401, the same as an unknown source. Almanac
    starts and serves everything else regardless; nothing outside the program decides whether it
    runs.
  </p>
  <div class="table-responsive"><table class="table table-sm align-middle mb-0">
  <thead><tr><th>File</th><th>Why</th><th class="text-end">Actions</th></tr></thead>
  <tbody>{rows}</tbody></table></div>
</div></div>"#
        )
    };

    // The reveal and copy controls fetch the token only when clicked,
    // so a token never sits in the page source waiting to be read over
    // someone's shoulder or scraped out of a cached page.
    let script = r#"<script src="/static/almanac-sources.js" defer></script>"#;

    let alert = error
        .map(|e| {
            format!(
                r#"<div class="alert alert-danger" role="alert"><pre class="mb-0 small">{}</pre></div>"#,
                escape(e)
            )
        })
        .unwrap_or_default();

    let (draft_source, draft_calendar) = draft.unwrap_or(("", ""));
    let draft_source = escape(draft_source);
    let draft_calendar = escape(draft_calendar);
    let draft_new_calendar = String::new();
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
        // Order matters: the create memory is reconciled against
        // Google's own answer first, so an id Google now lists is
        // forgotten there — then the tombstones subtract. The other way
        // round, a calendar hidden as deleted would look unlisted to
        // the create memory and be put straight back.
        Ok(calendars) => (
            state.without_deleted_calendars(state.with_created_calendars(calendars)),
            None,
        ),
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
    let calendar_note = match &calendar_error {
        Some(e) => format!(
            r#"<div class="form-text text-danger">Could not read the calendar list: {}</div>"#,
            escape(e)
        ),
        None if calendars.is_empty() => {
            String::from(r#"<div class="form-text">No calendars yet — make one below first.</div>"#)
        }
        None => String::from(r#"<div class="form-text">Make a new one in Calendars, below.</div>"#),
    };

    // Which sources write to each calendar. Built from the loaded
    // profiles rather than asked of Google: Google knows what a
    // calendar is, only almanac knows who writes to it — and this is
    // what decides whether the delete button is live.
    let loaded_profiles = state.profiles();
    let mut users: std::collections::BTreeMap<&str, Vec<&str>> = std::collections::BTreeMap::new();
    for profile in loaded_profiles.values() {
        users
            .entry(profile.target_calendar_id.as_str())
            .or_default()
            .push(profile.source_id.as_str());
    }
    for sources in users.values_mut() {
        sources.sort_unstable();
    }

    let calendar_rows: String = calendars
        .iter()
        .map(|(id, name)| {
            let sources = users.get(id.as_str()).cloned().unwrap_or_default();
            let listed = if sources.is_empty() {
                r#"<span class="text-secondary">none</span>"#.to_string()
            } else {
                sources
                    .iter()
                    .map(|s| format!("<code>{}</code>", escape(s)))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            // Disabled rather than hidden, and disabled rather than
            // refused on submit: the button being there and dead says
            // "this exists, and not yet", which a missing button does
            // not.
            let action = if sources.is_empty() {
                format!(
                    r#"<form method="post" action="/dashboard/calendars/{id}/delete" class="d-inline"
        data-confirm="Delete the calendar &quot;{name}&quot;? This removes it and every event on it, for everyone it is shared with. It cannot be undone."
        data-busy="Deleting…">
    <button class="btn btn-sm btn-outline-danger" type="submit">
      <span class="spinner-border spinner-border-sm d-none" aria-hidden="true"></span>
      <span class="label">Delete</span>
    </button>
  </form>"#,
                    id = escape(id),
                    name = escape(name)
                )
            } else {
                format!(
                    r#"<button class="btn btn-sm btn-outline-secondary" type="button" disabled
          title="{} source(s) still write here — delete them first">Delete</button>"#,
                    sources.len()
                )
            };
            format!(
                r#"<tr>
<td class="fw-medium">{}</td>
<td>{listed}</td>
<td class="text-end">{action}</td>
</tr>"#,
                escape(name)
            )
        })
        .collect();

    let calendars_card = format!(
        r#"<h2 class="h5">Calendars</h2>
<div class="card mb-4"><div class="card-body">
  <h2 class="h6 card-title">Make a calendar</h2>
  <p class="text-secondary small">
    A calendar Almanac makes is shared with you as its owner straight away — one it made
    without sharing would be visible to nobody, which is why {owner_note}
  </p>
  <form method="post" action="/dashboard/calendars" class="row g-2 align-items-start"
        data-busy="Asking Google…">
    <div class="col-sm-6">
      <label class="form-label" for="calendar_name">Name</label>
      <input type="text" class="form-control" id="calendar_name" name="name"
             value="{draft_new_calendar}" placeholder="Almanac · Huishouden" required {disabled}>
    </div>
    <div class="col-sm-auto d-flex align-items-end" style="min-height: 62px">
      <button class="btn btn-primary" type="submit" {disabled}>
        <span class="spinner-border spinner-border-sm d-none" aria-hidden="true"></span>
        <span class="label">Make calendar</span>
      </button>
    </div>
  </form>

  <div class="table-responsive mt-3"><table class="table table-sm align-middle mb-0">
  <thead><tr><th>Name</th><th>Sources</th><th class="text-end">Actions</th></tr></thead>
  <tbody>{calendar_rows}</tbody></table></div>

  <p class="text-secondary small mt-3 mb-0">
    <b>Delete removes the calendar and every event on it, for everyone it is shared with.</b>
    It is only available for a calendar no source writes to — delete the source first, and the
    button becomes live. Deleting a source never touches its events, so a calendar emptied that
    way still holds them until you remove it here.
  </p>
</div></div>"#,
        owner_note = if can_create {
            "Almanac refuses to make one when ALMANAC_CALENDAR_OWNER is unset."
        } else {
            "<b>ALMANAC_CALENDAR_OWNER is not set, so this is switched off.</b>"
        },
        disabled = if can_create { "" } else { "disabled" }
    );

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
  <form method="post" action="/dashboard/sources" class="row g-2 align-items-start">
    <div class="col-sm-4">
      <label class="form-label" for="source_id">Source name</label>
      <input type="text" class="form-control" id="source_id" name="source_id"
             value="{draft_source}" placeholder="kobo" required>
      <div class="form-text">Letters, digits, dots, hyphens, underscores.</div>
    </div>
    <div class="col-sm-5">
      <label class="form-label" for="calendar">Calendar</label>
      <select class="form-select" id="calendar" name="calendar" required>
        {options}
      </select>
      {calendar_note}
    </div>
    <!-- The button lines up with the CONTROLS, not with the bottom of
         the help text under them: align-items-end stretched this
         column to the tallest one, which is whichever field carries
         the longest hint. A fixed control-row height puts it back on
         the line a person reads it against. -->
    <div class="col-sm-auto d-flex align-items-end" style="min-height: 62px">
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

{unusable_card}

<form method="post" action="/dashboard/sources/reload" class="mb-4">
  <button class="btn btn-sm btn-outline-secondary" type="submit">Reload profiles from disk</button>
  <span class="text-secondary small ms-2">
    for a profile placed on the machine by hand, rather than through the box above
  </span>
</form>

{calendars_card}

<div class="alert alert-secondary" role="alert">
  <h2 class="h6">Where these live, and what survives</h2>
  <p class="mb-0">
    <b>Revoke token</b> takes a source's key away and leaves everything else; issue it a new
    one and it works again. <b>Delete</b> removes the source altogether — token and profile
    both gone, immediately. Neither touches events already on the calendar: those are the
    calendar's now, and removing them is a separate, deliberate act.
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
    /// A calendar id from the dropdown.
    calendar: String,
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
    let draft = Some((source_id, chosen));

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
    if chosen.is_empty() {
        return render_sources(&state, Some("Choose a calendar for this source."), draft).await;
    }
    let calendar_id = chosen.to_string();

    let toml = crate::core::profile::default_profile_toml(source_id, &calendar_id);
    if let Err(e) = crate::shell::profiles::save_new(&state.profiles_dir, &toml) {
        return render_sources(&state, Some(&e.to_string()), draft).await;
    }

    // Reload from disk rather than inserting the parsed profile: the
    // set is what has to stay valid, and reading it back is also the
    // only proof that what was written can be read again.
    state.set_profiles(crate::shell::profiles::load_map(&state.profiles_dir));
    tracing::info!(source_id = %source_id, "added a source from the dashboard");
    Redirect::to("/dashboard/sources").into_response()
}

/// `POST /dashboard/sources/{source_id}/delete` — remove a source
/// entirely (K21): its token and its profile, both gone.
///
/// The events it already put on the calendar are left alone (Kenny,
/// 2026-09-03). Deleting a source says something about the source, not
/// about what already happened; sweeping up months of calendar entries
/// is a second, deliberate act.
async fn delete_source(
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
             Deleting it now would leave them in the journal with no profile to deliver them by. \
             Wait for the queue to drain, or fix whatever is blocking delivery, and delete it then."
        );
        return render_sources(&state, Some(&message), None).await;
    }

    if let Err(e) = state.tokens.revoke(&source_id).await {
        return render_sources(&state, Some(&e.to_string()), None).await;
    }

    let removed = match crate::shell::profiles::delete(&state.profiles_dir, &source_id) {
        Ok(path) => path,
        Err(e) => return render_sources(&state, Some(&e.to_string()), None).await,
    };

    state.set_profiles(crate::shell::profiles::load_map(&state.profiles_dir));
    tracing::info!(
        source_id = %source_id,
        removed = %removed.display(),
        "deleted a source from the dashboard"
    );
    Redirect::to("/dashboard/sources").into_response()
}

/// What the make-a-calendar form sends (K24).
#[derive(Deserialize)]
struct NewCalendar {
    name: String,
}

/// `POST /dashboard/calendars` — make one and share it (K24).
async fn create_calendar(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(form): Form<NewCalendar>,
) -> Response {
    if !is_logged_in(&state, &headers).await {
        return Redirect::to("/login").into_response();
    }

    let name = form.name.trim();
    if name.is_empty() {
        return render_sources(&state, Some("Name the calendar."), None).await;
    }

    let Some(owner) = state.calendar_owner.as_deref() else {
        return render_sources(
            &state,
            Some(
                "ALMANAC_CALENDAR_OWNER is not set — without an owner to share it with, a \
                 calendar Almanac creates belongs to the service account and is visible to \
                 nobody.",
            ),
            None,
        )
        .await;
    };

    // Find-or-create, not create: a double submit, two tabs, or someone
    // retyping a name that already exists must not produce a second
    // calendar. A duplicate is close to invisible — events land,
    // nothing errors, and half of them are on a calendar nobody has
    // open.
    // Serialized per name, and consulted against what almanac made
    // moments ago, because `ensure_calendar` alone is not enough: it
    // looks for an existing calendar in Google's list, and that list
    // lags a create by seconds. Two clicks inside that window both
    // find nothing and both create.
    let lock = state.locks.for_key(&format!("calendar:{name}")).await;
    let _guard = lock.lock().await;

    if let Some(id) = state.remembered_calendar(name) {
        tracing::info!(
            calendar = %name,
            id = %id,
            "a calendar with that name was just created; reusing it rather than making a second"
        );
        return Redirect::to("/dashboard/sources").into_response();
    }

    match state.client.ensure_calendar(name, owner).await {
        Ok((id, created)) => {
            if created {
                state.remember_created_calendar(name, &id);
                // The sharing is named because the line without it
                // cannot tell "created and shared" from "created and
                // invisible to every human" — and that second outcome
                // has happened here twice. `ensure_calendar` shares
                // before it returns, so reaching this point means the
                // grant went through; saying so is what lets a check
                // from outside see it without opening Google Calendar
                // (homelab's observation, 2026-09-03).
                tracing::info!(
                    calendar = %name,
                    id = %id,
                    shared_with = %owner,
                    role = "owner",
                    "created a calendar from the dashboard and shared it"
                );
            }
            Redirect::to("/dashboard/sources").into_response()
        }
        Err(e) => render_sources(&state, Some(&e.to_string()), None).await,
    }
}

/// `POST /dashboard/calendars/{calendar_id}/delete` — remove a calendar
/// and everything on it (K24).
///
/// Guarded twice on purpose. The button is dead in the page when a
/// source still writes here, and the check is repeated on arrival: the
/// page is a snapshot, and a source can be added between the render and
/// the click.
async fn delete_calendar(
    State(state): State<Arc<AppState>>,
    Path(calendar_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !is_logged_in(&state, &headers).await {
        return Redirect::to("/login").into_response();
    }

    let users: Vec<String> = state
        .profiles()
        .values()
        .filter(|p| p.target_calendar_id == calendar_id)
        .map(|p| p.source_id.clone())
        .collect();
    if !users.is_empty() {
        return render_sources(
            &state,
            Some(&format!(
                "{} still writes to that calendar, so it cannot be deleted yet. Delete the \
                 source first — its events stay on the calendar either way.",
                users.join(", ")
            )),
            None,
        )
        .await;
    }

    match state.client.delete_calendar(&calendar_id).await {
        Ok(()) => {
            // Google's list lags a delete by seconds, so the page that
            // renders next would otherwise still show it.
            state.remember_deleted_calendar(&calendar_id);
            state.forget_created_calendar(&calendar_id);
            tracing::info!(calendar_id = %calendar_id, "deleted a calendar from the dashboard");
            Redirect::to("/dashboard/sources").into_response()
        }
        Err(e) => render_sources(&state, Some(&e.to_string()), None).await,
    }
}

/// `POST /dashboard/profiles/{file_name}/delete` — remove a file the
/// service cannot use (K23).
///
/// Addressed by file name rather than source id: a broken profile often
/// has no readable source id, which is frequently the thing wrong with
/// it.
async fn delete_unusable(
    State(state): State<Arc<AppState>>,
    Path(file_name): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !is_logged_in(&state, &headers).await {
        return Redirect::to("/login").into_response();
    }

    // Only a file the loader actually reported as unusable. A loaded
    // profile has its own delete, which revokes the token too, and
    // routing that through here would leave the token behind.
    let unusable = crate::shell::profiles::load_all(&state.profiles_dir).unusable;
    if !unusable.iter().any(|u| u.file_name() == file_name) {
        return (StatusCode::NOT_FOUND, "no such unusable profile").into_response();
    }

    match crate::shell::profiles::delete_file(&state.profiles_dir, &file_name) {
        Ok(removed) => {
            state.set_profiles(crate::shell::profiles::load_map(&state.profiles_dir));
            tracing::info!(
                removed = %removed.display(),
                "deleted an unusable profile from the dashboard"
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

    let profiles = crate::shell::profiles::load_map(&state.profiles_dir);
    tracing::info!(
        count = profiles.len(),
        "reloaded profiles from the dashboard"
    );
    state.set_profiles(profiles);
    Redirect::to("/dashboard/sources").into_response()
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

/// One of the kit's vendored assets (3.0.0): the no-flash script and the
/// display fonts, so the pages need neither inline scripts nor a CDN.
fn kit_asset(name: &str) -> Response {
    match chassis::shell::assets::ASSETS
        .iter()
        .find(|(asset, _, _)| *asset == name)
    {
        Some((_, content_type, bytes)) => (
            [
                (header::CONTENT_TYPE, *content_type),
                (header::CACHE_CONTROL, "public, max-age=86400"),
            ],
            *bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn theme_boot_js() -> Response {
    kit_asset("theme-boot.js")
}

async fn fonts_css() -> Response {
    kit_asset("fonts.css")
}

async fn font_file(axum::extract::Path(file): axum::extract::Path<String>) -> Response {
    kit_asset(&format!("fonts/{file}"))
}

/// Almanac's own page scripts (3.0.0): what used to be inline.
async fn almanac_head_js() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../static/almanac-head.js"),
    )
        .into_response()
}

async fn almanac_picker_js() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../static/almanac-picker.js"),
    )
        .into_response()
}

async fn almanac_sources_js() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../static/almanac-sources.js"),
    )
        .into_response()
}

/// Serves the vendored Bootstrap CSS. Compiled into the binary so a
/// LAN-only service never needs the internet to render its own pages
/// (Kenny's choice, 2026-08-28) and the file cannot go missing from a
/// deployment.
/// The kp-themes token file, vendored (K25). See its own header for
/// which version, and why a copy rather than a dependency.
async fn themes_css() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../../static/themes.css"),
    )
        .into_response()
}

/// The mapping from those tokens onto Bootstrap's own variables (K25).
async fn theme_bridge_css() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../../static/theme-bridge.css"),
    )
        .into_response()
}

/// The component styles that go with those tokens (K25) — the swatch,
/// the popover menu and the icon button the picker markup uses.
async fn kp_components_css() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../../static/kp-components.css"),
    )
        .into_response()
}

/// The picker's behaviour, vendored from the package itself since v1
/// (K25). Four modules rather than one because that is how upstream
/// ships it, and the relative imports between them resolve as long as
/// all four are served from `/static/`.
async fn theme_picker_js() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../static/theme-picker.js"),
    )
        .into_response()
}

async fn theme_core_js() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../static/theme-core.js"),
    )
        .into_response()
}

async fn theme_registry_js() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../static/theme-registry.js"),
    )
        .into_response()
}

/// The dictionary `theme-picker.js` reads its status text from, since
/// kp-themes 2.0.0 (K25). Vendored rather than configured: almanac's UI
/// is English (standing rule 1), which is the package's own default
/// since 3.0.0, so there is nothing to override here — the file only
/// has to exist for the relative import in theme-picker.js to resolve.
async fn theme_strings_js() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../static/strings.js"),
    )
        .into_response()
}

/// Almanac's own glue, and the one piece of theming that is not the
/// package's business: Bootstrap decides light or dark from
/// `data-bs-theme`, which upstream knows nothing about.
async fn theme_bootstrap_js() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../static/theme-bootstrap.js"),
    )
        .into_response()
}

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
        .route(
            "/dashboard/profiles/{file_name}/delete",
            axum::routing::post(delete_unusable),
        )
        .route("/dashboard/calendars", axum::routing::post(create_calendar))
        .route(
            "/dashboard/calendars/{calendar_id}/delete",
            axum::routing::post(delete_calendar),
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
            "/dashboard/sources/{source_id}/delete",
            axum::routing::post(delete_source),
        )
        .route(
            "/dashboard/sources/{source_id}/token",
            axum::routing::get(token_json),
        )
        .route(
            "/static/bootstrap.min.css",
            axum::routing::get(bootstrap_css),
        )
        .route("/static/themes.css", axum::routing::get(themes_css))
        .route(
            "/static/theme-bridge.css",
            axum::routing::get(theme_bridge_css),
        )
        .route(
            "/static/kp-components.css",
            axum::routing::get(kp_components_css),
        )
        .route(
            "/static/theme-picker.js",
            axum::routing::get(theme_picker_js),
        )
        .route("/static/theme-core.js", axum::routing::get(theme_core_js))
        .route(
            "/static/theme-registry.js",
            axum::routing::get(theme_registry_js),
        )
        .route("/static/strings.js", axum::routing::get(theme_strings_js))
        .route(
            "/static/theme-bootstrap.js",
            axum::routing::get(theme_bootstrap_js),
        )
        .route("/static/theme-boot.js", axum::routing::get(theme_boot_js))
        .route("/static/fonts.css", axum::routing::get(fonts_css))
        .route("/static/fonts/{file}", axum::routing::get(font_file))
        .route(
            "/static/almanac-head.js",
            axum::routing::get(almanac_head_js),
        )
        .route(
            "/static/almanac-picker.js",
            axum::routing::get(almanac_picker_js),
        )
        .route(
            "/static/almanac-sources.js",
            axum::routing::get(almanac_sources_js),
        )
}
