// The Journal section: one read of /status/journal, and what the page shows when the first read fails.
import { setStackbar } from '/static/kp/dist/js/index.js';
import { readJson } from './read.js';
import { $, hold, journalCard } from './card.js';
import { journal } from './state.js';
import { drawGlance } from './glance.js';
import { deliveriesTable, syncTable } from './deliveries.js';

/** One read of the journal; throws a ReadError. */
async function loadJournal() {
    const d = await readJson('/status/journal');
    journal.last = d;
    drawGlance();
    syncTable(d.deliveries);
    const t = deliveriesTable();
    t?.busy(null);
    t?.state('ready');
    const card = journalCard();
    if (card) hold(card).set('ready');
    journal.first = false;
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
export async function refreshJournal() {
    try {
        await loadJournal();
        return true;
    } catch (error) {
        console.warn('status: the journal could not be read', error);
        if (journal.first) journalFailed();
        return false;
    }
}
