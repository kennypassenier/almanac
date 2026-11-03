// Section 3 of the deliveries page: the entries set aside, one alert per cause with the events as a data table under it.
// The states are the package's: the busy card over the skeleton, the empty plate, the failed plate with Try again.
import { attachArt, attachDataTables, attachSelectionBars, statePlate, openBusyCard } from '/static/kp/dist/js/index.js';
import { state } from './state.js';
import { asideEl, asideLoading, asideEmpty, asideFailed } from './dom.js';
import { groupByCause, groupHtml } from './groups.js';
import { act, held, initActions } from './actions.js';

/** @type {import('/static/kp/dist/js/index.js').BusyCard | null} */
let asideBusy = null;
/** What the groups on screen were drawn from, so a poll that changed nothing leaves the tables (and their ticks) alone. */
let drawnFrom = '';
/** Lets the tables on screen go before new ones are made. @type {(() => void) | null} */
let detachTables = null;

/**
 * @param {() => void} onRetry reads the journal again, behind the loading state
 * @param {() => Promise<void>} reload reads it quietly */
export function initAside(onRetry, reload) {
    initActions({ redraw: renderAside, reload });
    asideEl.addEventListener('click', onRowAction);
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

/** The keys of the rows ticked now, so a redraw gives the ticks back. */
function tickedKeys() {
    const boxes = /** @type {NodeListOf<HTMLInputElement>} */ (asideEl.querySelectorAll('[data-kp-select-row]:checked'));
    return [...boxes].map((box) => box.closest('tr')?.getAttribute('data-kp-row-key') ?? '');
}

/** @param {string[]} keys */
function tick(keys) {
    for (const row of asideEl.querySelectorAll('tr[data-kp-row-key]')) {
        if (!keys.includes(row.getAttribute('data-kp-row-key') ?? '')) continue;
        const box = /** @type {HTMLInputElement | null} */ (row.querySelector('[data-kp-select-row]'));
        if (!box) continue;
        box.checked = true;
        box.dispatchEvent(new Event('change', { bubbles: true }));
    }
}

/** A selection bar's Retry or Dismiss: act on the keys it carries. @param {Event} event */
function onBarAction(event) {
    const { action, keys } = /** @type {CustomEvent} */ (event).detail;
    if (action === 'retry' || action === 'dismiss') act(action, keys);
}

/** A row's own Retry or Dismiss button. @param {MouseEvent} event */
function onRowAction(event) {
    const button = /** @type {HTMLElement | null} */ ((/** @type {HTMLElement} */ (event.target)).closest('[data-ad-act]'));
    const kind = button?.getAttribute('data-ad-act');
    const key = button?.getAttribute('data-ad-key');
    if (key && (kind === 'retry' || kind === 'dismiss')) act(kind, [key]);
}

/** Keys of the rows after the focused one, then before it (nearest first); null when focus is not in the section. */
function focusPlan() {
    const active = document.activeElement;
    if (!active || !asideEl.contains(active)) return null;
    const keys = [...asideEl.querySelectorAll('tr[data-kp-row-key]')].map((row) => row.getAttribute('data-kp-row-key') ?? '');
    const at = keys.indexOf(active.closest('tr')?.getAttribute('data-kp-row-key') ?? '');
    return at < 0 ? [] : [...keys.slice(at + 1), ...keys.slice(0, at).reverse()];
}

/** Gives focus back after a redraw: the wanted row, else the next one, else the first, else the section itself. @param {string[] | null} plan @param {string | undefined} wanted */
function restoreFocus(plan, wanted) {
    if (!plan && !wanted) return;
    const rows = [...asideEl.querySelectorAll('tr[data-kp-row-key]')];
    const byKey = (/** @type {string} */ key) => rows.find((row) => row.getAttribute('data-kp-row-key') === key);
    const row = (wanted ? byKey(wanted) : undefined) ?? (plan ?? []).map(byKey).find(Boolean) ?? rows[0];
    const target = row?.querySelector('[data-ad-act]') ?? (asideEl.hidden ? asideEmpty : asideEl);
    if (target === asideEmpty) asideEmpty.tabIndex = -1;
    /** @type {HTMLElement} */ (target).focus();
}

/** @param {any[]} list @param {string} [focusKey] a row key that should hold focus after the redraw */
function drawGroups(list, focusKey) {
    const from = JSON.stringify(list.map((item) => [item.id, item.cause, item.remedy]));
    if (from === drawnFrom) return;
    drawnFrom = from;
    const plan = focusPlan();
    const ticked = tickedKeys();
    detachTables?.();
    asideEl.innerHTML = groupByCause(list).map(groupHtml).join('\n');
    const detachData = attachDataTables(asideEl);
    const detachBars = attachSelectionBars(asideEl);
    detachTables = () => {
        detachBars();
        detachData();
    };
    for (const bar of asideEl.querySelectorAll('[data-kp-selection]')) bar.addEventListener('kp-selection-action', onBarAction);
    tick(ticked);
    restoreFocus(plan, focusKey);
}

/** @param {string} [focusKey] a row key that should hold focus after the redraw */
export function renderAside(focusKey) {
    syncBusyCard();
    asideLoading.hidden = state.status !== 'loading';
    asideFailed.hidden = state.status !== 'error';
    const all = state.status === 'ready' ? state.data.aside : [];
    // An id the journal no longer lists has been dealt with; it needs no holding any more.
    if (state.status === 'ready') for (const id of held) if (!all.some((item) => item.id === id)) held.delete(id);
    const list = all.filter((item) => !held.has(item.id));
    asideEl.hidden = list.length === 0;
    asideEmpty.hidden = !(state.status === 'ready' && list.length === 0);
    drawGroups(list, focusKey);
}
