//! Almanac's glue to chassis (3.0.0): what `/healthz` and `/metrics` say,
//! in the kit's shape, with Almanac's own words and metric names. The kit
//! answers the routes; these types answer the kit.

use std::sync::Arc;

use chassis::{App, ScrapeSource, Subsystem, SubsystemStatus};

use crate::shell::ingest::AppState;

/// `journal`: readable, with the number of undelivered entries. This is
/// deliberately NOT Google's reachability (M1): the health check going red
/// during an outage Almanac rides out via the journal would be a lie about
/// Almanac's own state. The only failing answer is a journal that cannot
/// be read — the one thing that stops deliveries for good.
pub struct JournalSubsystem(pub Arc<AppState>);

impl Subsystem for JournalSubsystem {
    fn name(&self) -> &str {
        "journal"
    }

    fn check(&self) -> SubsystemStatus {
        match self.0.journal.pending() {
            Ok(pending) if pending.is_empty() => SubsystemStatus::ok("readable; nothing pending"),
            Ok(pending) => SubsystemStatus::ok(format!(
                "readable; {} undelivered entr{} waiting for the worker",
                pending.len(),
                if pending.len() == 1 { "y" } else { "ies" }
            )),
            Err(e) => {
                SubsystemStatus::failing(format!("cannot be read: {e}. What now: {}", e.remedy()))
            }
        }
    }
}

/// The `almanac_*` series (M13), appended verbatim to the kit's `/metrics`
/// so every Grafana panel keeps its query. `almanac_build_info` is the
/// kit's since 3.0.0 (same name, same label).
pub struct AlmanacMetrics(pub Arc<AppState>);

impl ScrapeSource for AlmanacMetrics {
    fn scrape(&self) -> String {
        // The journal depth is read per scrape rather than tracked, because
        // a counter of "how many are pending" drifts from the file the
        // moment a replay, a compaction or a restart touches it.
        let pending = match self.0.journal.pending() {
            Ok(entries) => Some(entries.len() as u64),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "could not read the journal for a metrics scrape; reporting it as unreadable rather than as empty"
                );
                None
            }
        };
        self.0.metrics.render(pending, env!("CARGO_PKG_VERSION"))
    }
}

/// The Journal section on the kit's status page (A2-2): what is waiting
/// and what was just delivered. The section is a frame of kp-themes parts
/// in the loading state; `web/js/status.js` fills it from
/// `/status/journal` and keeps it current (4.3.0).
pub struct JournalSection(pub Arc<AppState>);

/// The head of both sections: the stylesheet and the page module (external,
/// so the kit's `script-src 'self'` holds). Loading a module twice is a no-op.
const STATUS_ASSETS: &str = concat!(
    "<link rel=\"stylesheet\" href=\"/ui/css/states.css\"><link rel=\"stylesheet\" href=\"/ui/css/status.css\">",
    "<script type=\"module\" src=\"/static/kp/js/auto.js\"></script>",
    "<script type=\"module\" src=\"/ui/js/status.js\"></script>",
);

