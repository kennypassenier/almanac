// Retry and Dismiss on set-aside events. The click hides the rows at once and shows a toast with Undo; the request is
// sent only when the toast's window closes, and Undo cancels it on this side (nothing was sent yet).
import { undoToast } from '/static/kp/dist/kp-themes.min.js';
import { events } from './words.js';
import { warnRead } from '../lib/leaving.js';

/** How long Undo stays open [ms]. */
const UNDO_MS = 6000;

/** What each action sends to and says. */
const KINDS = {
    retry: { path: '/deliveries/retry', words: (/** @type {number} */ n) => `${events(n)} will be sent again` },
    dismiss: { path: '/deliveries/dismiss', words: (/** @type {number} */ n) => `${events(n)} marked as handled` },
};

/** Ids whose rows are hidden while their toast counts down or their request is answered. @type {Set<string>} */
export const held = new Set();

/** @type {{ redraw: (focusKey?: string) => void, reload: () => Promise<void> }} */
let page = { redraw: () => {}, reload: async () => {} };

/** @param {{ redraw: (focusKey?: string) => void, reload: () => Promise<void> }} hooks redraw the section; read the journal again */
export function initActions(hooks) {
    page = hooks;
}

/** One POST. Rejects with the server's own plain sentence (it is what the toast shows). @param {string} path @param {string[]} ids */
async function send(path, ids) {
    let response;
    try {
        response = await fetch(path, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
            body: JSON.stringify({ ids }),
            credentials: 'same-origin',
        });
    } catch (cause) {
        warnRead(`deliveries: ${path} got no answer`, cause);
        throw new Error('Almanac did not answer. Check that the service is running, then try again');
    }
    /** @type {any} */
    let json = null;
    try {
        json = await response.json();
    } catch (cause) {
        // An HTML answer is the login page: the session ended.
        console.warn(`deliveries: ${path} did not answer JSON (status ${response.status})`, cause);
    }
    if (json === null) {
        const ended = response.status === 401 || new URL(response.url).pathname === "/login";
        throw new Error(ended ? 'Your session ended. Reload the page and log in again' : 'Almanac did not answer. Check that the service is running, then try again');
    }
    if (!response.ok) throw new Error(`${String(json.error ?? 'Almanac could not do that').replace(/\.$/, '')}. ${json.remedy ?? ''}`.trim());
    return json;
}

/**
 * Hides the rows and opens the undo toast.
 * @param {'retry' | 'dismiss'} kind @param {string[]} ids the event ids (row keys)
 */
export function act(kind, ids) {
    const fresh = ids.filter((id) => !held.has(id));
    if (fresh.length === 0) return;
    const give = () => {
        for (const id of fresh) held.delete(id);
        page.redraw(fresh[0]); // the toast and its Undo button go away, so focus goes to the first row put back
    };
    for (const id of fresh) held.add(id);
    page.redraw();
    undoToast(KINDS[kind].words(fresh.length), {
        ms: UNDO_MS,
        max: 3,
        commit: async () => {
            await send(KINDS[kind].path, fresh);
            // The ids stay held until an answer no longer lists them (see aside.js), so a poll already in flight cannot bring them back.
            await page.reload();
        },
        undo: give,
        restore: give,
        restored: 'Put back where it was',
    });
}
