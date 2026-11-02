// Loading for the Calendars page: the two reads, and the redraw of every section after each answer.
import { api } from './api.js';
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

/** @param {boolean} [quiet] keep what is shown while the answer comes (a fix button is running, or a refusal is being checked) */
export async function loadGoogle(quiet = false) {
    const turn = (googleTurn += 1);
    if (!quiet) {
        state.google = null;
        render();
    }
    try {
        // A refused answer carries no calendars; every section reads the list, so it is always there.
        const google = { calendars: [], .../** @type {any} */ (await api('/calendars/data/google')) };
        if (turn !== googleTurn) return;
        state.google = google;
    } catch (e) {
        if (turn !== googleTurn) return;
        const err = /** @type {any} */ (e);
        state.google = { ok: false, can_create: true, calendars: [], error: err.error, remedy: err.remedy };
    }
    render();
}

/** @param {boolean} [quiet] as in loadGoogle */
export async function loadProfiles(quiet = false) {
    const turn = (profilesTurn += 1);
    if (!quiet) {
        state.profiles = null;
        render();
    }
    try {
        const profiles = { ok: true, .../** @type {any} */ (await api('/calendars/data/profiles')) };
        if (turn !== profilesTurn) return;
        state.profiles = profiles;
    } catch (e) {
        if (turn !== profilesTurn) return;
        const err = /** @type {any} */ (e);
        state.profiles = { ok: false, folder_ok: false, served: [], unserved: [], sources: [], trend: null, error: err.error };
    }
    render();
}

/** Asks for everything at once; each card fills as its own answer arrives. @param {boolean} [quiet] */
export function load(quiet = false) {
    return Promise.all([loadGoogle(quiet), loadProfiles(quiet)]);
}

Object.assign(hooks, { render, load, loadGoogle, loadProfiles });
