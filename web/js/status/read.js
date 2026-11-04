// The status page's reads: the error a failed part becomes, a sentence a person can be told, and the wait for a kit table.
import { dataTable } from '/static/kp/dist/kp-themes.min.js';

/** How often, and how many times, to look for a data table the kit's script has not attached yet (together: 5 s). */
const TABLE_POLL_MS = 50;
const TABLE_POLL_TRIES = 100;

/** A read that failed in a way a person can be told about. */
export class ReadError extends Error {}

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
