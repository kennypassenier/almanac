// The Journal section: the `journal` part of /status/data, and what the page shows when the first read fails.
import { setStackbar } from '/static/kp/dist/kp-themes.min.js';
import { ReadError } from './read.js';
import { readPart, partWords } from '../lib/page-data.js';
import { $, hold, journalCard } from './card.js';
import { journal } from './state.js';
import { drawGlance } from './glance.js';
import { deliveriesTable, syncTable } from './deliveries.js';
import { warnRead } from '../lib/leaving.js';

/** Draws the journal part (read alone when the page's merged read did not bring it); throws a ReadError.
 * @param {import('../lib/page-data.js').Part} [given] */
async function loadJournal(given) {
    const part = given ?? (await readPart('/status/data', 'journal'));
    if (!part.ok) throw new ReadError(partWords(part));
    const d = part.data;
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

/** @param {import('../lib/page-data.js').Part} [given] the part from the page's merged read
 * @returns {Promise<boolean>} whether the read worked; a later failure leaves what is shown (the kit's connection line says it) */
export async function refreshJournal(given) {
    try {
        await loadJournal(given);
        return true;
    } catch (error) {
        warnRead('status: the journal could not be read', error);
        if (journal.first) journalFailed();
        return false;
    }
}
