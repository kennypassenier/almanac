// Section 1 of the Calendars page: the counts. Three of them are toggles that filter the rows they count; the fourth
// is the deliveries-per-hour trend.
import { attachKpiToggles } from '/static/kp/js/kpi.js';
import { attachCharts } from '/static/kp/js/chart.js';
import { statePlate } from '/static/kp/js/state-plate.js';
import { esc, moment, plural } from '../lib/format.js';
import { state, hooks, sources, sourcesOf } from './state.js';
import { page, strip, line } from './dom.js';

/** The kit flips `aria-pressed` in its own click handler; we read it after that has run (next task). */
const AFTER_KIT_TOGGLE_MS = 0;

const waiting = (/** @type {string} */ label) =>
    `<div class="kp-kpi" aria-busy="true"><span class="kp-kpi__label">${label}</span><span class="kp-kpi__value"><span class="kp-skeleton cal-skeleton-value"></span></span><span class="kp-kpi__trend"><span class="kp-skeleton cal-skeleton-line"></span></span></div>`;

const toggleTile = (/** @type {string} */ key, /** @type {string} */ label, /** @type {number} */ value, /** @type {string} */ trend, tone = '') =>
    `<button type="button" class="kp-kpi kp-kpi--toggle" aria-pressed="${state.filterKey === key}" data-cal-kpi="${key}"${tone ? ` data-kp-tone="${tone}"` : ''}><span class="kp-kpi__label">${label}</span><span class="kp-kpi__hint" aria-hidden="true"><span data-kp-when="off">Filter</span><span data-kp-when="on">Filtering ×</span></span><span class="kp-kpi__value">${value}</span><span class="kp-kpi__trend">${esc(trend)}</span></button>`;

/** The fourth tile: deliveries per hour, from the route ring (the latest deliveries since Almanac started). */
function trendTile() {
    const t = /** @type {any} */ (state.profiles).trend;
    const note = t.truncated ? `per hour, latest ${t.ring_size} deliveries` : 'per hour';
    const busiest = t.busiest ? `Busiest hour ${plural(t.busiest.count, 'event', 'events')} at ${moment(t.busiest.hour_ms).slice(11, 13)}:00` : 'Nothing delivered yet';
    const data = JSON.stringify({ unit: 'events', digits: 0, series: [{ label: 'Events delivered', start: t.start_ms, step: t.step_ms, values: t.values }] });
    const chart = t.total
        ? `<figure class="kp-kpi__chart" data-kp-chart="spark" data-kp-spark-head="none" data-kp-spark-axis="relative" aria-label="Events delivered per hour, last 24 hours"><script type="application/json" data-kp-chart-data>${data}</script></figure>`
        : '';
    return `<div class="kp-kpi kp-kpi--trend"><span class="kp-kpi__label">Events delivered <span class="kp-kpi__label-note">${note}</span></span><span class="kp-kpi__value">${t.total}<small>last 24 h</small></span><span class="kp-kpi__trend">${busiest}</span>${chart}</div>`;
}

/** What each count shows when it filters. @param {string | null} key */
function filterWords(key) {
    const calendars = state.google?.calendars.length ?? 0;
    const files = state.profiles?.unserved.length ?? 0;
    if (key === 'calendars') return `Showing the ${plural(calendars, 'calendar', 'calendars')} only.`;
    if (key === 'sources') return `Showing the ${plural(sources().length, 'source', 'sources')} only.`;
    if (key === 'unserved') return `Showing the ${plural(files, 'profile file that is not served', 'profile files that are not served')} only.`;
    return 'Showing every row. Press a count to show only the rows it counts.';
}

function applyFilter() {
    if (state.filterKey) page.setAttribute('data-cal-filter', state.filterKey);
    else page.removeAttribute('data-cal-filter');
    line.textContent = filterWords(state.filterKey);
}

function drawWaiting() {
    strip.setAttribute('aria-busy', 'true');
    strip.innerHTML = waiting('Calendars') + waiting('Sources') + waiting('Profile files not served') + waiting('Events delivered');
    line.textContent = '';
}

function drawFailed() {
    strip.replaceChildren(
        statePlate(document, {
            kind: 'error',
            title: 'The counts could not be read',
            text: 'Almanac could not ask Google or read the profiles folder.',
            actionLabel: 'Try again',
            onAction: () => void hooks.load(),
        }),
    );
    line.textContent = 'Nothing to filter.';
}

/** @param {{ calendars: import('./state.js').Calendar[] }} google @param {{ unserved: any[] }} profiles */
function drawCounts(google, profiles) {
    const calendars = google.calendars;
    const unusedCalendars = calendars.filter((c) => !sourcesOf(c.id).length).length;
    const silent = sources().filter((s) => !s.delivered).length;
    const files = profiles.unserved;
    const calendarsTrend = !calendars.length ? 'Make the first one' : unusedCalendars ? `${unusedCalendars} ${unusedCalendars === 1 ? 'is' : 'are'} used by no source` : 'Every calendar is used';
    const sourcesTrend = !sources().length ? 'Add one on the Sources page' : silent ? `${silent} delivered nothing since the start` : 'Every source has delivered';
    strip.innerHTML = [
        toggleTile('calendars', 'Calendars', calendars.length, calendarsTrend),
        toggleTile('sources', 'Sources', sources().length, sourcesTrend),
        toggleTile('unserved', 'Profile files not served', files.length, files.length ? files[0].file : 'Every profile file is served', files.length ? 'warning' : ''),
        trendTile(),
    ].join('');
    attachKpiToggles(strip);
    attachCharts(strip);
    applyFilter();
}

export function renderStrip() {
    strip.removeAttribute('aria-busy');
    const { google, profiles } = state;
    if (!google || !profiles) return drawWaiting();
    if (!google.ok || !profiles.ok) return drawFailed();
    drawCounts(google, profiles);
}

// A count is a toggle that filters the rows it counts: one at a time, and the line under the strip says which rows are shown.
export function initStrip() {
    strip.addEventListener('click', (event) => {
        const tile = event.target instanceof Element ? /** @type {HTMLElement | null} */ (event.target.closest('[data-cal-kpi]')) : null;
        if (!tile) return;
        setTimeout(() => {
            const on = tile.getAttribute('aria-pressed') === 'true';
            for (const other of strip.querySelectorAll('[data-cal-kpi]')) if (other !== tile) other.setAttribute('aria-pressed', 'false');
            state.filterKey = on ? tile.getAttribute('data-cal-kpi') : null;
            applyFilter();
        }, AFTER_KIT_TOGGLE_MS);
    });
}
