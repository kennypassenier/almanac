// The markup of the set-aside section: one alert per cause with its events in a data table, a selection bar and the
// per-row Retry and Dismiss buttons. Pure strings; aside.js puts them on the page.
import { esc, moment } from '../lib/format.js';
import { events } from './words.js';

/** The set-aside events by cause, the cause met first first (the list is newest first). @param {any[]} list */
export function groupByCause(list) {
    /** @type {Map<string, { key: string, cause: string, remedy: string, items: any[] }>} */
    const groups = new Map();
    for (const item of list) {
        const key = `${item.cause}\n${item.remedy}`;
        const group = groups.get(key) ?? { key, cause: item.cause, remedy: item.remedy, items: [] };
        group.items.push(item);
        groups.set(key, group);
    }
    return [...groups.values()];
}

/** @param {any} item */
function rowOf(item) {
    return `<tr data-kp-row-key="${esc(item.id)}" data-kp-row-label="${esc(item.title)}">
<td data-label="Select"><input class="kp-field__check" type="checkbox" data-kp-select-row aria-label="Select ${esc(item.title)}" /></td>
<td data-label="Event">${esc(item.title)}</td>
<td data-label="Source">${esc(item.source)}</td>
<td data-label="Set aside" data-kp-sort-value="${item.at}">${moment(item.at)}</td>
<td data-label="Reason">${esc(item.short)}</td>
<td data-label="Actions"><div class="ad-row-acts">
<button type="button" class="kp-button" data-ad-act="retry" data-ad-key="${esc(item.id)}" aria-label="Retry ${esc(item.title)}">Retry</button>
<button type="button" class="kp-button kp-button--ghost" data-ad-act="dismiss" data-ad-key="${esc(item.id)}" aria-label="Dismiss ${esc(item.title)}">Dismiss</button>
</div></td>
</tr>`;
}

/** The bar that appears over a table when rows are ticked. @param {string} tableId @param {string} what */
function barHtml(tableId, what) {
    return `<div class="kp-selection" data-kp-selection data-kp-selection-for="${tableId}" data-kp-placement="inline" data-kp-selection-names="count" data-kp-undo-ms="6000" aria-label="Selected events: ${esc(what)}">
<div class="kp-selection__actions">
<button type="button" class="kp-button" data-kp-selection-action="retry">Retry</button>
<button type="button" class="kp-button kp-button--ghost" data-kp-selection-action="dismiss">Dismiss</button>
</div>
</div>`;
}

/** @param {{ key: string, cause: string, remedy: string, items: any[] }} group @param {number} index */
export function groupHtml(group, index) {
    const sources = [...new Set(group.items.map((item) => item.source))];
    const what = `${events(group.items.length)} from ${sources.join(', ')} ${group.items.length === 1 ? 'was' : 'were'} set aside`;
    const tableId = `ad-group-table-${index}`;
    return `<article class="ad-group" data-ad-group="${index}">
<div class="kp-alert kp-alert--destructive" role="group" aria-label="${esc(what)}">
<strong class="kp-alert__what">${esc(what)}</strong>
<span class="kp-alert__why">${esc(group.cause)}</span>
<span class="kp-alert__remedy"><span class="kp-alert__remedy-label">What now</span> ${esc(group.remedy)}</span>
</div>
${barHtml(tableId, what)}
<div class="kp-datatable ad-group__table" id="${tableId}" data-kp-datatable>
<div class="kp-table-wrap"><table class="kp-table">
<caption class="kp-sr-only">${esc(what)}: ${esc(group.cause)}</caption>
<thead><tr>
<th scope="col"><input class="kp-field__check" type="checkbox" data-kp-select-all aria-label="Select every event of this cause" /></th>
<th scope="col" data-kp-sort="text">Event</th>
<th scope="col" data-kp-sort="text">Source</th>
<th scope="col" data-kp-sort="number">Set aside</th>
<th scope="col" data-kp-sort="text">Reason</th>
<th scope="col">Actions</th>
</tr></thead>
<tbody>${group.items.map(rowOf).join('\n')}</tbody>
</table></div>
<p class="kp-datatable__status" data-kp-datatable-status role="status" aria-live="polite"></p>
</div>
</article>`;
}
