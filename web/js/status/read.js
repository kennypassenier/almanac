// The status page's reads: a JSON fetch whose failure is a sentence a person can be told, and the wait for a kit table.
import { dataTable } from '/static/kp/dist/js/index.js';

/** How often, and how many times, to look for a data table the kit's script has not attached yet (together: 5 s). */
const TABLE_POLL_MS = 50;
const TABLE_POLL_TRIES = 100;

/** A read that failed in a way a person can be told about. */
export class ReadError extends Error {}

/** @param {string} url @param {RequestInit} [init] */
export async function readJson(url, init) {
    let response;
    try {
        response = await fetch(url, { credentials: 'same-origin', headers: { Accept: 'application/json' }, ...init });
    } catch (cause) {
        throw new ReadError('The Almanac service did not answer. Nothing was lost.', { cause });
    }
    const json = /json/.test(response.headers.get('content-type') ?? '') ? await response.json().catch(() => null) : null;
    if (!json) throw new ReadError('Your sign-in has ended. Reload the page to sign in again.');
    if (!response.ok) throw new ReadError(typeof json.error === 'string' ? [json.error, json.remedy].filter(Boolean).join(' ') : 'Almanac could not answer.');
    return json;
}

/** The handle of a data table once the kit's script has attached it. @param {HTMLElement} host */
export async function tableHandle(host) {
    for (let i = 0; i < TABLE_POLL_TRIES; i++) {
        const handle = dataTable(host);
        if (handle) return handle;
        await new Promise((resolve) => setTimeout(resolve, TABLE_POLL_MS));
    }
    console.warn('status: the deliveries table was never attached by the kit');
    return null;
}
