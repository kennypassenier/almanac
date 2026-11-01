// The Try page's sources: the one source picker, the profile card, and the read of /try/api/sources that every card waits on.
import { setCardState, setCode, attachPatterns, setSegmentedItems, setSegmentedValues } from '/static/kp/dist/kp-themes.min.js';
import { api } from './api.js';
import { state, sourceById } from './state.js';
import { factRow } from './words.js';
import { $, profileCard, profileToml, editorCard, editorSource, resultCard, traceBody } from './dom.js';
import { idle, idleTrace, emptyResult } from './result.js';
import { refreshTry } from './try-button.js';

function drawProfile() {
    const s = sourceById(state.profileSource);
    if (!s) return;
    $('profile-title').textContent = `Profile of ${s.source_id}`;
    const kv = $('profile-kv');
    kv.innerHTML = [
        factRow('Source id', s.source_id, true),
        factRow('Calendar', s.calendar_name, false),
        factRow('Calendar id', s.calendar_id, true),
        factRow('Time zone', s.timezone, false),
        factRow('Default length', `${s.default_duration_minutes} minutes${s.default_is_fallback ? " (Almanac's own default)" : ''}`, false),
        factRow('Schema version', String(s.schema_version), false),
        factRow('File', s.file_name ?? 'Not found', Boolean(s.file_name)),
    ].join('');
    attachPatterns(kv);
    if (s.toml == null) setCode(profileToml, { text: '', title: 'Profile file', language: 'TOML', state: 'error', message: 'The profile file could not be read' });
    else setCode(profileToml, { text: s.toml, title: 'Profile file', language: 'TOML', lines: true, wrap: true, size: `${new Blob([s.toml]).size} B` });
}

/** One picker for the page: the profile shown and the source Try sends as are always the same. */
function drawPicker() {
    const items = state.sources.map((s) => ({ value: s.source_id, label: s.source_id }));
    setSegmentedItems($('profile-picker'), items);
    setSegmentedValues($('profile-picker'), state.profileSource ? [state.profileSource] : []);
    editorSource.textContent = `Sends as ${state.profileSource}`;
}

/** Everything on the page waits on the profiles (Try needs a source), so the whole page is loading, or failed, with them. */
function pageWaits() {
    setCardState(profileCard, 'loading');
    setCode(profileToml, { text: '', title: 'Profile file', language: 'TOML', state: 'loading', loadingWords: 'Reading the profile file' });
    setCardState(editorCard, 'loading');
    setCardState(resultCard, 'loading');
    setCode(traceBody, { text: '', title: 'Answer', language: 'JSON', state: 'loading', loadingWords: 'Reading the profiles' });
}

function pageFailed() {
    setCardState(profileCard, 'error', { title: 'Could not read the profiles', text: 'Almanac did not answer. Nothing else is affected.' });
    setCode(profileToml, { text: '', title: 'Profile file', language: 'TOML', state: 'error', message: 'The profiles could not be read' });
    setCardState(editorCard, 'error', { title: 'Could not read the profiles', text: 'Try needs a source, and Almanac did not answer.' });
    setCardState(resultCard, 'error', { title: 'Could not read the profiles', text: 'Try needs a source, and Almanac did not answer.' });
    setCode(traceBody, { text: '', title: 'Answer', language: 'JSON', state: 'error', message: 'The profiles could not be read' });
}

/** The page is ready for a first Try: the editor is open, nothing was tried. */
function pageIdle() {
    setCardState(editorCard, 'ready');
    idleTrace();
    emptyResult();
}

/** The empty card's action: ask the server to read the profile folder again, then read the sources. */
function reloadButton() {
    const reload = document.createElement('button');
    reload.type = 'button';
    reload.className = 'kp-button kp-button--primary';
    reload.textContent = 'Reload profiles from disk';
    reload.addEventListener('click', async () => {
        pageWaits();
        try {
            await fetch('/sources/reload', { method: 'POST', credentials: 'same-origin' });
        } catch (error) {
            // The read below says what is wrong; the console keeps the cause.
            console.warn('try: the profile reload request failed', error);
        }
        loadSources();
    });
    return reload;
}

export async function loadSources() {
    state.sourcesFailed = false;
    pageWaits();
    try {
        state.sources = (await api('/try/api/sources')).sources;
    } catch (error) {
        console.warn('try: the sources could not be read', error);
        state.sourcesFailed = true;
        pageFailed();
        return;
    }
    if (!state.sources.length) {
        setCardState(profileCard, 'empty', { title: 'No sources loaded', text: 'Almanac serves a source only when its profile file is in the profiles folder.', action: reloadButton() });
    } else {
        if (!sourceById(state.profileSource)) state.profileSource = state.sources[0].source_id;
        state.editorSource = state.profileSource;
        drawPicker();
        drawProfile();
        setCardState(profileCard, 'ready');
    }
    pageIdle();
    refreshTry();
}

export function initSources() {
    $('profile-picker').addEventListener('kp-segmented-change', (event) => {
        state.profileSource = /** @type {CustomEvent} */ (event).detail.value;
        state.editorSource = state.profileSource;
        editorSource.textContent = `Sends as ${state.profileSource}`;
        drawProfile();
        idle();
    });
    profileCard.addEventListener('kp-card-retry', loadSources);
}