const JOURNAL_HTML: &str = r##"
<div class="st-journal" data-st-journal>
    <div class="st-glance-wrap"><section class="kp-card st-card" id="st-glance" data-kp-state="loading" data-kp-loading-words="Reading the journal" aria-labelledby="st-glance-title">
        <div class="kp-card__header"><h3 class="kp-card__title" id="st-glance-title">At a glance</h3></div>
        <p class="kp-card__body">What is waiting and what happened since the service started.</p>
        <div class="kp-card__stage">
            <div class="kp-card__data st-glance" data-st-glance>
                <div class="kp-kpis" role="group" aria-label="Journal figures">
                    <div class="kp-kpi" id="st-kpi-waiting">
                        <span class="kp-kpi__label">Waiting to be delivered</span>
                        <span class="kp-kpi__value"><span id="st-waiting" data-kp-tick>&ndash;</span><small>events</small></span>
                        <span class="kp-kpi__trend" id="st-waiting-note">&nbsp;</span>
                    </div>
                    <button type="button" class="kp-kpi kp-kpi--toggle" id="st-kpi-delivered" data-st-outcome="delivered" aria-pressed="false">
                        <span class="kp-kpi__label">Delivered since start</span>
                        <span class="kp-kpi__hint" aria-hidden="true"><span data-kp-when="off">Show in the bar</span><span data-kp-when="on">Showing only this &times;</span></span>
                        <span class="kp-kpi__value"><span id="st-delivered" data-kp-tick>&ndash;</span><small>events</small></span>
                        <span class="kp-kpi__trend" id="st-delivered-note">&nbsp;</span>
                    </button>
                    <button type="button" class="kp-kpi kp-kpi--toggle" id="st-kpi-failed" data-st-outcome="failed" aria-pressed="false">
                        <span class="kp-kpi__label">Failed since start</span>
                        <span class="kp-kpi__hint" aria-hidden="true"><span data-kp-when="off">Show in the bar</span><span data-kp-when="on">Showing only this &times;</span></span>
                        <span class="kp-kpi__value"><span id="st-failed" data-kp-tick>&ndash;</span><small>events</small></span>
                        <span class="kp-kpi__trend" id="st-failed-note">&nbsp;</span>
                    </button>
                    <div class="kp-kpi kp-kpi--trend">
                        <span class="kp-kpi__label">Deliveries per hour</span>
                        <a class="kp-kpi__link" href="#st-deliveries" title="Open the recent deliveries"><span class="kp-kpi__link-word">Deliveries</span> &#8599;</a>
                        <span class="kp-kpi__value"><span id="st-per-hour">&ndash;</span><small>per hour</small></span>
                        <span class="kp-kpi__trend">Last 24 hours, since the service started</span>
                        <figure class="kp-kpi__chart" id="st-trend" data-kp-chart="spark" data-kp-chart-loading data-kp-spark-head="none" data-kp-spark-axis="relative" aria-label="Deliveries per hour, last 24 hours"></figure>
                    </div>
                </div>
                <div class="st-health" role="group" aria-label="Subsystems">
                    <div class="st-health__item">
                        <span class="st-health__name">Journal</span>
                        <span class="kp-health" data-state="ok" id="st-health-journal"><span class="kp-health__dot" aria-hidden="true"></span> <span data-st-health-text>&nbsp;</span></span>
                    </div>
                    <div class="st-health__item">
                        <span class="st-health__name">Store</span>
                        <span class="kp-health" data-state="ok" id="st-health-store"><span class="kp-health__dot" aria-hidden="true"></span> <span data-st-health-text>&nbsp;</span></span>
                    </div>
                </div>
                <figure class="kp-stackbar" id="st-outcomes"></figure>
            </div>
        </div>
    </section></div>

    <div class="kp-formgrid st-live" id="st-deliveries" data-kp-formgrid-align="center">
        <h3 class="st-live__title">Recent deliveries</h3>
        <div class="kp-refresh" id="st-refresh" data-kp-refresh="0" data-kp-refresh-choices="0,15,30,60" data-kp-look="menu" data-kp-remember="almanac-status-refresh"></div>
        <div class="st-live__card">
        <div class="kp-connection" id="st-connection" data-kp-connection data-kp-connection-source="app" data-kp-placement="top" data-kp-retry-delays="off"></div>
        <section class="kp-card st-card" id="st-deliveries-card" data-kp-state="loading" data-kp-loading-words="Reading the deliveries" aria-label="Recent deliveries">
            <p class="kp-card__body">The newest first, from this run of the service. Filter by result; open a row to read what went wrong.</p>
            <p class="kp-card__body st-key">Result key: <span class="kp-badge kp-badge--success" data-kp-semantic>Created</span> a new calendar entry <span class="kp-badge kp-badge--info" data-kp-semantic>Updated</span> an entry changed <span class="kp-badge kp-badge--warning" data-kp-semantic>Retrying</span> failed, tried again on its own <span class="kp-badge kp-badge--destructive" data-kp-semantic>Failed</span> gave up</p>
            <div class="kp-card__stage">
                <div class="kp-card__data">
                    <div class="kp-datatable st-table" id="st-table" data-kp-datatable data-kp-expandable data-kp-expand-single data-kp-sort-multi="click" data-kp-page-size="8" data-kp-page-sizes="none" data-kp-remember="almanac-status-deliveries">
                        <div class="kp-datatable__bar st-bar">
                            <div class="kp-segmented kp-segmented--chips" data-kp-segmented="multi" role="group" aria-label="Show deliveries by result">
                                <button type="button" class="kp-segmented__item" data-value="all" data-kp-segmented-all aria-pressed="true"><span class="kp-segmented__label">All</span><span class="kp-segmented__count"></span></button>
                                <button type="button" class="kp-segmented__item" data-value="created" aria-pressed="false"><span class="kp-segmented__label">Created</span><span class="kp-segmented__count"></span></button>
                                <button type="button" class="kp-segmented__item" data-value="updated" aria-pressed="false"><span class="kp-segmented__label">Updated</span><span class="kp-segmented__count"></span></button>
                                <button type="button" class="kp-segmented__item" data-value="retrying" aria-pressed="false"><span class="kp-segmented__label">Retrying</span><span class="kp-segmented__count"></span></button>
                                <button type="button" class="kp-segmented__item" data-value="failed" aria-pressed="false"><span class="kp-segmented__label">Failed</span><span class="kp-segmented__count"></span></button>
                            </div>
                            <input class="kp-datatable__search" type="search" data-kp-datatable-search aria-label="Search the deliveries" placeholder="Search source or result" />
                        </div>
                        <div class="kp-table-wrap">
                            <table class="kp-table">
                                <caption class="kp-sr-only">Recent deliveries, newest first; each row opens for details</caption>
                                <thead><tr>
                                    <th scope="col" data-kp-sort="number">When</th>
                                    <th scope="col" data-kp-sort="text">Source</th>
                                    <th scope="col" data-kp-sort="text" data-kp-filter="choice">Result</th>
                                </tr></thead>
                                <tbody></tbody>
                            </table>
                        </div>
                        <div class="kp-empty" data-kp-datatable-empty data-kp-art="nomatch" hidden>
                            <p class="kp-empty__title">Nothing matches</p>
                            <p class="kp-empty__body">No delivery fits this filter.</p>
                            <button type="button" class="kp-button kp-button--secondary" data-kp-datatable-clear>Clear the filter</button>
                        </div>
                        <div class="kp-datatable__bar">
                            <p class="kp-datatable__status" data-kp-datatable-status role="status" aria-live="polite"></p>
                            <div class="kp-datatable__pager" data-kp-datatable-pager></div>
                        </div>
                    </div>
                </div>
            </div>
        </section>
        </div>
    </div>
