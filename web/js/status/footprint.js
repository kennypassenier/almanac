// A workaround for an open package gap: a failed or empty deliveries table keeps the height it had with its rows.
import { $ } from './card.js';

/* A failed or empty table keeps the height it had with its rows, so the page under it does not jump (kp-themes 11.2.0 still lets a
   driven empty or failed table fall short of its loaded height: kp-gaps almanac 32). */
let tableHeight = 0;
export function holdTableFootprint() {
    const root = /** @type {HTMLElement} */ ($('#st-table'));
    new ResizeObserver(() => {
        const st = root.getAttribute('data-kp-state');
        if (st && st !== 'ready') return;
        tableHeight = Math.max(tableHeight, root.getBoundingClientRect().height);
    }).observe(root);
    const settle = () => {
        const empty = $('[data-kp-datatable-empty]', root);
        const held = ['failed'].includes(root.getAttribute('data-kp-state') ?? '') || (empty !== null && !empty.hasAttribute('hidden'));
        if (held && tableHeight) root.style.setProperty('min-block-size', `min(${tableHeight}px, var(--kp-footprint-max, 48rem))`);
        else root.style.removeProperty('min-block-size');
    };
    new MutationObserver(settle).observe(root, { attributes: true, attributeFilter: ['data-kp-state'] });
    const slot = $('[data-kp-datatable-empty]', root);
    if (slot) new MutationObserver(settle).observe(slot, { attributes: true, attributeFilter: ['hidden'] });
}
