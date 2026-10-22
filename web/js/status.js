// status page: the Journal and Sources sections inside the kit's section cards (chassis-rs 3.9.0). The kit draws each card and
// its states and owns the page's one refresh control; this module fills the two stages from /status/journal and
// /status/sources, reads them again on the kit's refresh, and keeps rows updating in place by their stable id.

import { setCardState } from '/static/kp/js/card-state.js';
import { setStackbar } from '/static/kp/js/stackbar.js';
import { setSegmentedCounts, setSegmentedValues, getSegmentedValues, attachSegmented } from '/static/kp/js/segmented.js';
import { dataTable } from '/static/kp/js/datatable.js';
import { agoMoment, attachAgo } from '/static/kp/js/freshness.js';
import { attachStatusLists, buildStatusList, setStatus } from '/static/kp/js/statusrow.js';
import { setTrendData, attachCharts } from '/static/kp/js/chart.js';
import { attachSparklines, attachKpiToggles, attachKpiStrips } from '/static/kp/js/kpi.js';
import { attachTicks } from '/static/kp/js/tick.js';

const $ = (/** @type {string} */ selector, /** @type {ParentNode} */ root = document) => /** @type {HTMLElement | null} */ (root.querySelector(selector));
const escape = (/** @type {string} */ text) => text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

/** Resolves once chassis.js has attached the page (the kit's own parts attach after that, so handles are awaited). */
const ready = /** @type {Promise<unknown>} */ (/** @type {any} */ (window).chassis?.ready ?? new Promise((resolve) => document.addEventListener('chassis:ready', resolve, { once: true })));

/** The handle of a data table once the kit's script has attached it. @param {HTMLElement} host */
async function tableHandle(host) {
    for (let i = 0; i < 100; i++) {
        const handle = dataTable(host);
        if (handle) return handle;
        await new Promise((resolve) => setTimeout(resolve, 50));
    }
    return null;
}

/** A read that failed in a way a person can be told about. */
class ReadError extends Error {}

/** @param {string} url @param {RequestInit} [init] */
async function readJson(url, init) {
    let response;
    try {
        response = await fetch(url, { credentials: 'same-origin', headers: { Accept: 'application/json' }, ...init });
    } catch {
        throw new ReadError('The Almanac service did not answer. Nothing was lost.');
    }
    const json = /json/.test(response.headers.get('content-type') ?? '') ? await response.json().catch(() => null) : null;
    if (!json) throw new ReadError('Your sign-in has ended. Reload the page to sign in again.');
    if (!response.ok) throw new ReadError(typeof json.error === 'string' ? [json.error, json.remedy].filter(Boolean).join(' ') : 'Almanac could not answer.');
    return json;
}

/* ------------------------------------------------------------------ the Journal section */

const RESULT = { created: ['Created', 'success'], updated: ['Updated', 'info'], retrying: ['Retrying', 'warning'], failed: ['Failed', 'destructive'] };

/** @type {any} the last answer, for a redraw by a toggle */
let last = null;
let firstJournal = true;

/** The kit's section card around a section's stage. @param {string} marker */
const cardOf = (marker) => /** @type {HTMLElement | null} */ ($(`[${marker}]`)?.closest('.kp-card[data-chassis-section]') ?? null);
const journalCard = () => cardOf('data-st-journal');
const deliveriesTable = () => dataTable(/** @type {HTMLElement} */ ($('#st-table')));

/** The table's own loading state: its busy card over the rows, until the read answers. */
function tableLoading() {
    const t = deliveriesTable();
    t?.busy({ text: 'Reading the deliveries', overlay: true });
    t?.state('loading');
}