</div>
"##;

impl chassis::StatusSection for JournalSection {
    fn render(&self) -> chassis::Section {
        // The page fills itself from /status/journal; the server only
        // frames it, so the waiting count is read once, in the explain.
        let explain = match self.0.journal.pending() {
            Ok(_) => String::new(),
            Err(e) => format!(" {e} — {}", e.remedy()),
        };
        chassis::Section {
            title: "Journal".into(),
            explain: format!(
                "Events are journalled on arrival and delivered to Google by the worker; \
                 what is waiting has not reached the calendar yet.{explain}"
            ),
            rows: Vec::new(),
            html: Some(format!("{STATUS_ASSETS}{JOURNAL_HTML}")),
        }
    }
}

/// The Profiles section on the kit's status page (A2-2): which sources
/// are served, and where their events go.
/// The calendar a source writes to, by name (K16 column, 4.0.2).
pub struct TargetCalendar(pub Arc<AppState>);

impl chassis::shell::dashboard::ClientColumn for TargetCalendar {
    fn title(&self) -> String {
        // Not "Calendar": chassis-rs 2.0.0 (feat-clients-2) renders the
        // "calendar" form field's own raw stored value under that header
        // automatically now, so a second "Calendar" column would sit next
        // to it with nothing but the header to tell them apart. This one
        // stays because it is the friendly name (K16); the kit's shows
        // the id, mirroring the Calendars page's own id-behind-a-toggle.
        "Calendar name".into()
    }
    fn cell(&self, client: &chassis::shell::clients_api::ClientView) -> String {
        match self.0.profiles().get(&client.name) {
            Some(p) => crate::core::html::escape(&self.0.calendar_name(&p.target_calendar_id)),
            None => "<span class=\"text-secondary italic\">no profile</span>".into(),
        }
    }
}

