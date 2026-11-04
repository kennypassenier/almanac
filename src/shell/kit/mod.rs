//! Almanac's glue to chassis (3.0.0): what `/healthz` and `/metrics` say,
//! in the kit's shape, with Almanac's own words and metric names. The kit
//! answers the routes; these types answer the kit.
//!
//! `health` answers `/healthz` and `/metrics`, `sections` adds the status
//! page's Journal and Sources, `sources` ties the kit's clients to
//! Almanac's profiles, and `import` moves 3.x tokens over once. `mount`
//! hangs all of it on the app.

use std::sync::Arc;

use chassis::App;
use chassis::shell::pages::Page;

use crate::shell::state::AppState;

mod health;
mod import;
mod sections;
mod sources;

pub use health::{AlmanacMetrics, JournalSubsystem};
pub use import::import_source_tokens;
pub use sections::{JournalSection, ProfilesSection};
pub use sources::{TargetCalendar, create_source, remove_source};

/// Everything Almanac hangs on the kit, in one place, so the binary and the
/// in-process test harness assemble the same service (4.0.0).
pub fn mount(app: &mut App, state: Arc<AppState>) {
    mount_storage(app, &state);
    // The machine API behind the kit's door (per-source client tokens, the
    // admin's login token), the pages behind the admin login (A2-2): `/`
    // is the kit's status page with a Journal and a Sources section, the
    // calendars live on /calendars, and the sources — profile, token and
    // each source's last requests (K13) — on the kit's clients page,
    // labelled Sources (S1, 4.0.2).
    app.api_routes(crate::shell::build_router(Arc::clone(&state)));
    app.dashboard_routes(crate::shell::pages(Arc::clone(&state)));
    mount_sources_page(app, &state);
    mount_pages(app, state);
}

/// The delivery history's database, its Storage-page entry, and the two
/// subsystems behind `/healthz` and `/metrics`.
fn mount_storage(app: &mut App, state: &Arc<AppState>) {
    // The delivery history lives in the kit's app database (4.4.0): the
    // Storage page shows it as "Delivery log" and prunes its old rows. The
    // handle opens at start; the state keeps it for the worker and the
    // History page. history.json is imported once, in one transaction.
    let app_db = app
        .app_db(crate::shell::history_store::app_db_spec(
            state.journal.legacy_file().to_path_buf(),
        ))
        .expect("the app database is declared once");
    app.storage_kind(crate::shell::history_store::kind());
    state.set_app_db(app_db);
    app.subsystem(JournalSubsystem(Arc::clone(state)));
    app.metrics_source(AlmanacMetrics(Arc::clone(state)));
}

/// The kit's clients page as Almanac's Sources page.
fn mount_sources_page(app: &mut App, state: &Arc<AppState>) {
    // S1 (4.0.2): one Sources page — the kit's clients page with a calendar
    // field on the issue form, a Calendar column, and Almanac's profile
    // written and removed through the kit's hooks. `vocabulary` (chassis-rs
    // 1.8.0) replaces the narrower `clients_label`: every kit sentence that
    // used to say "client" — the login page, refusals from the clients API,
    // not only the page heading — now says "source"/"sources".
    app.vocabulary("source", "sources");
    let options = Arc::clone(state);
    app.client_form_field(chassis::shell::dashboard::ClientFormField::select(
        "calendar",
        "Calendar",
        move || options.calendar_options(),
    ));
    let issued = Arc::clone(state);
    app.on_client_issued(move |client, fields| {
        create_source(
            &issued,
            &client.name,
            fields.get("calendar").map(String::as_str).unwrap_or(""),
        )
    });
    let deleted = Arc::clone(state);
    app.on_client_deleted(move |client| remove_source(&deleted, &client.name));
    app.client_column(TargetCalendar(Arc::clone(state)));
}

/// Almanac's own pages: id, title, path and the line under the title.
const PAGES: [(&str, &str, &str, &str); 4] = [
    (
        "calendars",
        "Calendars",
        "/calendars",
        "The Google calendars events land in",
    ),
    (
        "deliveries",
        "Deliveries",
        "/deliveries",
        "What reached Google Calendar and what waits",
    ),
    ("history", "History", "/history", "Every event, day by day"),
    (
        "try",
        "Try a source",
        "/try",
        "See what a source's event would become",
    ),
];

/// The navigation, the project pages and the status-page sections.
fn mount_pages(app: &mut App, state: Arc<AppState>) {
    // Status stays the first entry, as the brand link's own page; project
    // pages sit at order 0 and would otherwise come before it.
    app.kit_page("status", |page| page.order(-1));
    // Pages with a hint, so the palette and the phone's More drawer print
    // one line under each title, as they do for the kit's own pages.
    for (id, title, path, hint) in PAGES {
        app.page(Page::new(id, title, path).hint(hint));
    }
    crate::shell::web::mount(app);
    app.status_section(JournalSection(Arc::clone(&state)));
    app.status_section(ProfilesSection(state));
    // "Send test" on the Sources page posts a ping with that client's token;
    // the round trip shows under the row's Last requests (K13): "does my
    // token work?" has a button.
    app.test_route(
        "POST",
        "/v1/ping",
        "application/json",
        r#"{"hello":"from the dashboard"}"#,
    );
}

#[cfg(test)]
mod tests;