function drawGlance() {
    const d = last;
    if (!d) return;
    const set = (/** @type {string} */ id, /** @type {string} */ text) => {
        const el = $(`#${id}`);
        if (el && el.textContent !== text) el.textContent = text;
    };
    const waiting = d.waiting;
    set('st-waiting', waiting === null ? '–' : String(waiting));
    set('st-delivered', String(d.delivered));
    set('st-failed', String(d.gaveUp));
    set('st-per-hour', String(d.trend.lastHour));
    const tone = (/** @type {string} */ id, /** @type {string | null} */ value) => {
        if (value) $(id)?.setAttribute('data-kp-tone', value);
        else $(id)?.removeAttribute('data-kp-tone');
    };
    tone('#st-kpi-waiting', waiting ? 'warning' : null);
    tone('#st-kpi-failed', d.gaveUp > 0 ? 'destructive' : null);
    set('st-waiting-note', waiting === null ? 'The journal cannot be read' : waiting === 0 ? 'Nothing is waiting' : waiting === 1 ? 'On its way to the calendar' : 'On their way to the calendars');
    set('st-delivered-note', `${d.accepted} accepted in all`);
    set('st-failed-note', d.gaveUp === 0 ? 'None gave up' : 'Gave up; see Recent deliveries');

    for (const [id, health] of [['st-health-journal', d.journal], ['st-health-store', d.store]]) {
        const el = $(`#${id}`);
        if (!el) continue;
        el.setAttribute('data-state', health.state);
        const text = $('[data-st-health-text]', el);
        if (text && text.textContent !== health.text) text.textContent = health.text;
    }

    const trend = d.trend;
    setTrendData(/** @type {HTMLElement} */ ($('#st-trend')), {
        points: trend.values.map((/** @type {number} */ v, /** @type {number} */ i) => [trend.start + i * trend.step, v]),
        step: trend.step,
        unit: 'events',
        digits: 0,
    });

    const only = new Set([...document.querySelectorAll('[data-st-outcome][aria-pressed="true"]')].map((b) => b.getAttribute('data-st-outcome')));
    const o = d.outcomes;
    const parts = [
        { key: 'created', label: 'Created', value: o.created, group: 'delivered' },
        { key: 'updated', label: 'Updated', value: o.updated, group: 'delivered' },
        { key: 'failed', label: 'Failed', value: o.failed, group: 'failed', tone: /** @type {'destructive'} */ ('destructive') },
    ].filter((p) => only.size === 0 || only.has(p.group));
    const total = parts.reduce((sum, p) => sum + p.value, 0);
    const scope = o.partial ? 'Outcomes of the latest deliveries' : 'Outcomes since start';
    setStackbar(/** @type {HTMLElement} */ ($('#st-outcomes')), {
        title: only.size ? `${scope}: ${[...only].join(' and ')} only` : scope,
        summary: `${total} ${only.size ? 'shown' : 'finished'}`,
        legend: true,
        parts,
        state: total === 0 ? 'empty' : 'ready',
        message: 'Nothing of this kind since the service started.',
    });
}
document.addEventListener('kp-kpi-toggle', () => drawGlance());

/** @param {any} d one delivery */
function rowHtml(d) {
    const [word, tone] = RESULT[d.result];
    const at = Date.parse(d.at);
    const moment = agoMoment(at);
    let detail;
    if (d.result === 'failed' || d.result === 'retrying') {
        const lead = d.result === 'failed' ? 'The delivery did not go through.' : 'The delivery did not go through yet.';
        const follow = d.result === 'failed' ? 'The entry stays in the journal; Almanac has given up on it.' : 'The entry stays in the journal and Almanac tries again on its own.';
        detail = `<div class="st-detail"><p><b>${lead}</b> ${follow}</p><pre class="kp-code-block"><code>${escape(d.error ?? '')}</code></pre></div>`;
    } else {
        const where = d.calendar ? escape(d.calendar) : 'its calendar';
        detail = `<div class="st-detail"><p><b>${word} ${moment}.</b> The entry was written to ${where}. No errors.</p></div>`;
    }
    return { at, moment, word, tone, detail };
}

/** @param {any} d @param {{ at: number, moment: string, word: string, tone: string, detail: string }} v */
function fillRow(d, v) {
    return (
        `<td data-label="When" data-kp-sort-value="${v.at}"><time class="kp-ago" data-kp-ago="${new Date(v.at).toISOString()}" datetime="${new Date(v.at).toISOString()}" data-kp-ago-verb="delivered" title="${v.moment}"></time></td>` +
        `<td data-label="Source"><span class="kp-tag">${escape(d.source)}</span></td>` +
        `<td data-label="Result" data-kp-filter-value="${d.result}"><span class="kp-badge kp-badge--${v.tone}" data-kp-semantic>${v.word}</span></td>`
    );
}

