// Section 2 of the Calendars page: the attention band, the fix buttons in it, and the same profile reload in the
// header menu.
import { runFlashButton, setAttention } from '/static/kp/dist/kp-themes.min.js';
import { plural } from '../lib/format.js';
import { api, refusal, flavour, said, reportFailure } from './api.js';
import { state, hooks } from './state.js';
import { band, menuWrap } from './dom.js';

const fix = (/** @type {string} */ key, /** @type {string} */ label) => {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'kp-button kp-button--sm';
    b.setAttribute('data-cal-fix', key);
    b.textContent = label;
    return b;
};

export function renderAttention() {
    const { google, profiles } = state;
    /** @type {import('/static/kp/dist/kp-themes.min.js').AttentionItem[]} */
    const items = [];
    if (google && !google.ok) {
        items.push({
            key: 'google',
            severity: 'critical',
            title: 'Google Calendar cannot be reached',
            text: 'Calendars cannot be listed or made, and new events wait in the journal until Google answers again.',
            action: fix('google', 'Check again'),
        });
    }
    const files = profiles?.unserved ?? [];
    if (files.length) {
        const first = files.length === 1 ? files[0].file : `${files[0].file} and ${files.length - 1} more`;
        items.push({
            key: 'file',
            severity: 'warning',
            title: `${plural(files.length, 'profile file is', 'profile files are')} not being served`,
            text: `${first}: the ${files.length === 1 ? 'source it describes receives' : 'sources they describe receive'} nothing, and a post to ${files.length === 1 ? 'it' : 'them'} answers 401.`,
            action: fix('file', 'Reload profiles from disk'),
        });
    }
    setAttention(band, items);
}

/** Reloads the profile files; resolves with what the server says. */
const reloadFiles = () => /** @type {Promise<{ served: number, unserved: number }>} */ (api('/sources/reload', {}));

/** The outcome of a fix: what the button shows, and what to say once it has settled. @typedef {{ result: any, after?: () => void }} Fixed */

/** @returns {Promise<Fixed>} */
async function fixGoogle() {
    await hooks.loadGoogle(true);
    const google = state.google;
    if (!google?.ok) return { result: refusal(google?.error ?? 'Google did not answer.', google?.remedy ?? 'Try again in a moment.') };
    return { result: 'Done', after: () => flavour('success', 'Google Calendar answers again; the waiting events are being delivered.') };
}

/** @returns {Promise<Fixed>} */
async function fixFile() {
    const before = { served: state.profiles?.served.length ?? 0, unserved: state.profiles?.unserved.length ?? 0 };
    /** @type {{ served: number, unserved: number }} */
    let now;
    try {
        now = await reloadFiles();
    } catch (e) {
        return { result: /** @type {any} */ (e) };
    }
    await hooks.loadProfiles(true);
    if (!now.unserved) return { result: 'Done', after: () => flavour('success', `Profile files read again: ${now.served} served, 0 not served.`) };
    if (now.unserved === before.unserved && now.served === before.served) {
        return { result: { word: 'Same' }, after: () => flavour('info', 'Reloaded: the same files are served, nothing was changed.') };
    }
    return { result: { word: 'Partly' }, after: () => flavour('warning', `Reloaded, but ${plural(now.unserved, 'profile file is', 'profile files are')} still not served.`) };
}

export function initFixes() {
    band.addEventListener('click', (event) => {
        const button = event.target instanceof Element ? /** @type {HTMLElement | null} */ (event.target.closest('[data-cal-fix]')) : null;
        if (!button) return;
        const key = button.getAttribute('data-cal-fix');
        /** @type {(() => void) | undefined} */
        let after;
        runFlashButton(
            button,
            async () => {
                const fixed = await (key === 'google' ? fixGoogle() : fixFile());
                after = fixed.after;
                return fixed.result;
            },
            { refusal: 'toast', busyLabel: key === 'google' ? 'Checking' : 'Reloading', width: 'keep' },
        )
            .then(() => after?.())
            .catch(reportFailure);
    });

    // "Reload profiles from disk" in the header menu: the same call, and the page stays where it is.
    menuWrap.addEventListener('kp-menu-select', async () => {
        try {
            const now = await reloadFiles();
            await hooks.loadProfiles(true);
            flavour('success', `Profile files read again: ${now.served} served, ${now.unserved} not served.`);
        } catch (e) {
            flavour('error', said(/** @type {any} */ (e)));
        }
    });
}