/// S1 (4.0.2): a source is a name AND a calendar, made in one go on the
/// kit's Sources page. Refusing here issues no token.
pub fn create_source(
    state: &AppState,
    source_id: &str,
    calendar: &str,
) -> Result<(), chassis::Error> {
    let source_id = source_id.trim();
    let calendar = calendar.trim();
    // A source that already has a profile (a 3.x source getting its token
    // issued again, or one whose profile was placed by hand) keeps it; the
    // calendar chosen on the form does not overwrite a profile on disk.
    if state.profiles().contains_key(source_id) {
        tracing::info!(source_id = %source_id, "token issued for a source that already has a profile");
        return Ok(());
    }
    if !crate::core::profile::source_id_is_safe(source_id) {
        return Err(chassis::Error::invalid(
            format!("\"{source_id}\" cannot be a source name"),
            "use letters, digits, '.', '-' and '_', and do not start with a dot",
        ));
    }
    if calendar.is_empty() {
        return Err(chassis::Error::invalid(
            "no calendar chosen",
            "pick the calendar this source writes to",
        ));
    }
    let toml = crate::core::profile::default_profile_toml(source_id, calendar);
    crate::shell::profiles::save_new(&state.profiles_dir, &toml)
        .map_err(|e| chassis::Error::invalid(e.to_string(), e.remedy().to_string()))?;
    // Reload from disk rather than inserting the parsed profile: reading
    // it back is the only proof that what was written can be read again.
    state.set_profiles(crate::shell::profiles::load_map(&state.profiles_dir));
    tracing::info!(source_id = %source_id, "added a source from the Sources page");
    Ok(())
}

/// The other half of S1, asked before the kit deletes the client: the
/// profile goes with it — unless the journal still holds events for it,
/// which need the profile to be delivered; then the delete is refused and
/// the events it already put on the calendar stay either way (Kenny,
/// 2026-09-03).
pub fn remove_source(state: &AppState, source_id: &str) -> Result<(), chassis::Error> {
    if !state.profiles().contains_key(source_id) {
        return Ok(());
    }
    let waiting = state
        .journal
        .pending()
        .map_err(|e| chassis::Error::internal(e.to_string(), e.remedy().to_string()))?
        .iter()
        .filter(|e| e.source_id == source_id)
        .count();
    if waiting > 0 {
        return Err(chassis::Error::invalid(
            format!("{source_id} still has {waiting} event(s) waiting to be delivered"),
            "wait for the queue to drain, or fix whatever is blocking delivery, and delete it then",
        ));
    }
    let removed = crate::shell::profiles::delete(&state.profiles_dir, source_id)
        .map_err(|e| chassis::Error::internal(e.to_string(), e.remedy().to_string()))?;
    state.set_profiles(crate::shell::profiles::load_map(&state.profiles_dir));
    tracing::info!(source_id = %source_id, removed = %removed.display(), "source profile removed with its client");
    Ok(())
}

pub struct ProfilesSection(pub Arc<AppState>);

const SOURCES_HTML: &str = r##"
<div class="st-sources" data-st-sources>
    <section class="kp-card st-card" id="st-sources-card" data-kp-state="loading" data-kp-loading-words="Reading the profile files" aria-labelledby="st-sources-title">
        <div class="kp-card__header">
            <h3 class="kp-card__title" id="st-sources-title">Profile files</h3>
            <div class="kp-card__actions">
                <button type="button" class="kp-button" id="st-reload" data-kp-flash-button data-kp-post="/status/sources/reload" data-kp-flash-word="Done" data-kp-busy-label="Reloading">Reload profiles from disk</button>
            </div>
        </div>
        <p class="kp-card__body" id="st-sources-intro">Which source writes to which calendar, and whether its profile file is being served.</p>
        <div class="kp-card__stage">
            <div class="kp-card__data st-sources__data" data-st-sources-list></div>
        </div>
    </section>
</div>
"##;

impl chassis::StatusSection for ProfilesSection {
    // The reload is a flash button inside the card (K29 put it under the
    // section through the kit's button; 4.3.0 posts the JSON twin of the
    // same handler so the toast can say the real count), so the kit's own
    // action block stays empty.
    fn render(&self) -> chassis::Section {
        chassis::Section {
            title: "Sources".into(),
            explain: "Every loaded mapping profile and the calendar it writes to; sources are managed on the Sources page, calendars on theirs.".into(),
            rows: Vec::new(),
            html: Some(format!("{STATUS_ASSETS}{SOURCES_HTML}")),
        }
    }
}

