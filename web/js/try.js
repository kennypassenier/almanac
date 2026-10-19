// Try a source: the page's own wiring. Every component is the package's (kp-themes 11.0.0); this file hands them Almanac's
// real data from the two JSON routes of src/shell/ui/try_source.rs and wires the states (loading while asking, empty,
// failed with a working Retry).

import { setCardState } from '/static/kp/js/card-state.js';
import { setCode } from '/static/kp/js/code.js';
import { setTrace, trace } from '/static/kp/js/path-trace.js';
import { jsonField } from '/static/kp/js/json-field.js';
import { attachCommands } from '/static/kp/js/command.js';
import { attachPatterns } from '/static/kp/js/patterns.js';
import { setSegmentedItems, setSegmentedValues } from '/static/kp/js/segmented.js';
import { statePlate } from '/static/kp/js/state-plate.js';

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

const $ = (/** @type {string} */ id) => /** @type {HTMLElement} */ (document.getElementById(id));
/** @param {unknown} text */
const esc = (text) => String(text).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c] ?? c);
let serial = 0;

/** @param {string} url @param {RequestInit} [init] */
async function api(url, init) {
    const response = await fetch(url, { credentials: 'same-origin', ...init, headers: { 'Content-Type': 'application/json', ...init?.headers } });
    if (!response.ok) throw new Error(String(response.status));
    return response.json();
}

/** @param {string} value */
const copyable = (value) => {
    const id = `try-copy-${(serial += 1)}`;
    return `<span class="kp-copyable"><span class="kp-copyable__value" id="${id}">${esc(value)}</span><button type="button" class="kp-button kp-button--ghost kp-copyable__button" data-kp-copy="${id}">Copy</button></span>`;
};
/** @param {HTMLElement} alert @param {'info' | 'success' | 'warning' | 'destructive'} tone @param {string} label @param {string} text */
function setAlert(alert, tone, label, text) {
    alert.className = `kp-alert kp-alert--${tone}`;
    alert.setAttribute('role', tone === 'destructive' ? 'alert' : 'status');
    alert.innerHTML = `<span><span class="kp-alert__label">${esc(label)}: </span>${esc(text)}</span>`;
}
/** A refusal in two plain parts: what is wrong, and what to do (the API repeats the remedy inside `message`). */
function refusalWords(body) {
    const what = body.message.endsWith(` — ${body.remedy}`) ? body.message.slice(0, -(body.remedy.length + 3)) : body.message;
    return `${what.replace(/, expected one of .*$/, '')}. ${body.remedy[0].toUpperCase()}${body.remedy.slice(1)}`;
}

// ---- Sources: the pickers and the profile card -------------------------------------------------------------------------------
/** @type {any[]} */
let sources = [];
const sourceById = (/** @type {string} */ id) => sources.find((s) => s.source_id === id) ?? null;
let profileSource = '';
let editorSource = '';

const profileCard = $('profile-card');
const tryButton = /** @type {HTMLButtonElement} */ ($('editor-try'));
const area = /** @type {HTMLTextAreaElement} */ ($('editor-body'));

function drawProfile() {
    const s = sourceById(profileSource);
    if (!s) return;
    $('profile-title').textContent = `Profile of ${s.source_id}`;
    const row = (/** @type {string} */ label, /** @type {string} */ value) => `<div class="kp-kv__row"><dt>${label}</dt><dd>${value}</dd></div>`;
    const kv = $('profile-kv');
    kv.innerHTML = [
        row('Source id', copyable(s.source_id)),
        row('Calendar', esc(s.calendar_name)),
        row('Calendar id', copyable(s.calendar_id)),
        row('Time zone', esc(s.timezone)),
        row('Default length', `${s.default_duration_minutes} minutes${s.default_is_fallback ? ' (Almanac&rsquo;s own default)' : ''}`),
        row('Schema version', String(s.schema_version)),
        row('File', s.file_name ? copyable(`profiles/${s.file_name}`) : 'Not found'),
    ].join('');
    attachPatterns(kv);
    const file = $('profile-toml');
    if (s.toml == null) setCode(file, { text: '', title: 'Profile file', language: 'TOML', state: 'error', message: 'The profile file could not be read' });
    else setCode(file, { text: s.toml, title: 'Profile file', language: 'TOML', lines: true, wrap: true, size: `${new Blob([s.toml]).size} B` });
}

