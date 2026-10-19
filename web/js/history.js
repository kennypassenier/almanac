// history page: kp-themes 11.0.0 components wired to Almanac JSON routes.
import { setCalendarDays, setCalendarState, setCalendarLegend, setCalendarCaption, calendarSelect } from '/static/kp/js/calendar.js';
import { setWeekEvents, setWeekState, setWeekLegend } from '/static/kp/js/week.js';
import { dataTable } from '/static/kp/js/datatable.js';

/** Resolves once js/auto.js has attached everything the page carries. */
const ready = new Promise((resolve) => {
    const done = () => document.documentElement.hasAttribute('data-kp-auto-ready');
    if (done()) return resolve(undefined);
    new MutationObserver((_, watch) => {
        if (done()) {
            watch.disconnect();
            resolve(undefined);
        }
    }).observe(document.documentElement, { attributes: true, attributeFilter: ['data-kp-auto-ready'] });
});
await ready;

const RESULT = { created: ['success', 'Created'], updated: ['info', 'Updated'], failed: ['destructive', 'Failed'] };
/** @param {string} text */
const esc = (text) => String(text).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c] ?? c);
/** `2026-10-19` as `19/10/2026`. @param {string} iso */
const human = (iso) => iso.split('-').reverse().join('/');
/** @param {string} id */
const byId = (id) => /** @type {HTMLElement} */ (document.getElementById(id));

/** @param {string} url */
async function getJson(url) {
    const response = await fetch(url, { headers: { 'Content-Type': 'application/json' } });
    if (!response.ok) throw new Error(String(response.status));
    return response.json();
}

/** @param {HTMLElement | null} kv @param {[string, string][]} rows labels and ready-made inline markup */
const fillKv = (kv, rows) => {
    if (kv) kv.innerHTML = rows.map(([k, v]) => `<div class="kp-kv__row"><dt>${k}</dt><dd>${v}</dd></div>`).join('');
};

/** @param {string} id @param {boolean} show */
const showRetry = (id, show) => {
    byId(id).hidden = !show;
};

// ---------------------------------------------------------------- section 1: deliveries per day
const calendar = byId('hist-calendar');
const tableEl = byId('hist-table');
const handle = dataTable(tableEl);
/** The first recorded day, once known. @type {string | null} */
let firstDay = null;
let pickedDay = '';
let dayToken = 0;

setCalendarLegend(calendar, [
    { tone: 'ok', label: 'Every event delivered' },
    { tone: 'bad', label: 'At least one failed' },
    { tone: 'before', label: 'Before Almanac started' },
    { tone: 'future', label: 'Still to come' },
]);
setCalendarCaption(calendar, 'Under each day: events delivered of events accepted.');
// Loading from the first frame: the card stands over the month until the days arrive.
setCalendarState(calendar, 'loading', 'Reading the deliveries of this month');
handle?.busy({ text: 'Reading the deliveries of this day', overlay: true });

/** @param {string} iso @param {number} n */
function shift(iso, n) {
    const d = new Date(`${iso}T12:00:00Z`);
    d.setUTCDate(d.getUTCDate() + n);
    return d.toISOString().slice(0, 10);
}

async function loadMonth() {
    showRetry('hist-month-retry-row', false);
    setCalendarState(calendar, 'loading', 'Reading the deliveries of this month');
    try {
        const data = await getJson('/history/api/days');
        firstDay = data.first_day;
        if (!firstDay) {
            setCalendarState(calendar, 'empty', 'Nothing has been delivered yet');
            showDay(data.today);
            return;
        }
        /** @type {Record<string, import('/static/kp/js/calendar.js').CalendarDay>} */
        const days = {};
        for (let iso = shift(firstDay, -62); iso < firstDay; iso = shift(iso, 1)) {
            days[iso] = { tone: 'before', label: `${human(iso)}: before Almanac started` };
        }
        for (const d of data.days) {
            days[d.date] = {
                tone: d.failed ? 'bad' : 'ok',
                count: `${d.delivered}/${d.accepted}`,
                label: `${human(d.date)}: ${d.delivered} of ${d.accepted} events delivered${d.failed ? `, ${d.failed} failed` : ''}`,
            };
        }
        for (let iso = firstDay; iso <= data.today; iso = shift(iso, 1)) {
            days[iso] ??= { tone: 'muted', count: '0/0', label: `${human(iso)}: 0 of 0 events delivered` };
        }
        setCalendarDays(calendar, days);
        setCalendarState(calendar, 'ready');
        calendarSelect(calendar, data.today);
        if (!pickedDay) showDay(data.today);
    } catch {
        setCalendarState(calendar, 'error', 'Could not read the deliveries of this month');
        showRetry('hist-month-retry-row', true);
        if (!pickedDay) handle?.fail('The deliveries could not be read just now.');
    }
}