/// A2-1 · one-time import of the 3.x per-source tokens into the kit's
/// client store, so JobTracker and every other source keep their tokens.
///
/// Runs before the kit opens the store; does nothing once
/// `clients.json.enc` exists or when there is no 3.x store to read, so
/// it is idempotent across restarts. The tokens are copied unchanged.
pub async fn import_source_tokens(
    state_dir: &std::path::Path,
    token_store: &std::path::Path,
    key_hex: &str,
) -> Result<usize, String> {
    use chassis::core::clients::{Client, ClientsFile};
    use chassis::shell::store::{ClientStore, EncryptedFile, FileClientStore};

    let file = state_dir.join("clients.json.enc");
    if file.exists() || !token_store.exists() {
        return Ok(0);
    }
    let key_bytes: [u8; 32] = hex::decode(key_hex)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| "ALMANAC_SECRET_KEY must be 64 hex characters".to_string())?;
    let store = crate::shell::token_store::TokenStore::with_key_loading(
        token_store.to_path_buf(),
        key_bytes,
    )
    .map_err(|e| format!("{e}: {}", e.remedy()))?;
    let mut clients = Vec::new();
    for (source_id, issued_at) in store.list().await {
        if let Some(token) = store.reveal(&source_id).await.map_err(|e| e.to_string())? {
            // chassis-rs 2.0.0: Client is #[non_exhaustive], so a struct
            // literal from outside the kit no longer compiles — build with
            // Client::adopted instead, which fills every field the kit adds
            // later with its own default.
            clients.push(Client::adopted(
                format!("source-{source_id}"),
                source_id,
                token,
                issued_at,
            ));
        }
    }
    if clients.is_empty() {
        return Ok(0);
    }
    let kit_key = chassis::core::crypto::Key::parse_hex("ALMANAC_SECRET_KEY", key_hex, key_hex)
        .map_err(|e| e.to_string())?;
    let kit_store = FileClientStore::open(EncryptedFile::new(file, kit_key, "clients"))
        .map_err(|e| e.to_string())?;
    let count = clients.len();
    kit_store
        .update(&mut |clients_file: &mut ClientsFile| {
            clients_file.clients.extend(clients.iter().cloned());
            Ok(clients.last().cloned().expect("at least one source"))
        })
        .map_err(|e| e.to_string())?;
    Ok(count)
}

/// Everything Almanac hangs on the kit, in one place, so the binary and the
/// in-process test harness assemble the same service (4.0.0).
pub fn mount(app: &mut App, state: Arc<AppState>) {
    app.subsystem(JournalSubsystem(Arc::clone(&state)));
    app.metrics_source(AlmanacMetrics(Arc::clone(&state)));
    // The machine API behind the kit's door (per-source client tokens, the
    // admin's login token), the pages behind the admin login (A2-2): `/`
    // is the kit's status page with a Journal and a Sources section, the
    // calendars live on /calendars, and the sources — profile, token and
    // each source's last requests (K13) — on the kit's clients page,
    // labelled Sources (S1, 4.0.2).
    app.api_routes(crate::shell::build_router(Arc::clone(&state)));
    app.dashboard_routes(crate::shell::pages(Arc::clone(&state)));
    // S1 (4.0.2): one Sources page — the kit's clients page with a calendar
    // field on the issue form, a Calendar column, and Almanac's profile
    // written and removed through the kit's hooks. `vocabulary` (chassis-rs
    // 1.8.0) replaces the narrower `clients_label`: every kit sentence that
    // used to say "client" — the login page, refusals from the clients API,
    // not only the page heading — now says "source"/"sources".
    app.vocabulary("source", "sources");
    let options = Arc::clone(&state);
    app.client_form_field(chassis::shell::dashboard::ClientFormField::select(
        "calendar",
        "Calendar",
        move || options.calendar_options(),
    ));
    let issued = Arc::clone(&state);
    app.on_client_issued(move |client, fields| {
        create_source(
            &issued,
            &client.name,
            fields.get("calendar").map(String::as_str).unwrap_or(""),
        )
    });
    let deleted = Arc::clone(&state);
    app.on_client_deleted(move |client| remove_source(&deleted, &client.name));
    app.client_column(TargetCalendar(Arc::clone(&state)));
    // Status stays the first entry, as the brand link's own page; project
    // pages sit at order 0 and would otherwise come before it.
    app.kit_page("status", |page| page.order(-1));
    app.nav_entry("Calendars", "/calendars");
    app.nav_entry("Deliveries", "/deliveries");
    app.nav_entry("History", "/history");
    app.nav_entry("Try a source", "/try");
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