function drawPickers() {
    const items = sources.map((s) => ({ value: s.source_id, label: s.source_id }));
    for (const [id, value] of [['profile-picker', profileSource], ['editor-picker', editorSource]]) {
        setSegmentedItems($(id), items);
        setSegmentedValues($(id), value ? [value] : []);
    }
}

async function loadSources() {
    setCardState(profileCard, 'loading');
    try {
        sources = (await api('/try/api/sources')).sources;
    } catch {
        setCardState(profileCard, 'error', { title: 'Could not read the profiles', text: 'Almanac did not answer. Nothing else is affected.' });
        return;
    }
    if (!sources.length) {
        const reload = document.createElement('button');
        reload.type = 'button';
        reload.className = 'kp-button kp-button--primary';
        reload.textContent = 'Reload profiles from disk';
        reload.addEventListener('click', async () => {
            setCardState(profileCard, 'loading');
            try {
                await fetch('/sources/reload', { method: 'POST', credentials: 'same-origin' });
            } catch {
                // The next read says what is wrong.
            }
            loadSources();
        });
        setCardState(profileCard, 'empty', { title: 'No sources loaded', text: 'Almanac serves a source only when its profile file is in the profiles folder.', action: reload });
        refreshTry();
        return;
    }
    if (!sourceById(profileSource)) profileSource = sources[0].source_id;
    if (!sourceById(editorSource)) editorSource = sources[0].source_id;
    drawPickers();
    drawProfile();
    setCardState(profileCard, 'ready');
    refreshTry();
}

$('profile-picker').addEventListener('kp-segmented-change', (event) => {
    profileSource = /** @type {CustomEvent} */ (event).detail.value;
    drawProfile();
});
$('editor-picker').addEventListener('kp-segmented-change', (event) => {
    editorSource = /** @type {CustomEvent} */ (event).detail.value;
    idle();
});
profileCard.addEventListener('kp-card-retry', loadSources);

// ---- Event editor ------------------------------------------------------------------------------------------------------------
const editorCard = $('editor-card');
const answer = $('editor-answer');
const idle = () => setAlert(answer, 'info', 'Note', 'Nothing tried yet. Press Try it.');
let fieldReason = '';
function refreshTry() {
    const reason = !sources.length ? 'No source is loaded' : fieldReason;
    tryButton.disabled = reason !== '';
    if (reason) tryButton.setAttribute('data-kp-disabled-reason', reason);
    else tryButton.removeAttribute('data-kp-disabled-reason');
}
area.addEventListener('kp-json-state', (event) => {
    const { state, message } = /** @type {CustomEvent} */ (event).detail;
    fieldReason = state === 'valid' ? '' : state === 'empty' ? 'Paste or write an event call first' : `Fix the call first: ${message ?? 'it is not valid'}`;
    refreshTry();
});
// A starting call that is valid, on the next full hour, so Try works at once.
const nextHour = new Date(Math.ceil(Date.now() / 3600000) * 3600000).toISOString().slice(0, 19).replace('Z', '');
area.value = JSON.stringify({ external_id: 'try-1', title: 'Example event', start: `${nextHour}+00:00` }, null, 2);
area.dispatchEvent(new Event('input', { bubbles: true }));
refreshTry();

// ---- Path, result: filled by the last Try -------------------------------------------------------------------------------------
const traceEl = $('try-trace');
const traceAnswer = $('trace-answer');
const traceBody = $('trace-body');
const traceHttp = $('trace-http');
const traceRefused = $('trace-refused');
const resultCard = $('result-card');
const badge = $('result-badge');
const note = $('result-note');
const specArea = $('result-spec-area');
const diffArea = $('result-diff-area');
const curlArea = $('result-curl-area');
const PALETTE = { 1: 'lavender', 2: 'sage', 3: 'grape', 4: 'flamingo', 5: 'banana', 6: 'tangerine', 7: 'peacock', 8: 'graphite', 9: 'blueberry', 10: 'basil', 11: 'tomato' };