/** Brings the table's rows in line with the answer: a row stays where it is and changes in place, a new one goes in at the top, a gone one leaves. @param {any[]} deliveries */
function syncTable(deliveries) {
    const root = /** @type {HTMLElement} */ ($('#st-table'));
    const body = /** @type {HTMLElement} */ ($('tbody', root));
    /** @type {Map<string, HTMLElement>} */
    const have = new Map();
    for (const tr of body.querySelectorAll(':scope > tr[data-kp-row-key]')) have.set(/** @type {string} */ (tr.getAttribute('data-kp-row-key')), /** @type {HTMLElement} */ (tr));
    const keep = new Set(deliveries.map((d) => d.id));
    for (const [key, tr] of have) {
        if (keep.has(key)) continue;
        const next = tr.nextElementSibling;
        if (next?.hasAttribute('data-kp-row-detail')) next.remove();
        tr.remove();
        have.delete(key);
    }
    /** @type {string[]} */
    const fresh = [];
    for (const d of deliveries) {
        const v = rowHtml(d);
        const sig = `${d.at}|${d.result}|${d.error ?? ''}|${d.calendar ?? ''}`;
        const tr = have.get(d.id);
        if (!tr) {
            fresh.push(
                `<tr data-kp-row-key="${escape(d.id)}" data-st-sig="${escape(sig)}">${fillRow(d, v)}</tr>` +
                    `<tr data-kp-row-detail hidden><td colspan="4">${v.detail}</td></tr>`,
            );
            continue;
        }
        if (tr.getAttribute('data-st-sig') === sig) continue;
        tr.setAttribute('data-st-sig', sig);
        const cells = new DOMParser().parseFromString(`<table><tbody><tr>${fillRow(d, v)}</tr></tbody></table>`, 'text/html').querySelectorAll('td');
        for (const cell of cells) {
            const mine = tr.querySelector(`td[data-label="${cell.getAttribute('data-label')}"]`);
            if (mine) mine.replaceWith(cell);
        }
        const detailCell = tr.nextElementSibling?.matches('[data-kp-row-detail]') ? tr.nextElementSibling.querySelector('td') : null;
        if (detailCell) detailCell.innerHTML = v.detail;
    }
    if (fresh.length) {
        const template = document.createElement('template');
        template.innerHTML = fresh.join('');
        body.prepend(template.content);
    }
    dataTable(root)?.refresh();
    const count = (/** @type {string} */ r) => deliveries.filter((d) => d.result === r).length;
    setSegmentedCounts(/** @type {HTMLElement} */ ($('.kp-segmented', root)), {
        all: deliveries.length,
        created: count('created'),
        updated: count('updated'),
        retrying: count('retrying'),
        failed: count('failed'),
    });
}

/** Joins the chips to the table's Result filter, both ways. */
function joinChips() {
    const root = /** @type {HTMLElement} */ ($('#st-table'));
    const column = () => [...root.querySelectorAll('thead th')].findIndex((th) => th.hasAttribute('data-kp-filter'));
    const chips = /** @type {HTMLElement} */ ($('.kp-segmented', root));
    chips.addEventListener('kp-segmented-change', () => {
        const values = getSegmentedValues(chips);
        dataTable(root)?.filter(column(), values.length ? values : null);
    });
    root.addEventListener('kp-datatable-view', (event) => {
        const filter = /** @type {CustomEvent} */ (event).detail?.filters?.[column()];
        const now = Array.isArray(filter) ? filter : [];
        if (now.join() !== getSegmentedValues(chips).join()) setSegmentedValues(chips, now);
    });
}

/** One read of the journal; throws a ReadError. */
async function loadJournal() {
    const d = await readJson('/status/journal');
    last = d;
    drawGlance();
    syncTable(d.deliveries);
    const t = deliveriesTable();
    t?.busy(null);
    t?.state('ready');
    const card = journalCard();
    if (card) hold(card).set('ready');
    firstJournal = false;
}

/** The first read failed: the kit's card says so, with its normal Try again; the parts that have a state of their own say it too. */
function journalFailed() {
    const note = { title: 'The journal could not be read', text: 'Almanac did not answer. Nothing was lost; try again.' };
    const card = journalCard();
    if (card) hold(card).set('error', note);
    deliveriesTable()?.busy(null);
    deliveriesTable()?.fail('The deliveries could not be read. Almanac did not answer; nothing was lost.');
    const bar = $('#st-outcomes');
    if (bar) setStackbar(bar, { title: 'Outcomes', state: 'error', parts: [], message: note.title });
}

/** @returns {Promise<boolean>} whether the read worked; a later failure leaves what is shown (the kit's connection line says it) */
async function refreshJournal() {
    try {
        await loadJournal();
        return true;
    } catch {
        if (firstJournal) journalFailed();
        return false;
    }
}

/* ------------------------------------------------------------------ the Sources section */

const sourcesCard = () => cardOf('data-st-sources');
let firstSources = true;

/** @param {any} d */
function drawSources(d) {
    const host = /** @type {HTMLElement} */ ($('[data-st-sources-list]'));
    const rows = d.sources.map((/** @type {any} */ s) => ({ id: s.id, label: s.id, detail: s.detail, state: s.state, ...(s.fix ? { fix: s.fix } : {}) }));
    const intro = $('#st-sources-intro');
    if (intro) intro.textContent = `Which source writes to which calendar, and whether its profile file is being served. Profile files, schema version ${d.schemaVersion}.`;
    const card = /** @type {HTMLElement} */ (sourcesCard());
    if (rows.length === 0) {
        host.replaceChildren();
        hold(card).set('empty', { title: 'No profile files on disk', text: 'Add a profile file for a source, then reload the profiles.' });
        return;
    }
    let list = $('.kp-statuslist', host);
    const ids = rows.map((/** @type {any} */ r) => r.id).join('\n');
    if (list && list.getAttribute('data-st-ids') === ids) {
        for (const r of rows) setStatus(list, r.id, { state: r.state, detail: r.detail, fix: r.fix ?? '' });
    } else {
        host.replaceChildren();
        list = buildStatusList(rows, { label: 'Sources and their calendars', single: true, retry: false });
        list.classList.add('kp-statuslist--rows');
        list.setAttribute('data-st-ids', ids);
        host.append(list);
        attachStatusLists(host);
    }
    hold(card).set('ready');
}

