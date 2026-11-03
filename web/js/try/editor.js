// The event editor and the Try call: the JSON field, the dry run, and the states it moves the cards through.
import { setCardState, setCode, trace, jsonField, form as formOf, VALID_EVENT } from '/static/kp/dist/js/index.js';
import { api } from './api.js';
import { state, sourceById } from './state.js';
import { area, editorCard, editorForm, resultCard, traceEl, traceBody, note } from './dom.js';
import { drawResult, drawTrace } from './result.js';
import { refreshTry } from './try-button.js';
import { loadSources } from './sources.js';

const HOUR_MS = 60 * 60 * 1000;
/** The length of an ISO timestamp without its fraction: `2026-10-30T14:00:00`. */
const ISO_SECONDS_LENGTH = 19;

/** Increases with every Try; an answer that arrives for an older one is dropped. */
let asking = 0;

/** Drops the answer of a Try in flight, as after a change of source. */
export function cancelAsk() {
    asking += 1;
}

/** A starting call that is valid, on the next full hour, so Try works at once. */
function startingCall() {
    const nextHour = new Date(Math.ceil(Date.now() / HOUR_MS) * HOUR_MS).toISOString().slice(0, ISO_SECONDS_LENGTH).replace('Z', '');
    area.value = JSON.stringify({ external_id: 'try-1', title: 'Example event', start: `${nextHour}+00:00` }, null, 2);
    area.dispatchEvent(new Event('input', { bubbles: true }));
}

function askingNow() {
    setCardState(editorCard, 'loading');
    setCardState(resultCard, 'loading');
    trace(traceEl)?.setState('running');
    setCode(traceBody, { text: '', title: 'Answer', language: 'JSON', state: 'loading', loadingWords: 'Waiting for the answer', copy: false });
}

function askFailed() {
    setCardState(editorCard, 'error', { title: 'Almanac did not answer', text: 'The dry-run call did not get an answer. Your event call is still in the field.' });
    formOf(editorForm)?.errors({ body: 'Almanac did not answer the dry-run call. Your event call is still here; try again.' });
    trace(traceEl)?.setState('error', { message: 'Almanac did not answer the dry-run call.' });
    setCode(traceBody, { text: '', title: 'Answer', language: 'JSON', state: 'error', message: 'There is no answer to show', copy: false });
    setCardState(resultCard, 'error', { title: 'Could not ask Almanac', text: 'The dry-run call did not get an answer.' });
}

async function ask() {
    const value = jsonField(area)?.value();
    if (value === undefined || !sourceById(state.editorSource)) return;
    formOf(editorForm)?.clear();
    const turn = (asking += 1);
    askingNow();
    let data;
    try {
        data = await api(`/try/api/run/${encodeURIComponent(state.editorSource)}`, { method: 'POST', body: JSON.stringify({ event: value }) });
    } catch (error) {
        console.warn('try: the dry run got no answer', error);
        if (turn === asking) askFailed();
        return;
    }
    if (turn !== asking) return;
    try {
        drawTrace(data);
        drawResult(data, value, ask);
    } catch (error) {
        console.warn('try: the answer could not be drawn', error);
        askFailed();
        return;
    }
    setCardState(editorCard, 'ready');
    setCardState(resultCard, 'ready');
}

export function initEditor() {
    area.addEventListener('kp-json-state', (event) => {
        const { state: fieldState, message } = /** @type {CustomEvent} */ (event).detail;
        state.fieldReason = fieldState === 'valid' ? '' : fieldState === 'empty' ? 'Paste or write an event call first' : `Fix the call first: ${message ?? 'it is not valid'}`;
        refreshTry();
    });
    startingCall();
    refreshTry();
    // The package form validates, takes the busy submit and ends it with done(); the page only sends.
    editorForm.addEventListener('submit', (event) => event.preventDefault());
    editorForm.addEventListener(VALID_EVENT, async (event) => {
        const { done } = /** @type {CustomEvent} */ (event).detail;
        await ask();
        done();
        refreshTry();
    });
    // A failed first read retries the profiles; otherwise Retry asks again.
    const retry = () => (state.sourcesFailed ? loadSources() : ask());
    editorCard.addEventListener('kp-card-retry', retry);
    resultCard.addEventListener('kp-card-retry', retry);
    traceEl.addEventListener('kp-trace-retry', ask);
    traceEl.addEventListener('kp-trace-rule', () => note.focus({ preventScroll: true }));
}
