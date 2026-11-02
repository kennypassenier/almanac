// The Calendars page's talk with the server: one JSON call, and the words a refusal or a toast uses.
import { toast } from '/static/kp/dist/kp-themes.min.js';

/** A refusal in the shape the package shows: a sentence and what to do. @param {string} error @param {string} remedy */
export const refusal = (error, remedy) => ({ error, remedy });

/** One JSON call. Rejects with `{ error, remedy }` (the server's own words when it sent them). @param {string} path @param {unknown} [body] */
export async function api(path, body) {
    let res;
    try {
        res = await fetch(path, {
            method: body === undefined ? 'GET' : 'POST',
            headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
            body: body === undefined ? undefined : JSON.stringify(body),
            credentials: 'same-origin',
        });
    } catch (cause) {
        console.warn(`calendars: ${path} got no answer`, cause);
        throw refusal('Almanac did not answer.', 'Check that the service is running, then try again.');
    }
    /** @type {any} */
    let json = null;
    try {
        json = await res.json();
    } catch (cause) {
        // An HTML answer is the login page: the session ended. Handled below, and noted here for the console.
        console.warn(`calendars: ${path} did not answer JSON (status ${res.status})`, cause);
    }
    if (!res.ok || json === null) {
        const ended = json === null && (res.status === 401 || /login/.test(res.url));
        throw refusal(
            json?.error ?? (ended ? 'Your session ended.' : json === null ? 'Almanac did not answer.' : 'Almanac could not do that.'),
            json?.remedy ?? (ended ? 'Reload the page and log in again.' : 'Try again in a moment.'),
        );
    }
    return json;
}

/** The toast of one flavour (success, info, warning, error, destructive). @param {string} name @param {string} words */
export const flavour = (name, words) =>
    toast(words, { className: `kp-toast--${name}`, ...(name === 'error' || name === 'destructive' ? { live: /** @type {const} */ ('assertive') } : {}) });

/** A refusal as one sentence for a toast. @param {{ error: string, remedy?: string }} e */
export const said = (e) => `${e.error.replace(/\.$/, '')}. ${e.remedy ?? ''}`.trim();

/** Shows a rejected action as an error toast, for a promise nobody else catches. @param {unknown} e */
export const reportFailure = (e) => {
    console.error('calendars: an action failed', e);
    const refused = /** @type {any} */ (e);
    flavour('error', typeof refused?.error === 'string' ? said(refused) : 'Something went wrong. Reload the page and try again.');
};