/** @returns {Promise<boolean>} */
async function loadSources() {
    try {
        drawSources(await readJson('/status/sources'));
        firstSources = false;
        return true;
    } catch {
        if (firstSources) {
            const card = sourcesCard();
            if (card) hold(card).set('error', { title: 'The profile files could not be read', text: 'Almanac could not list them. Try again in a moment.' });
        }
        return false;
    }
}

/* ------------------------------------------------------------------ the kit's card states */

/**
 * The kit's section card is the whole-section state (loading, empty, failed). `data-chassis-owned` (chassis-rs 3.10) keeps the
 * kit from settling it back to ready after each of its own reads, so the state this module sets is the one that stays.
 * @param {HTMLElement} card
 */
function hold(card) {
    card.setAttribute('data-chassis-owned', '');
    return {
        /** @param {string} state @param {object} [note] */
        set(state, note) {
            if (card.getAttribute('data-kp-state') !== state) setCardState(card, /** @type {any} */ (state), note);
        },
    };
}

/* ------------------------------------------------------------------ go */

const hasJournal = () => document.querySelector('[data-st-journal]') !== null;
const hasSources = () => document.querySelector('[data-st-sources]') !== null;

await ready;

// The parts the kit's own page script does not attach (it takes the stage's datatables, ages and cards).
const stage = /** @type {HTMLElement} */ (document.querySelector('.kp-ops__body') ?? document.body);
attachSparklines(stage);
attachKpiToggles(stage);
attachKpiStrips(stage);
attachCharts(stage);
attachSegmented(stage);
attachTicks(stage);

/** One read of everything, never two at once: the kit's first refresh and this module's own first read are the same read. */
let reading = /** @type {Promise<boolean> | null} */ (null);
function run() {
    if (!reading) {
        reading = Promise.all([hasJournal() ? refreshJournal() : true, hasSources() ? loadSources() : true])
            .then(([journal, sources]) => journal && sources)
            .finally(() => {
                reading = null;
            });
    }
    return reading;
}

if (hasSources()) {
    const card = sourcesCard();
    if (card) hold(card).set('loading', { words: 'Reading the profile files' });
}

if (hasJournal()) {
    // The outcome bar and the trend are never blank before the first answer: the theme's own loading state, at their height.
    const bar = $('#st-outcomes');
    if (bar) setStackbar(bar, { title: 'Outcomes', state: 'loading', parts: [] });
    attachAgo(document);
    joinChips();
    const table = /** @type {HTMLElement} */ ($('#st-table'));
    await tableHandle(table);
    tableLoading();
    $('#st-table')?.addEventListener('kp-datatable-retry', () => {
        tableLoading();
        run();
    });
    const card = journalCard();
    card?.addEventListener('kp-card-retry', () => {
        if (card) hold(card).set('ready');
        tableLoading();
        const bar = $('#st-outcomes');
        if (bar) setStackbar(bar, { title: 'Outcomes', state: 'loading', parts: [] });
        firstJournal = true;
        run();
    });
    // The outcome bar's own Retry (a failed first read) reads again like the card's.
    $('#st-outcomes')?.addEventListener('kp-stackbar-retry', () => {
        const bar = $('#st-outcomes');
        if (bar) setStackbar(bar, { title: 'Outcomes', state: 'loading', parts: [] });
        run();
    });
}

// The page's one refresh control is the kit's; a refresh there reads these sections again. The kit's own read answers
// the control (its freshness line), so this module only listens.
document.querySelector('.kp-ops .kp-refresh')?.addEventListener('kp-refresh', () => {
    run();
});
run();

if (hasSources()) {
    sourcesCard()?.addEventListener('kp-card-retry', () => {
        const card = sourcesCard();
        if (card) hold(card).set('loading', { words: 'Reading the profile files' });
        firstSources = true;
        run();
    });
    // The kit's "Reload profiles from disk" button says what it did in its toast; the list follows it.
    document.addEventListener('kp-flash', (event) => {
        const detail = /** @type {CustomEvent} */ (event).detail;
        const own = /** @type {Element} */ (event.target).closest?.('[data-chassis-flash]');
        if (!own || !sourcesCard()?.contains(own) || detail?.state !== 'success') return;
        loadSources();
    });
}
