// status page: the Journal and Sources sections inside the kit's section cards (chassis-rs 3.9.0). The kit draws each card and
// its states and owns the page's one refresh control; this module fills the two stages from /status/journal and
// /status/sources, reads them again on the kit's refresh, and keeps rows updating in place by their stable id.
// The parts live in js/status/*.js.

import { setStackbar, attachSegmented, attachAgo, attachSparklines, attachKpiToggles, attachKpiStrips, attachCharts, attachTicks } from '/static/kp/dist/js/index.js';
import { tableHandle } from './status/read.js';
import { $, hold, journalCard, sourcesCard } from './status/card.js';
import { journal, sources } from './status/state.js';
import { drawGlance } from './status/glance.js';
import { holdTableFootprint } from './status/footprint.js';
import { joinChips, tableLoading } from './status/deliveries.js';
import { refreshJournal } from './status/journal.js';
import { loadSources } from './status/sources.js';

/** Resolves once chassis.js has attached the page (the kit's own parts attach after that, so handles are awaited). */
const ready = /** @type {Promise<unknown>} */ (/** @type {any} */ (window).chassis?.ready ?? new Promise((resolve) => document.addEventListener('chassis:ready', resolve, { once: true })));

const hasJournal = () => document.querySelector('[data-st-journal]') !== null;
const hasSources = () => document.querySelector('[data-st-sources]') !== null;

/** The outcome bar's loading state: never blank before the first answer. */
function outcomesLoading() {
    const bar = $('#st-outcomes');
    if (bar) setStackbar(bar, { title: 'Outcomes', state: 'loading', parts: [] });
}

/** One read of everything, never two at once: the kit's first refresh and this module's own first read are the same read. */
let reading = /** @type {Promise<boolean> | null} */ (null);
function run() {
    if (!reading) {
        reading = Promise.all([hasJournal() ? refreshJournal() : true, hasSources() ? loadSources() : true])
            .then(([journalOk, sourcesOk]) => journalOk && sourcesOk)
            .finally(() => {
                reading = null;
            });
    }
    return reading;
}

/** The parts the kit's own page script does not attach (it takes the stage's datatables, ages and cards). */
function attachStage() {
    const stage = /** @type {HTMLElement} */ (document.querySelector('.kp-ops__body') ?? document.body);
    attachSparklines(stage);
    attachKpiToggles(stage);
    attachKpiStrips(stage);
    attachCharts(stage);
    attachSegmented(stage);
    attachTicks(stage);
}

async function initJournal() {
    // The outcome bar and the trend are never blank before the first answer: the theme's own loading state, at their height.
    outcomesLoading();
    holdTableFootprint();
    attachAgo(document);
    joinChips();
    await tableHandle(/** @type {HTMLElement} */ ($('#st-table')));
    tableLoading();
    $('#st-table')?.addEventListener('kp-datatable-retry', () => {
        tableLoading();
        run();
    });
    const card = journalCard();
    card?.addEventListener('kp-card-retry', () => {
        hold(card).set('ready');
        tableLoading();
        outcomesLoading();
        journal.first = true;
        run();
    });
    // The outcome bar's own Retry (a failed first read) reads again like the card's.
    $('#st-outcomes')?.addEventListener('kp-stackbar-retry', () => {
        outcomesLoading();
        run();
    });
}

function initSources() {
    sourcesCard()?.addEventListener('kp-card-retry', () => {
        const card = sourcesCard();
        if (card) hold(card).set('loading', { words: 'Reading the profile files' });
        sources.first = true;
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

document.addEventListener('kp-kpi-toggle', () => drawGlance());

/** The page's start, after chassis.js attached it. Not awaited at the top level: the kit attaches after
 * DOMContentLoaded, and a module still awaiting would hold that event back, so the page never finished loading. */
async function start() {
    await ready;
    attachStage();

    if (hasSources()) {
        const card = sourcesCard();
        if (card) hold(card).set('loading', { words: 'Reading the profile files' });
    }
    if (hasJournal()) await initJournal();

    // The page's one refresh control is the kit's; a refresh there reads these sections again. The kit's own read answers
    // the control (its freshness line), so this module only listens.
    document.querySelector('.kp-ops .kp-refresh')?.addEventListener('kp-refresh', () => {
        run();
    });
    run();

    if (hasSources()) initSources();
}

start();
