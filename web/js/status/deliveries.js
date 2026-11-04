// The Journal section's Recent deliveries table: rows kept in step with the answer, and the chips joined to its Result filter.
import { agoMoment, dataTable, setSegmentedCounts, setSegmentedValues, getSegmentedValues } from '/static/kp/dist/kp-themes.min.js';
import { esc as escape } from '../lib/format.js';
import { $ } from './card.js';

const RESULT = { created: ['Created', 'success'], updated: ['Updated', 'info'], retrying: ['Retrying', 'warning'], failed: ['Failed', 'destructive'] };

export const deliveriesTable = () => dataTable(/** @type {HTMLElement} */ ($('#st-table')));

/** The table's own loading state: its busy card over the rows, until the read answers. */
export function tableLoading() {
    const t = deliveriesTable();
    t?.busy({ text: 'Reading the deliveries', overlay: true });
    t?.state('loading');
}

/** @param {any} d one delivery */
function rowHtml(d) {
    const [word, tone] = RESULT[d.result] ?? [String(d.result), 'info'];
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

/** The row's key to the row, for every delivery row now in the body. @param {HTMLElement} body */
function rowsByKey(body) {
    /** @type {Map<string, HTMLElement>} */
    const have = new Map();
    for (const tr of body.querySelectorAll(':scope > tr[data-kp-row-key]')) have.set(/** @type {string} */ (tr.getAttribute('data-kp-row-key')), /** @type {HTMLElement} */ (tr));
    return have;
}

/** Removes the rows (and their detail rows) whose delivery is gone. @param {Map<string, HTMLElement>} have @param {any[]} deliveries */
function removeGone(have, deliveries) {
    const keep = new Set(deliveries.map((d) => d.id));
    for (const [key, tr] of have) {
        if (keep.has(key)) continue;
        const next = tr.nextElementSibling;
        if (next?.hasAttribute('data-kp-row-detail')) next.remove();
        tr.remove();
        have.delete(key);
    }
}

/** Changes a row in place, cell by cell, after its delivery moved on. @param {HTMLElement} tr @param {any} d @param {ReturnType<typeof rowHtml>} v @param {string} sig */
function updateRow(tr, d, v, sig) {
    tr.setAttribute('data-st-sig', sig);
    const cells = new DOMParser().parseFromString(`<table><tbody><tr>${fillRow(d, v)}</tr></tbody></table>`, 'text/html').querySelectorAll('td');
    for (const cell of cells) {
        const mine = tr.querySelector(`td[data-label="${cell.getAttribute('data-label')}"]`);
        if (mine) mine.replaceWith(cell);
    }
    const detailCell = tr.nextElementSibling?.matches('[data-kp-row-detail]') ? tr.nextElementSibling.querySelector('td') : null;
    if (detailCell) detailCell.innerHTML = v.detail;
}

/** The chips' counts follow the answer. @param {HTMLElement} root @param {any[]} deliveries */
function updateChipCounts(root, deliveries) {
    const count = (/** @type {string} */ r) => deliveries.filter((d) => d.result === r).length;
    setSegmentedCounts(/** @type {HTMLElement} */ ($('.kp-segmented', root)), {
        all: deliveries.length,
        created: count('created'),
        updated: count('updated'),
        retrying: count('retrying'),
        failed: count('failed'),
    });
}

/** Brings the table's rows in line with the answer: a row stays where it is and changes in place, a new one goes in at the top, a gone one leaves. @param {any[]} deliveries */
export function syncTable(deliveries) {
    const root = /** @type {HTMLElement} */ ($('#st-table'));
    const body = /** @type {HTMLElement} */ ($('tbody', root));
    const have = rowsByKey(body);
    removeGone(have, deliveries);
    /** @type {string[]} */
    const fresh = [];
    for (const d of deliveries) {
        const v = rowHtml(d);
        const sig = `${d.at}|${d.result}|${d.error ?? ''}|${d.calendar ?? ''}`;
        const tr = have.get(d.id);
        if (!tr) {
            fresh.push(`<tr data-kp-row-key="${escape(d.id)}" data-st-sig="${escape(sig)}">${fillRow(d, v)}</tr><tr data-kp-row-detail hidden><td colspan="4">${v.detail}</td></tr>`);
        } else if (tr.getAttribute('data-st-sig') !== sig) updateRow(tr, d, v, sig);
    }
    if (fresh.length) {
        const template = document.createElement('template');
        template.innerHTML = fresh.join('');
        body.prepend(template.content);
    }
    dataTable(root)?.refresh();
    updateChipCounts(root, deliveries);
}

/** Joins the chips to the table's Result filter, both ways. */
export function joinChips() {
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
