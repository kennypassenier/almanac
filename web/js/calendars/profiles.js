// Section 6 of the Calendars page: the profile files, served and not served, and the typed delete of an unserved file.
import { toast, runFlashButton, statePlate, setProblems, attachArt } from '/static/kp/dist/js/index.js';
import { esc } from '../lib/format.js';
import { api, reportFailure } from './api.js';
import { state, hooks, calName } from './state.js';
import { must, allServed, problemsBox, profileCard, servedBody, servedTable, servedWrap } from './dom.js';

const NONE_SERVED = ['No profile file is being served', 'Every source posts with a served profile; none is served right now.'];

/** The what-is-lost list of a file's typed delete, kept as a template the button names. @param {{ file: string }} u */
function losesFor(u) {
    const id = `cal-lose-file-${u.file.replace(/[^a-z0-9]/gi, '-')}`;
    if (!document.getElementById(id)) {
        const t = document.createElement('template');
        t.id = id;
        t.innerHTML = `<ul class="kp-type-confirm__list"><li>The file ${esc(u.file)} from the profiles folder</li><li>Nothing else: it is not served, so no source uses it</li></ul>`;
        profileCard.append(t);
    }
    return id;
}

/** The served table's own empty plate, worded for the case. @param {string} title @param {string} text @param {HTMLElement} [action] */
function servedEmpty(title, text, action) {
    const served = must('[data-kp-datatable-empty]', servedWrap);
    const plate = statePlate(document, { kind: 'empty', title, text, into: served });
    plate.setAttribute('data-kp-art', 'none');
    plate.querySelector('.kp-stateplate__title')?.classList.add('kp-empty__title');
    if (action) plate.append(action);
}

/** The Delete button of an unserved file's problem row. @param {{ file: string }} u */
function deleteFileButton(u) {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'kp-button kp-button--destructive kp-button--sm';
    b.setAttribute('data-kp-destructive', '');
    b.setAttribute('data-kp-confirm', `Delete the file ${u.file}?`);
    b.setAttribute('data-kp-confirm-type', u.file);
    b.setAttribute('data-kp-confirm-loses', losesFor(u));
    b.setAttribute('data-kp-confirm-label', 'Delete file');
    b.setAttribute('data-cal-delete-file', u.file);
    b.textContent = 'Delete the file…';
    return b;
}

/** @param {any} s one served file */
function servedRow(s) {
    const warn = s.health === 'warn';
    const calendar = state.google?.ok ? calName(s.calendar_id) : 'Not known while Google does not answer';
    return `<tr data-kp-row-key="${esc(s.file)}">
<td data-label="Source">${esc(s.source)}</td>
<td data-label="Calendar">${esc(calendar)}</td>
<td data-label="Latest delivery" data-kp-sort-value="${warn ? 1 : 0}"><span class="kp-badge kp-badge--${warn ? 'warning' : 'success'}">${warn ? 'Failed' : 'Fine'}</span></td>
</tr>`;
}

/** The plate the served table shows when it has no rows: no files at all, or none served. @param {{ served: any[], unserved: any[] }} p */
function drawServedEmpty(p) {
    if (p.served.length || p.unserved.length) return servedEmpty(...(/** @type {[string, string]} */ (NONE_SERVED)));
    const add = document.createElement('a');
    add.className = 'kp-button kp-button--sm kp-button--primary';
    add.href = '/clients';
    add.textContent = 'Add a source';
    servedEmpty('No profile files yet', 'Add a source on the Sources page and Almanac writes its profile here.', add);
}

export function renderProfiles() {
    const p = state.profiles;
    if (!p) {
        allServed.hidden = true;
        servedTable.busy({ text: 'Reading the profile files', overlay: true });
        servedTable.state('loading');
        return;
    }
    if (!p.ok || !p.folder_ok) {
        allServed.hidden = true;
        servedTable.busy(null);
        servedTable.fail('The profiles folder could not be read: Almanac has no access to it right now.');
        return;
    }
    servedTable.busy(null);
    drawServedEmpty(p);
    setProblems(
        problemsBox,
        p.unserved.map((u) => ({ key: u.file, what: `${u.file} is not being served`, why: u.why, remedy: u.remedy, severity: /** @type {const} */ ('warning'), action: deleteFileButton(u) })),
    );
    allServed.hidden = p.unserved.length > 0 || !p.served.length;
    servedWrap.hidden = false;
    servedBody.innerHTML = p.served.map(servedRow).join('\n');
    servedTable.refresh();
    servedTable.state('ready');
}

/** Deletes an unserved file on the server; the button shows the outcome. @param {string} file */
async function removeFile(file) {
    try {
        await api('/profiles/remove', { file });
    } catch (e) {
        return /** @type {any} */ (e);
    }
    return { word: 'Deleted' };
}

export function initProfiles() {
    // The empty plate of the served table is worded for "none served" until a read says otherwise.
    servedEmpty(...(/** @type {[string, string]} */ (NONE_SERVED)));
    attachArt(servedWrap);
    servedWrap.addEventListener('kp-datatable-retry', () => void hooks.loadProfiles());
    problemsBox.addEventListener('click', (event) => {
        const button = event.target instanceof Element ? /** @type {HTMLElement | null} */ (event.target.closest('[data-cal-delete-file]')) : null;
        if (!button) return;
        const file = button.getAttribute('data-cal-delete-file') ?? '';
        runFlashButton(button, () => removeFile(file), { refusal: 'toast', busyLabel: 'Deleting' })
            .then((result) => {
                if (!result?.ok || !state.profiles) return;
                state.profiles.unserved = state.profiles.unserved.filter((u) => u.file !== file);
                hooks.render();
                toast(`Deleted ${file}.`, { className: 'kp-toast--success' });
            })
            .catch(reportFailure);
    });
}