/** @param {string} iso */
async function showDay(iso) {
    pickedDay = iso;
    const token = ++dayToken;
    byId('hist-day-title').textContent = human(iso);
    handle?.busy({ text: `Reading the deliveries of ${human(iso)}`, overlay: true });
    try {
        const data = await getJson(`/history/api/day?date=${encodeURIComponent(iso)}`);
        if (token !== dayToken) return;
        firstDay = data.first_day ?? firstDay;
        fillKv(byId('hist-day-summary'), [
            ['Accepted', `${data.accepted} <span class="kp-sr-only">events</span>`],
            ['Created / updated / failed', `${data.created} / ${data.updated} / ${data.failed}`],
        ]);
        byId('hist-rows').innerHTML = data.rows
            .map((/** @type {any} */ r) => {
                const [tone, word] = RESULT[/** @type {keyof typeof RESULT} */ (r.result)];
                return `<tr data-kp-row-key="${esc(r.id)}"><td class="kp-timestamp">${esc(r.delivered)}</td><td>${esc(r.title)}<span class="hist-sub">${esc(r.source)}${
                    r.reason ? ` · ${esc(r.reason)}` : ''
                }</span></td><td>${esc(r.calendar)}</td><td><span class="kp-badge kp-badge--${tone}">${word}</span></td></tr>`;
            })
            .join('');
        const before = firstDay && iso < firstDay;
        byId('hist-empty-title').textContent = before ? 'Almanac had not started yet' : 'Nothing was delivered on this day';
        byId('hist-empty-text').textContent = before ? `The first delivery on record is ${human(firstDay ?? '')}.` : 'Almanac accepted no events.';
        handle?.refresh();
        handle?.busy(null);
        handle?.state('ready');
    } catch {
        if (token !== dayToken) return;
        handle?.fail('The deliveries could not be read just now. Try again in a moment.');
    }
}

calendar.addEventListener('kp-calendar-pick', (event) => showDay(/** @type {CustomEvent} */ (event).detail.date));
tableEl.addEventListener('kp-datatable-retry', () => showDay(pickedDay));
byId('hist-month-retry').addEventListener('click', loadMonth);

// ---------------------------------------------------------------- section 2: this week
const weekEl = byId('hist-week');
/** @type {Map<string, any>} */
let weekById = new Map();
let weekToken = 0;
/** The start the agenda shows, or null for this week. @type {string | null} */
let weekStart = null;

async function loadWeek() {
    const token = ++weekToken;
    showRetry('hist-week-retry-row', false);
    setWeekState(weekEl, 'loading', '');
    try {
        const data = await getJson(`/history/api/week${weekStart ? `?start=${weekStart}` : ''}`);
        if (token !== weekToken) return;
        weekById = new Map(data.events.map((/** @type {any} */ e) => [e.id, e]));
        const now = Date.now();
        const upcoming = data.events.filter((/** @type {any} */ e) => Date.parse(e.start) > now).sort((/** @type {any} */ a, /** @type {any} */ b) => Date.parse(a.start) - Date.parse(b.start))[0];
        const events = data.events.map((/** @type {any} */ e) => ({
            id: e.id,
            start: e.start,
            label: e.label,
            series: e.series,
            detail: e.detail,
            ...(e === upcoming ? { state: 'next' } : {}),
        }));
        setWeekEvents(weekEl, events);
        setWeekLegend(
            weekEl,
            data.calendars.map((/** @type {any} */ c) => ({ series: c.series, label: c.name })),
        );
        if (!events.length) setWeekState(weekEl, 'empty', 'Almanac wrote nothing in this week.');
    } catch {
        if (token !== weekToken) return;
        setWeekState(weekEl, 'error', 'Could not read the calendars Almanac writes to.');
        showRetry('hist-week-retry-row', true);
    }
}

weekEl.addEventListener('kp-week-range', (event) => {
    weekStart = /** @type {CustomEvent} */ (event).detail.start;
    loadWeek();
});
byId('hist-week-retry').addEventListener('click', loadWeek);

weekEl.addEventListener('kp-week-open', (event) => {
    const e = weekById.get(/** @type {CustomEvent} */ (event).detail.id);
    if (!e) return;
    const [tone, word] = e.result ? RESULT[/** @type {keyof typeof RESULT} */ (e.result)] : ['success', 'Delivered'];
    byId('hist-event-title').textContent = e.label;
    fillKv(byId('hist-event-kv'), [
        ['Source', `<span class="kp-id">${esc(e.source)}</span>`],
        ['Calendar', esc(e.calendar)],
        ['Event time', esc(e.event_time)],
        ['Delivered', e.delivered ? `<span class="kp-timestamp">${esc(e.delivered)}</span>` : 'Not on record'],
        ['Result', `<span class="kp-badge kp-badge--${tone}">${word}</span>`],
    ]);
});

loadMonth();
loadWeek();
