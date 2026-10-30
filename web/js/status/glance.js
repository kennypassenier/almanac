// The Journal section's glance: the counts, the health lines, the trend and the outcome bar, drawn from the last answer.
import { setStackbar } from '/static/kp/js/stackbar.js';
import { setTrendData } from '/static/kp/js/chart.js';
import { $ } from './card.js';
import { journal } from './state.js';

/** Sets an element's text only when it changed, so nothing flickers on a poll. @param {string} id @param {string} text */
function setText(id, text) {
    const el = $(`#${id}`);
    if (el && el.textContent !== text) el.textContent = text;
}

/** @param {string} selector @param {string | null} value */
function setTone(selector, value) {
    if (value) $(selector)?.setAttribute('data-kp-tone', value);
    else $(selector)?.removeAttribute('data-kp-tone');
}

/** @param {number | null} waiting */
function waitingNote(waiting) {
    if (waiting === null) return 'The journal cannot be read';
    if (waiting === 0) return 'Nothing is waiting';
    return waiting === 1 ? 'On its way to the calendar' : 'On their way to the calendars';
}

/** @param {any} d */
function drawCounts(d) {
    const waiting = d.waiting;
    setText('st-waiting', waiting === null ? '–' : String(waiting));
    setText('st-delivered', String(d.delivered));
    setText('st-failed', String(d.gaveUp));
    setText('st-per-hour', String(d.trend.lastHour));
    setTone('#st-kpi-waiting', waiting ? 'warning' : null);
    setTone('#st-kpi-failed', d.gaveUp > 0 ? 'destructive' : null);
    setText('st-waiting-note', waitingNote(waiting));
    setText('st-delivered-note', `${d.accepted} accepted in all`);
    setText('st-failed-note', d.gaveUp === 0 ? 'None gave up' : 'Gave up; see Recent deliveries');
}

/** @param {any} d */
function drawHealth(d) {
    for (const [id, health] of [['st-health-journal', d.journal], ['st-health-store', d.store]]) {
        const el = $(`#${id}`);
        if (!el) continue;
        el.setAttribute('data-state', health.state);
        const text = $('[data-st-health-text]', el);
        if (text && text.textContent !== health.text) text.textContent = health.text;
    }
}

/** @param {any} trend */
function drawTrend(trend) {
    setTrendData(/** @type {HTMLElement} */ ($('#st-trend')), {
        points: trend.values.map((/** @type {number} */ v, /** @type {number} */ i) => [trend.start + i * trend.step, v]),
        step: trend.step,
        unit: 'events',
        digits: 0,
    });
}

/** @param {any} o the outcome counts */
function drawOutcomes(o) {
    const only = new Set([...document.querySelectorAll('[data-st-outcome][aria-pressed="true"]')].map((b) => b.getAttribute('data-st-outcome')));
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

export function drawGlance() {
    const d = journal.last;
    if (!d) return;
    drawCounts(d);
    drawHealth(d);
    drawTrend(d.trend);
    drawOutcomes(d.outcomes);
}
