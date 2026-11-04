// Loading for the Calendars page: the two reads, and the redraw of every section after each answer.
import { api } from './api.js';
import { readParts } from '../lib/page-data.js';
import { state, hooks } from './state.js';
import { renderStrip } from './strip.js';
import { renderAttention } from './fixes.js';
import { renderTable } from './table.js';
import { renderProfiles } from './profiles.js';
import { renderUses } from './uses.js';
import { refreshNewButton } from './new-dialog.js';

export function render() {
    renderStrip();
    renderAttention();
    renderTable();
    renderProfiles();
    renderUses();
    refreshNewButton();
}

let googleTurn = 0;
let profilesTurn = 0;

/** The card's data: from the page's merged read when given, else read alone; rejects with `{ error, remedy }`.
 * @param {string} name @param {Promise<import('../lib/page-data.js').Part | undefined>} [given] */
async function readCard(name, given) {
    const part = given ? await given : undefined;
    if (!part) return api(`/calendars/data/${name}`);
    if (!part.ok) throw { error: part.error, remedy: part.remedy };
    return part.data;
}

/** @param {boolean} [quiet] keep what is shown while the answer comes (a fix button is running, or a refusal is being checked)
 * @param {Promise<import('../lib/page-data.js').Part | undefined>} [given] the part from the page's merged read */
export async function loadGoogle(quiet = false, given = undefined) {
    const turn = (googleTurn += 1);
    if (!quiet) {
        state.google = null;
        render();
    }
    try {
        // A refused answer carries no calendars; every section reads the list, so it is always there.
        const google = { calendars: [], .../** @type {any} */ (await readCard('google', given)) };
        if (turn !== googleTurn) return;
        state.google = google;
    } catch (e) {
        if (turn !== googleTurn) return;
        const err = /** @type {any} */ (e);
        state.google = { ok: false, can_create: true, calendars: [], error: err.error, remedy: err.remedy };
    }
    render();
}

/** @param {boolean} [quiet] as in loadGoogle @param {Promise<import('../lib/page-data.js').Part | undefined>} [given] as in loadGoogle */
export async function loadProfiles(quiet = false, given = undefined) {
    const turn = (profilesTurn += 1);
    if (!quiet) {
        state.profiles = null;
        render();
    }
    try {
        const profiles = { ok: true, .../** @type {any} */ (await readCard('profiles', given)) };
        if (turn !== profilesTurn) return;
        state.profiles = profiles;
    } catch (e) {
        if (turn !== profilesTurn) return;
        const err = /** @type {any} */ (e);
        state.profiles = { ok: false, folder_ok: false, served: [], unserved: [], sources: [], trend: null, error: err.error };
    }
    render();
}

/** Asks for both cards in one read of /calendars/data, each part on its own; a merged read that cannot start leaves
 * each card to read its own part. @param {boolean} [quiet] */
export function load(quiet = false) {
    const parts = readParts('/calendars/data', ['google', 'profiles']).catch(() => undefined);
    return Promise.all([loadGoogle(quiet, parts.then((p) => p?.google)), loadProfiles(quiet, parts.then((p) => p?.profiles))]);
}

Object.assign(hooks, { render, load, loadGoogle, loadProfiles });
