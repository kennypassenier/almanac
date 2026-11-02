//! Almanac's two sections on the kit's status page: Journal and Sources.

use std::sync::Arc;

use crate::core::metrics::history_failure_line;
use crate::shell::state::AppState;

/// The Journal section on the kit's status page (A2-2): what is waiting
/// and what was just delivered. The section is a frame of kp-themes parts
/// in the loading state; `web/js/status.js` fills it from
/// `/status/journal` and keeps it current (4.3.0).
pub struct JournalSection(pub Arc<AppState>);

/// The files both sections need, declared to the kit (kit-kp11): it renders
/// the tags once in the status page's `<head>`. The module attaches the few
/// kp-themes parts the kit's own page script does not (key-figure toggles,
/// the trend, the chips, the outcome bar, the ticking counts).
fn status_assets() -> chassis::SectionAssets {
    chassis::SectionAssets::new()
        .stylesheet("/ui/css/states.css")
        .stylesheet("/ui/css/status.css")
        .module_script("/ui/js/status.js")
}

/// The content of the kit's "Journal" card. The kit draws the card (title,
/// sentence, stage, its four data states); this is what stands in the stage,
/// on the stage's own tracks: no card of its own.
const JOURNAL_HTML: &str = r##"
<div class="st-journal" data-st-journal>
    <div class="st-glance-wrap"><div class="st-glance" data-st-glance>
        <h3 class="st-sub" id="st-glance-title">At a glance</h3>
        <p class="st-lede">What is waiting and what happened since the service started.</p>
        <div class="kp-kpis" role="group" aria-labelledby="st-glance-title">
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
    </div></div>

    <div class="st-live" id="st-deliveries" role="group" aria-labelledby="st-live-title">
        <h3 class="st-sub" id="st-live-title">Recent deliveries</h3>
        <p class="st-lede">The newest first, from this run of the service. Filter by result; open a row to read what went wrong.</p>
        <p class="st-lede st-key">Result key: <span class="kp-badge kp-badge--success" data-kp-semantic>Created</span> a new calendar entry <span class="kp-badge kp-badge--info" data-kp-semantic>Updated</span> an entry changed <span class="kp-badge kp-badge--warning" data-kp-semantic>Retrying</span> failed, tried again on its own <span class="kp-badge kp-badge--destructive" data-kp-semantic>Failed</span> gave up</p>
        <div class="kp-datatable st-table" id="st-table" data-kp-datatable data-kp-busy-overlay data-kp-expandable data-kp-expand-single data-kp-sort-multi="click" data-kp-page-size="8" data-kp-page-sizes="none" data-kp-remember="almanac-status-deliveries">
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
                <p class="kp-empty__title">Nothing to show</p>
                <p class="kp-empty__body">Nothing has been delivered yet this run, or no delivery fits this filter. When Almanac writes an event to a calendar, it appears here.</p>
                <button type="button" class="kp-button kp-button--secondary" data-kp-datatable-clear>Clear the filter</button>
            </div>
            <div class="kp-datatable__bar">
                <p class="kp-datatable__status" data-kp-datatable-status role="status" aria-live="polite"></p>
                <div class="kp-datatable__pager" data-kp-datatable-pager></div>
            </div>
        </div>
    </div>
</div>
"##;

impl chassis::StatusSection for JournalSection {
    fn render(&self) -> chassis::Section {
        // The page fills itself from /status/journal; the server only
        // frames it, so the waiting count is read once, in the explain.
        let mut explain = match self.0.journal.pending() {
            Ok(_) => String::new(),
            Err(e) => format!(" {e} — {}", e.remedy()),
        };
        if let Some(line) = history_failure_line(&self.0.metrics.snapshot()) {
            explain.push(' ');
            explain.push_str(&line);
        }
        chassis::Section {
            title: "Journal".into(),
            explain: format!(
                "Events are journalled on arrival and delivered to Google by the worker; \
                 what is waiting has not reached the calendar yet.{explain}"
            ),
            rows: Vec::new(),
            html: Some(JOURNAL_HTML.into()),
        }
    }

    fn assets(&self) -> chassis::SectionAssets {
        status_assets()
    }
}

pub struct ProfilesSection(pub Arc<AppState>);

const SOURCES_HTML: &str = r##"
<div class="st-sources" data-st-sources>
    <p class="st-lede" id="st-sources-intro">Which source writes to which calendar, and whether its profile file is being served.</p>
    <div class="st-sources__data" data-st-sources-list></div>
</div>
"##;

impl chassis::StatusSection for ProfilesSection {
    // The reload is the kit's flash button in the card's footer (kit-kp11):
    // it posts the JSON twin of the same handler and the toast says what the
    // route answered; `status.js` reads the list again when it succeeds.
    fn render(&self) -> chassis::Section {
        chassis::Section {
            title: "Sources".into(),
            explain: "Every loaded mapping profile and the calendar it writes to; sources are managed on the Sources page, calendars on theirs.".into(),
            rows: Vec::new(),
            html: Some(SOURCES_HTML.into()),
        }
    }

    fn assets(&self) -> chassis::SectionAssets {
        status_assets()
    }

    fn flash_actions(&self) -> Vec<chassis::FlashAction> {
        vec![
            chassis::FlashAction::post("Reload profiles from disk", "/status/sources/reload")
                .word("Done"),
        ]
    }
}
