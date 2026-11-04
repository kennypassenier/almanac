// Shared by the page scripts: the kit's `window.chassisPageData` (chassis-rs 4.8.0), one read of a page's named parts
// where a failing part is that part's `{ ok: false, error, remedy }`. chassis.js installs it; a page script can start
// first, so each call waits for the kit's ready.

/** @typedef {{ ok: true, data: any } | { ok: false, status: number, error: string, remedy: string }} Part */

const win = /** @type {any} */ (window);
const kitReady = () =>
    win.chassisPageData
        ? Promise.resolve()
        : (win.chassis?.ready ?? new Promise((resolve) => document.addEventListener('chassis:ready', resolve, { once: true })));

/** Every named part in one read. @param {string} url @param {string[]} names @returns {Promise<Record<string, Part>>} */
export async function readParts(url, names) {
    await kitReady();
    return win.chassisPageData(url, names);
}

/** One part alone, for a retry or a refresh of one section. @param {string} url @param {string} name @returns {Promise<Part>} */
export async function readPart(url, name) {
    await kitReady();
    return win.chassisPageData.one(url, name);
}

/** A failed part as one sentence a person can be told. @param {Part} part */
export const partWords = (part) => (part.ok ? '' : [part.error, part.remedy].filter(Boolean).join(' '));
