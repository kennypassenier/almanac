// Section 3 of the deliveries page: the entries set aside, one alert per cause with the events as a data table under it.
// The states are the package's: the busy card over the skeleton, the empty plate, the failed plate with Try again.
import { attachArt, attachDataTables, statePlate, openBusyCard } from '/static/kp/dist/kp-themes.min.js';
import { esc, moment } from '../lib/format.js';
import { state } from './state.js';
import { asideEl, asideLoading, asideEmpty, asideFailed } from './dom.js';
import { events } from './words.js';

/** @type {import('/static/kp/dist/kp-themes.min.js').BusyCard | null} */
let asideBusy = null;
/** What the groups on screen were drawn from, so a poll that changed nothing leaves the tables (and their ticks) alone. */
let drawnFrom = '';
/** Lets the tables on screen go before new ones are made. @type {(() => void) | null} */
let detachTables = null;

/** @param {() => void} onRetry reads the journal again */
export function initAside(onRetry) {
    statePlate(document, { kind: 'empty', title: 'Nothing has been set aside', text: 'Every event Almanac accepted is either delivered or still being tried.', into: asideEmpty });
    asideEmpty.setAttribute('data-kp-art', 'done');
    attachArt(asideEmpty.parentElement ?? document);
    statePlate(document, { kind: 'error', title: 'Could not read the entries set aside', text: 'The journal did not answer.', actionLabel: 'Try again', onAction: onRetry, into: asideFailed });
}

/** The busy card is open exactly while the first read is pending. */
function syncBusyCard() {
    const frame = /** @type {HTMLElement} */ (asideLoading.parentElement);
    if (state.status === 'loading' && !asideBusy) asideBusy = openBusyCard(frame, { words: 'Reading the journal', area: () => [asideLoading], arrive: false });
    else if (state.status !== 'loading' && asideBusy) {
        void asideBusy.close();
        asideBusy = null;
    }
}

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
    return `<tr data-kp-row-key="${esc(item.id)}">
<td data-label="Event">${esc(item.title)}</td>
<td data-label="Source">${esc(item.source)}</td>
<td data-label="Set aside" data-kp-sort-value="${item.at}">${moment(item.at)}</td>
<td data-label="Reason">${esc(item.short)}</td>
</tr>`;
}

/** @param {{ key: string, cause: string, remedy: string, items: any[] }} group @param {number} index */
function groupHtml(group, index) {
    const sources = [...new Set(group.items.map((item) => item.source))];
    const what = `${events(group.items.length)} from ${sources.join(', ')} ${group.items.length === 1 ? 'was' : 'were'} set aside`;
    return `<article class="ad-group" data-ad-group="${index}">
<div class="kp-alert kp-alert--destructive" role="group" aria-label="${esc(what)}">
<strong class="kp-alert__what">${esc(what)}</strong>
<span class="kp-alert__why">${esc(group.cause)}</span>
<span class="kp-alert__remedy"><span class="kp-alert__remedy-label">What now</span> ${esc(group.remedy)}</span>
</div>
<div class="kp-datatable ad-group__table" data-kp-datatable>
<div class="kp-table-wrap"><table class="kp-table">
<caption class="kp-sr-only">${esc(what)}: ${esc(group.cause)}</caption>
<thead><tr>
<th scope="col" data-kp-sort="text">Event</th>
<th scope="col" data-kp-sort="text">Source</th>
<th scope="col" data-kp-sort="number">Set aside</th>
<th scope="col" data-kp-sort="text">Reason</th>
</tr></thead>
<tbody>${group.items.map(rowOf).join('\n')}</tbody>
</table></div>
<p class="kp-datatable__status" data-kp-datatable-status role="status" aria-live="polite"></p>
</div>
</article>`;
}

/** @param {any[]} list */
function drawGroups(list) {
    const from = JSON.stringify(list.map((item) => [item.id, item.cause, item.remedy]));
    if (from === drawnFrom) return;
    drawnFrom = from;
    detachTables?.();
    asideEl.innerHTML = groupByCause(list).map(groupHtml).join('\n');
    detachTables = attachDataTables(asideEl);
}

export function renderAside() {
    syncBusyCard();
    asideLoading.hidden = state.status !== 'loading';
    asideFailed.hidden = state.status !== 'error';
    const list = state.status === 'ready' ? state.data.aside : [];
    asideEl.hidden = list.length === 0;
    asideEmpty.hidden = !(state.status === 'ready' && list.length === 0);
    drawGroups(list);
}