const addDays = (/** @type {string} */ date, /** @type {number} */ days) => {
    const [y, m, d] = date.split('-').map(Number);
    return new Date(Date.UTC(y, m - 1, d + days)).toISOString().slice(0, 10);
};
/** The calendar spec of an event as label/value rows, in the words a person reads (dd/mm/yyyy HH:MM, 24 h). */
function specRows(event, calendarName) {
    const rows = [];
    const stamp = (point) => {
        if (point.date) return point.date.split('-').reverse().join('/');
        const parts = Object.fromEntries(
            new Intl.DateTimeFormat('en-GB', { timeZone: point.timeZone, year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' })
                .formatToParts(new Date(point.dateTime)).map((p) => [p.type, p.value]),
        );
        return `${parts.day}/${parts.month}/${parts.year} ${parts.hour}:${parts.minute}`;
    };
    rows.push(['Title', event.summary]);
    if (event.start.date) {
        const last = addDays(event.end.date, -1);
        const days = (Date.parse(event.end.date) - Date.parse(event.start.date)) / 86400000;
        rows.push(['When', `${stamp(event.start)}${days > 1 ? ` to ${stamp({ date: last })}` : ''} (all day, ${days} ${days === 1 ? 'day' : 'days'})`]);
    } else {
        rows.push(['When', `${stamp(event.start)} to ${stamp(event.end).slice(11)} (${event.start.timeZone})`]);
    }
    rows.push(['Calendar', calendarName]);
    if (event.location) rows.push(['Location', event.location]);
    if (event.description) rows.push(['Description', event.description]);
    rows.push(['Shows as', event.transparency === 'transparent' ? 'Free (does not block your time)' : 'Busy']);
    if (event.colorId) rows.push(['Colour', `${PALETTE[event.colorId] ?? event.colorId} (id ${event.colorId})`]);
    if (event.status) rows.push(['Status', event.status]);
    if (event.reminders) {
        const o = event.reminders.overrides ?? [];
        const mins = (m) => (m % 1440 === 0 ? `${m / 1440} ${m === 1440 ? 'day' : 'days'}` : m % 60 === 0 ? `${m / 60} h` : `${m} min`);
        rows.push(['Reminders', o.length ? o.map((r) => `${r.method} ${mins(r.minutes)} before`).join(', ') : 'None (silent)']);
    } else rows.push(['Reminders', "The calendar's defaults"]);
    if (event.extendedProperties) rows.push(['Marker', event.extendedProperties.private.almanac_source_id]);
    return rows;
}

/** A line diff of two JSON documents (LCS), as kp-diff rows: `{ kind, oldNo, newNo, text }`. */
function diffLines(before, after) {
    const a = before == null ? [] : JSON.stringify(before, null, 2).split('\n');
    const b = JSON.stringify(after, null, 2).split('\n');
    const n = a.length;
    const m = b.length;
    const table = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
    for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) table[i][j] = a[i] === b[j] ? table[i + 1][j + 1] + 1 : Math.max(table[i + 1][j], table[i][j + 1]);
    const rows = [];
    let i = 0;
    let j = 0;
    while (i < n && j < m) {
        if (a[i] === b[j]) rows.push({ kind: 'same', oldNo: ++i, newNo: ++j, text: b[j - 1] });
        else if (table[i + 1][j] >= table[i][j + 1]) rows.push({ kind: 'removed', oldNo: ++i, newNo: null, text: a[i - 1] });
        else rows.push({ kind: 'added', oldNo: null, newNo: ++j, text: b[j - 1] });
    }
    while (i < n) rows.push({ kind: 'removed', oldNo: ++i, newNo: null, text: a[i - 1] });
    while (j < m) rows.push({ kind: 'added', oldNo: null, newNo: ++j, text: b[j - 1] });
    return rows;
}

function curlFor(url, payload) {
    const body = JSON.stringify(payload).replaceAll("'", "'\\''");
    return [`curl -sS -X POST "${url}" \\`, '  -H "Authorization: Bearer $ALMANAC_TOKEN" \\', '  -H "Content-Type: application/json" \\', `  -d '${body}'`].join('\n');
}

const badgeTone = (/** @type {string | null} */ tone, /** @type {string} */ text) => {
    badge.hidden = false;
    badge.textContent = text;
    badge.className = `kp-badge${tone ? ` kp-badge--${tone}` : ''}`;
};

function drawCurl(data, payload) {
    const command = document.createElement('div');
    command.className = 'kp-command';
    command.setAttribute('data-kp-command', 'bash');
    command.setAttribute('data-kp-command-where', 'a shell with ALMANAC_TOKEN set');
    command.innerHTML = '<pre class="kp-command__code"><code></code></pre>';
    /** @type {HTMLElement} */ (command.querySelector('code')).textContent = curlFor(data.urls.dry_run, payload);
    curlArea.replaceChildren(command);
    attachCommands(curlArea);
}

function drawResult(data, payload) {
    const diffBox = (/** @type {Node} */ child) => diffArea.replaceChildren(child);
    const calendarName = data.calendar_name ?? '';
    if (data.http !== 200) {
        badgeTone('destructive', 'Refused');
        setAlert(note, 'destructive', `Refused (${data.http})`, refusalWords(data.answer));
        $('result-spec-title').textContent = 'The answer';
        $('result-diff-title').textContent = 'Against the calendar';
        specArea.innerHTML = '<figure class="kp-code" data-kp-code id="result-refusal"></figure>';
        setCode(/** @type {HTMLElement} */ (specArea.firstElementChild), { text: JSON.stringify(data.answer, null, 2), title: `Answer · HTTP ${data.http}`, language: 'JSON', lines: true, wrap: true });
        diffBox(statePlate(document, { kind: 'empty', title: 'Nothing to compare', text: 'The call was refused, so no event would be written.' }));
    } else {
        const event = data.answer.event;
        $('result-spec-title').textContent = 'The event';
        specArea.innerHTML = `<dl class="kp-kv kp-kv--lines" aria-label="The calendar event">${specRows(event, calendarName)
            .map(([label, value]) => `<div class="kp-kv__row"><dt>${esc(label)}</dt><dd>${label === 'Marker' ? copyable(value) : esc(value)}</dd></div>`)
            .join('')}</dl>`;
        attachPatterns(specArea);
        if (data.lookup_failed) {
            badgeTone('warning', 'Could not compare');
            setAlert(note, 'warning', 'Not compared', `Almanac could not look in ${calendarName} just now, so it cannot say whether this event is new. Nothing was written.`);
            $('result-diff-title').textContent = 'Against the calendar';
            diffBox(statePlate(document, { kind: 'error', title: 'Could not look in the calendar', text: 'The calendar did not answer.', actionLabel: 'Try again', onAction: ask }));
        } else {
            const rows = diffLines(data.existing, event);
            const changed = rows.filter((r) => r.kind !== 'same').length;
            const verdict = data.verdict;
            if (verdict === 'create') {
                badgeTone('success', 'Would create');
                setAlert(note, 'success', 'New event', `Nothing in ${calendarName} carries this marker yet, so Almanac would create the event. Nothing was written.`);
            } else if (verdict === 'update') {
                badgeTone('info', 'Would update');
                setAlert(note, 'info', 'Update', `${calendarName} already holds this event; Almanac would change ${rows.filter((r) => r.kind === 'added').length} of its ${rows.filter((r) => r.kind !== 'removed').length} lines. Nothing was written.`);
            } else {
                badgeTone('warning', 'No change');
                setAlert(note, 'warning', 'No change', `${calendarName} already holds exactly this event; sending it again changes nothing.`);
            }
            $('result-diff-title').textContent = data.existing ? 'Against the event in the calendar' : 'Against the calendar: nothing there yet';
            const pre = document.createElement('pre');
            pre.className = 'kp-diff try-diff';
            pre.tabIndex = 0;
            pre.setAttribute('aria-label', changed ? 'Changes against the calendar event' : 'The event, identical to the one in the calendar');
            pre.innerHTML = rows
                .map(
                    (r) =>
                        `<span class="kp-diff__line" data-kind="${r.kind}"><span class="kp-diff__number">${r.newNo ?? r.oldNo}</span><span class="kp-diff__sign">${r.kind === 'added' ? '+' : r.kind === 'removed' ? '-' : ' '}</span><span>${esc(r.text)}</span></span>`,
                )
                .join('');
            diffBox(pre);
        }
    }
    drawCurl(data, payload);
}

function drawTrace(data) {
    setTrace(traceEl, data.hops);
    traceHttp.hidden = false;
    traceHttp.textContent = `HTTP ${data.http}`;
    traceHttp.className = data.http === 200 ? 'kp-badge kp-badge--success' : 'kp-badge kp-badge--destructive';
    traceRefused.hidden = data.http === 200;
    if (data.http === 200) setAlert(traceAnswer, 'success', 'Accepted', `Almanac would write this event to ${data.calendar_name}. Nothing was written.`);
    else setAlert(traceAnswer, 'destructive', `Refused (${data.http})`, refusalWords(data.answer));
    // The event is left out of the accepted answer here: it is the result's subject, below.
    const shown = data.http === 200 ? { ...data.answer, event: 'shown in the result below' } : data.answer;
    setCode(traceBody, { text: JSON.stringify(shown, null, 2), title: `Answer · HTTP ${data.http}`, language: 'JSON', lines: true, wrap: true });
}

function idleTrace() {
    traceHttp.hidden = true;
    traceRefused.hidden = true;
    trace(traceEl)?.setState('empty');
    setAlert(traceAnswer, 'info', 'Note', 'Press Try it to see the path of a call.');
    setCode(traceBody, { text: '', title: 'Answer', language: 'JSON', state: 'empty', message: 'No answer yet' });
}

function emptyResult() {
    badge.hidden = true;
    setCardState(resultCard, 'empty', { title: 'Nothing tried yet', text: 'Write an event call in the editor and press Try it; the result shows here.', action: null });
}

// ---- Try ---------------------------------------------------------------------------------------------------------------------
let asking = 0;
async function ask() {
    const value = jsonField(area)?.value();
    if (value === undefined || !sourceById(editorSource)) return;
    const turn = (asking += 1);
    setCardState(editorCard, 'loading');
    setCardState(resultCard, 'loading');
    trace(traceEl)?.setState('running');
    setAlert(traceAnswer, 'info', 'Asking', 'Almanac is checking the call.');
    setCode(traceBody, { text: '', title: 'Answer', language: 'JSON', state: 'loading', loadingWords: 'Waiting for the answer' });
    let data;
    try {
        data = await api(`/try/api/run/${encodeURIComponent(editorSource)}`, { method: 'POST', body: JSON.stringify({ event: value }) });
    } catch {
        if (turn !== asking) return;
        setCardState(editorCard, 'error', { title: 'Almanac did not answer', text: 'The dry-run call did not get an answer. Your event call is still in the field.' });
        trace(traceEl)?.setState('error', { message: 'Almanac did not answer the dry-run call.' });
        setAlert(traceAnswer, 'destructive', 'Failed', 'Almanac did not answer.');
        setCode(traceBody, { text: '', title: 'Answer', language: 'JSON', state: 'error', message: 'There is no answer to show' });
        setCardState(resultCard, 'error', { title: 'Could not ask Almanac', text: 'The dry-run call did not get an answer.' });
        return;
    }
    if (turn !== asking) return;
    if (data.http === 200) setAlert(answer, 'success', 'Accepted (200)', `Would write "${data.answer.event.summary}" to ${data.calendar_name}. Nothing was written.`);
    else setAlert(answer, 'destructive', `Refused (${data.http})`, refusalWords(data.answer));
    drawTrace(data);
    drawResult(data, value);
    setCardState(editorCard, 'ready');
    setCardState(resultCard, 'ready');
}

$('editor-form').addEventListener('submit', (event) => {
    event.preventDefault();
    if (!tryButton.disabled) ask();
});
editorCard.addEventListener('kp-card-retry', ask);
resultCard.addEventListener('kp-card-retry', ask);
traceEl.addEventListener('kp-trace-retry', ask);
traceEl.addEventListener('kp-trace-rule', () => traceAnswer.focus());

idleTrace();
emptyResult();
await loadSources();
