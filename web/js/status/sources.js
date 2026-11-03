// The Sources section: one read of /status/sources, kept in step by each source's stable id.
import { attachStatusLists, buildStatusList, setStatus } from '/static/kp/dist/js/index.js';
import { readJson } from './read.js';
import { $, hold, sourcesCard } from './card.js';
import { sources as sourcesState } from './state.js';

/** @param {any[]} rows @param {HTMLElement} host */
function drawList(rows, host) {
    let list = $('.kp-statuslist', host);
    const ids = rows.map((r) => r.id).join('\n');
    if (list && list.getAttribute('data-st-ids') === ids) {
        for (const r of rows) setStatus(list, r.id, { state: r.state, detail: r.detail, fix: r.fix ?? '' });
        return;
    }
    host.replaceChildren();
    list = buildStatusList(rows, { label: 'Sources and their calendars', single: true, retry: false });
    list.classList.add('kp-statuslist--rows');
    list.setAttribute('data-st-ids', ids);
    host.append(list);
    attachStatusLists(host);
}

/** @param {any} d */
function drawSources(d) {
    const host = /** @type {HTMLElement} */ ($('[data-st-sources-list]'));
    const rows = d.sources.map((/** @type {any} */ s) => ({ id: s.id, label: s.id, detail: s.detail, state: s.state, ...(s.fix ? { fix: s.fix } : {}) }));
    const intro = $('#st-sources-intro');
    if (intro) intro.textContent = `Which source writes to which calendar, and whether its profile file is being served. Profile files, schema version ${d.schemaVersion}.`;
    const card = /** @type {HTMLElement} */ (sourcesCard());
    if (rows.length === 0) {
        host.replaceChildren();
        hold(card).set('empty', { title: 'No profile files on disk', text: 'Add a profile file for a source, then reload the profiles.' });
        return;
    }
    drawList(rows, host);
    hold(card).set('ready');
}

/** @returns {Promise<boolean>} */
export async function loadSources() {
    try {
        drawSources(await readJson('/status/sources'));
        sourcesState.first = false;
        return true;
    } catch (error) {
        console.warn('status: the sources could not be read', error);
        if (sourcesState.first) {
            const card = sourcesCard();
            if (card) hold(card).set('error', { title: 'The profile files could not be read', text: 'Almanac could not list them. Try again in a moment.' });
        }
        return false;
    }
}
