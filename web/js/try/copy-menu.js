// One "Copy" menu per card on the Try page instead of a Copy button per line: each card registers what its entries copy.
import { toast } from '/static/kp/dist/js/index.js';

/** The text each entry copies, read when it is picked (the card's content changes after every Try). @type {Map<string, () => string>} */
const sources = new Map();

/** Says what can be copied right now: an entry whose text is empty is disabled. @param {HTMLElement} menu */
function refresh(menu) {
    for (const item of /** @type {NodeListOf<HTMLButtonElement>} */ (menu.querySelectorAll('[data-kp-value]'))) {
        item.disabled = !(sources.get(item.dataset.kpValue ?? '')?.() ?? '');
    }
}

/** @param {string} text @param {string} what */
async function copy(text, what) {
    try {
        await navigator.clipboard.writeText(text);
        toast(`Copied ${what}.`);
    } catch (error) {
        console.warn('try: the clipboard refused the copy', error);
        toast(`${what[0].toUpperCase()}${what.slice(1)} could not be copied: the browser refused it.`, { className: 'kp-toast--error' });
    }
}

/** Wires a card's Copy menu; `entries` maps an entry's data-kp-value to the text it copies. @param {HTMLElement} wrap @param {Record<string, () => string>} entries */
export function initCopyMenu(wrap, entries) {
    for (const [key, read] of Object.entries(entries)) sources.set(key, read);
    wrap.addEventListener('kp-menu-open', () => refresh(wrap));
    wrap.addEventListener('kp-menu-select', (event) => {
        const { item, value } = /** @type {CustomEvent} */ (event).detail;
        const text = sources.get(value)?.() ?? '';
        if (text) void copy(text, String(item?.textContent ?? 'it').trim().toLowerCase());
    });
}
