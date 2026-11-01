// The Path and Result cards: filled by the last Try, and their idle, refused and ready faces.
import { setCardState, setCode, setTrace, trace, attachCommands, attachPatterns, statePlate } from '/static/kp/dist/kp-themes.min.js';
import { esc } from '../lib/format.js';
import { factRow, setAlert, refusalWords } from './words.js';
import { specRows, diffLines, curlFor } from './spec.js';
import { $, badge, note, specArea, diffArea, curlArea, resultCard, traceEl, traceBody, traceHttp } from './dom.js';

/** Back to the untried state, as after a change of source. */
export function idle() {
    idleTrace();
    emptyResult();
}

/** @param {string | null} tone @param {string} text */
const badgeTone = (tone, text) => {
    badge.hidden = false;
    badge.textContent = text;
    badge.className = `kp-badge${tone ? ` kp-badge--${tone}` : ''}`;
};

/** @param {any} data @param {unknown} payload */
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

/** @param {any} data */
function drawRefused(data) {
    badgeTone('destructive', 'Refused');
    setAlert(note, 'destructive', `Refused (${data.http})`, `${refusalWords(data.answer)} A refused call is not kept: fix it and send it again.`);
    $('result-spec-title').textContent = 'The answer';
    $('result-diff-title').textContent = 'Against the calendar';
    specArea.innerHTML = '<figure class="kp-code" data-kp-code id="result-refusal"></figure>';
    setCode(/** @type {HTMLElement} */ (specArea.firstElementChild), { text: JSON.stringify(data.answer, null, 2), title: `Answer · HTTP ${data.http}`, language: 'JSON', lines: true, wrap: true });
    diffArea.replaceChildren(statePlate(document, { kind: 'empty', title: 'Nothing to compare', text: 'The call was refused, so no event would be written.' }));
}

/** The badge and note for what Almanac would do with the event. @param {any} data @param {ReturnType<typeof diffLines>} rows @param {string} calendarName */
function drawVerdict(data, rows, calendarName) {
    if (data.verdict === 'create') {
        badgeTone('success', 'Would create');
        setAlert(note, 'success', 'New event', `Nothing in ${calendarName} carries this marker yet, so Almanac would create the event. Nothing was written.`);
    } else if (data.verdict === 'update') {
        badgeTone('info', 'Would update');
        setAlert(note, 'info', 'Update', `${calendarName} already holds this event; Almanac would change ${rows.filter((r) => r.kind === 'added').length} of its ${rows.filter((r) => r.kind !== 'removed').length} lines. Nothing was written.`);
    } else {
        badgeTone('warning', 'No change');
        setAlert(note, 'warning', 'No change', `${calendarName} already holds exactly this event; sending it again changes nothing.`);
    }
}

/** @param {ReturnType<typeof diffLines>} rows */
function diffBlock(rows) {
    const changed = rows.filter((r) => r.kind !== 'same').length;
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
    return pre;
}

/** @param {any} data @param {string} calendarName @param {() => void} onRetry */
function drawAccepted(data, calendarName, onRetry) {
    const event = data.answer.event;
    $('result-spec-title').textContent = 'The event';
    specArea.innerHTML = `<div class="kp-formgrid" data-kp-formgrid-lines="lines" role="group" aria-label="The calendar event">${specRows(event, calendarName)
        .map(([label, value]) => factRow(label, value, label === 'Marker'))
        .join('')}</div>`;
    attachPatterns(specArea);
    if (data.lookup_failed) {
        badgeTone('warning', 'Could not compare');
        setAlert(note, 'warning', 'Not compared', `Almanac could not look in ${calendarName} just now, so it cannot say whether this event is new. Nothing was written.`);
        $('result-diff-title').textContent = 'Against the calendar';
        diffArea.replaceChildren(statePlate(document, { kind: 'error', title: 'Could not look in the calendar', text: 'The calendar did not answer.', actionLabel: 'Try again', onAction: onRetry }));
        return;
    }
    const rows = diffLines(data.existing, event);
    drawVerdict(data, rows, calendarName);
    $('result-diff-title').textContent = data.existing ? 'Against the event in the calendar' : 'Against the calendar: nothing there yet';
    diffArea.replaceChildren(diffBlock(rows));
}

/** @param {any} data @param {unknown} payload @param {() => void} onRetry asks again */
export function drawResult(data, payload, onRetry) {
    if (data.http !== 200) drawRefused(data);
    else drawAccepted(data, data.calendar_name ?? '', onRetry);
    drawCurl(data, payload);
}

/** @param {any} data */
export function drawTrace(data) {
    setTrace(traceEl, data.hops);
    traceHttp.hidden = false;
    traceHttp.textContent = `HTTP ${data.http}`;
    traceHttp.className = data.http === 200 ? 'kp-badge kp-badge--success' : 'kp-badge kp-badge--destructive';
    // The event is left out of the accepted answer here: it is the result's subject, above.
    const shown = data.http === 200 ? { ...data.answer, event: 'shown in the result above' } : data.answer;
    setCode(traceBody, { text: JSON.stringify(shown, null, 2), title: `Answer · HTTP ${data.http}`, language: 'JSON', lines: true, wrap: true });
}

export function idleTrace() {
    traceHttp.hidden = true;
    trace(traceEl)?.setState('empty');
    setCode(traceBody, { text: '', title: 'Answer', language: 'JSON', state: 'empty', message: 'No answer yet' });
}

export function emptyResult() {
    badge.hidden = true;
    setCardState(resultCard, 'empty', { title: 'Nothing tried yet', text: 'Write an event call in the editor and press Try it; the result shows here.', action: null });
}
