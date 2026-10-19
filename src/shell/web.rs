//! Almanac's own browser files (kp-themes 11.0.0 adoption, 4.3.0): the page
//! scripts and styles the server-rendered pages import, served by the kit's
//! `webapp` feature under `/ui`, behind the admin login, same origin, so the
//! kit's CSP (`script-src 'self'`) holds without an inline script.

/// Every file under `web/`, embedded. A file that is not listed here is a
/// 404 in the binary, so a new page script is added in both places.
pub const FILES: &[(&str, &[u8])] = &[
    ("index.html", include_bytes!("../../web/index.html")),
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
    ("js/history.js", include_bytes!("../../web/js/history.js")),
    (
        "css/history.css",
        include_bytes!("../../web/css/history.css"),
    ),
];

/// Mounts [`FILES`] at `/ui`.
pub fn mount(app: &mut chassis::App) {
    app.webapp(chassis::shell::webapp::WebApp::embedded(FILES).at("/ui"));
}
