// The kit's section cards around the status page's stages, and how this module holds their state.
import { setCardState } from '/static/kp/js/card-state.js';

export const $ = (/** @type {string} */ selector, /** @type {ParentNode} */ root = document) => /** @type {HTMLElement | null} */ (root.querySelector(selector));

/** The kit's section card around a section's stage. @param {string} marker */
export const cardOf = (marker) => /** @type {HTMLElement | null} */ ($(`[${marker}]`)?.closest('.kp-card[data-chassis-section]') ?? null);
export const journalCard = () => cardOf('data-st-journal');
export const sourcesCard = () => cardOf('data-st-sources');

/**
 * The kit's section card is the whole-section state (loading, empty, failed). `data-chassis-owned` (chassis-rs 3.10) keeps the
 * kit from settling it back to ready after each of its own reads, so the state this module sets is the one that stays.
 * @param {HTMLElement} card
 */
export function hold(card) {
    card.setAttribute('data-chassis-owned', '');
    return {
        /** @param {string} state @param {object} [note] */
        set(state, note) {
            if (card.getAttribute('data-kp-state') !== state) setCardState(card, /** @type {any} */ (state), note);
        },
    };
}
