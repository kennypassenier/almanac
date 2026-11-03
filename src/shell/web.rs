//! Almanac's own browser files (kp-themes 11.0.0 adoption, 4.3.0): the page
//! scripts and styles the server-rendered pages import, served by the kit's
//! `webapp` feature under `/ui`, behind the admin login, same origin, so the
//! kit's CSP (`script-src 'self'`) holds without an inline script.

/// Every file under `web/`, embedded. A file that is not listed here is a
/// 404 in the binary, so a new page script is added in both places.
pub const FILES: &[(&str, &[u8])] = &[
    ("index.html", include_bytes!("../../web/index.html")),
    // The approved app icon (2026-11-01), served by the kit as install icons.
    (
        "pwa/icon-180.png",
        include_bytes!("../../web/pwa/icon-180.png"),
    ),
    (
        "pwa/icon-192.png",
        include_bytes!("../../web/pwa/icon-192.png"),
    ),
    (
        "pwa/icon-512.png",
        include_bytes!("../../web/pwa/icon-512.png"),
    ),
    (
        "pwa/icon-maskable-192.png",
        include_bytes!("../../web/pwa/icon-maskable-192.png"),
    ),
    (
        "pwa/icon-maskable-512.png",
        include_bytes!("../../web/pwa/icon-maskable-512.png"),
    ),
    (
        "pwa/icon-monochrome-512.png",
        include_bytes!("../../web/pwa/icon-monochrome-512.png"),
    ),
    ("pwa/icon.svg", include_bytes!("../../web/pwa/icon.svg")),
    ("css/states.css", include_bytes!("../../web/css/states.css")),
    (
        "js/calendars.js",
        include_bytes!("../../web/js/calendars.js"),
    ),
    (
        "css/calendars.css",
        include_bytes!("../../web/css/calendars.css"),
    ),
    ("js/status.js", include_bytes!("../../web/js/status.js")),
    ("css/status.css", include_bytes!("../../web/css/status.css")),
    ("js/try.js", include_bytes!("../../web/js/try.js")),
    ("css/try.css", include_bytes!("../../web/css/try.css")),
    (
        "js/deliveries.js",
        include_bytes!("../../web/js/deliveries.js"),
    ),
    (
        "css/deliveries.css",
        include_bytes!("../../web/css/deliveries.css"),
    ),
    (
        "js/calendars/api.js",
        include_bytes!("../../web/js/calendars/api.js"),
    ),
    (
        "js/calendars/delete-dialog.js",
        include_bytes!("../../web/js/calendars/delete-dialog.js"),
    ),
    (
        "js/calendars/dom.js",
        include_bytes!("../../web/js/calendars/dom.js"),
    ),
    (
        "js/calendars/fixes.js",
        include_bytes!("../../web/js/calendars/fixes.js"),
    ),
    (
        "js/calendars/load.js",
        include_bytes!("../../web/js/calendars/load.js"),
    ),
    (
        "js/calendars/new-dialog.js",
        include_bytes!("../../web/js/calendars/new-dialog.js"),
    ),
    (
        "js/calendars/profiles.js",
        include_bytes!("../../web/js/calendars/profiles.js"),
    ),
    (
        "js/calendars/state.js",
        include_bytes!("../../web/js/calendars/state.js"),
    ),
    (
        "js/calendars/strip.js",
        include_bytes!("../../web/js/calendars/strip.js"),
    ),
    (
        "js/calendars/table.js",
        include_bytes!("../../web/js/calendars/table.js"),
    ),
    (
        "js/calendars/uses.js",
        include_bytes!("../../web/js/calendars/uses.js"),
    ),
    (
        "js/deliveries/actions.js",
        include_bytes!("../../web/js/deliveries/actions.js"),
    ),
    (
        "js/deliveries/aside.js",
        include_bytes!("../../web/js/deliveries/aside.js"),
    ),
    (
        "js/deliveries/chain.js",
        include_bytes!("../../web/js/deliveries/chain.js"),
    ),
    (
        "js/deliveries/dom.js",
        include_bytes!("../../web/js/deliveries/dom.js"),
    ),
    (
        "js/deliveries/groups.js",
        include_bytes!("../../web/js/deliveries/groups.js"),
    ),
    (
        "js/deliveries/load.js",
        include_bytes!("../../web/js/deliveries/load.js"),
    ),
    (
        "js/deliveries/schedule.js",
        include_bytes!("../../web/js/deliveries/schedule.js"),
    ),
    (
        "js/deliveries/state.js",
        include_bytes!("../../web/js/deliveries/state.js"),
    ),
    (
        "js/deliveries/words.js",
        include_bytes!("../../web/js/deliveries/words.js"),
    ),
    (
        "js/lib/format.js",
        include_bytes!("../../web/js/lib/format.js"),
    ),
    (
        "js/lib/ready.js",
        include_bytes!("../../web/js/lib/ready.js"),
    ),
    (
        "js/status/card.js",
        include_bytes!("../../web/js/status/card.js"),
    ),
    (
        "js/status/deliveries.js",
        include_bytes!("../../web/js/status/deliveries.js"),
    ),
    (
        "js/status/footprint.js",
        include_bytes!("../../web/js/status/footprint.js"),
    ),
    (
        "js/status/glance.js",
        include_bytes!("../../web/js/status/glance.js"),
    ),
    (
        "js/status/journal.js",
        include_bytes!("../../web/js/status/journal.js"),
    ),
    (
        "js/status/read.js",
        include_bytes!("../../web/js/status/read.js"),
    ),
    (
        "js/status/sources.js",
        include_bytes!("../../web/js/status/sources.js"),
    ),
    (
        "js/status/state.js",
        include_bytes!("../../web/js/status/state.js"),
    ),
    ("js/try/api.js", include_bytes!("../../web/js/try/api.js")),
    (
        "js/try/copies.js",
        include_bytes!("../../web/js/try/copies.js"),
    ),
    (
        "js/try/copy-menu.js",
        include_bytes!("../../web/js/try/copy-menu.js"),
    ),
    ("js/try/dom.js", include_bytes!("../../web/js/try/dom.js")),
    (
        "js/try/editor.js",
        include_bytes!("../../web/js/try/editor.js"),
    ),
    (
        "js/try/result.js",
        include_bytes!("../../web/js/try/result.js"),
    ),
    (
        "js/try/sources.js",
        include_bytes!("../../web/js/try/sources.js"),
    ),
    ("js/try/spec.js", include_bytes!("../../web/js/try/spec.js")),
    (
        "js/try/state.js",
        include_bytes!("../../web/js/try/state.js"),
    ),
    (
        "js/try/try-button.js",
        include_bytes!("../../web/js/try/try-button.js"),
    ),
    (
        "js/try/words.js",
        include_bytes!("../../web/js/try/words.js"),
    ),
    ("js/history.js", include_bytes!("../../web/js/history.js")),
    (
        "css/history.css",
        include_bytes!("../../web/css/history.css"),
    ),
];

/// Mounts [`FILES`] at `/ui`, and has the kit sync the pages' remembered
/// view state (sorts, toggles, the schedule's list or timeline) across
/// Kenny's devices (chassis-rs 3.12). Only view state: no key here ever
/// holds a secret.
pub fn mount(app: &mut chassis::App) {
    use chassis::shell::webapp::{Pwa, WebApp};
    // Installable on /ui (chassis-rs 4.2.0), opening on Almanac's own pages;
    // the kit layout carries the install tags and the worker on every page.
    app.webapp(
        WebApp::embedded(FILES)
            .kp_modules()
            .at("/ui")
            .pwa(Pwa::new().start_url("/")),
    );
    app.sync_browser_keys(
        chassis::storage::BrowserKeys::new()
            .prefix("kp-remember:")
            .prefix("almanac-"),
    );
}

#[cfg(test)]
mod tests;
