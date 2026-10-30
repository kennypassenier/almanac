// The deliveries page's wording: counts with their nouns, and the two-letter mark of a source.
export const times = (/** @type {number} */ n) => `${n} ${n === 1 ? 'time' : 'times'}`;
export const tries = (/** @type {number} */ n) => `${n} ${n === 1 ? 'try' : 'tries'}`;
export const events = (/** @type {number} */ n) => `${n} ${n === 1 ? 'event' : 'events'}`;

/** The marks of the sources the household knows; any other source gets the first two letters of its id. */
const MARKS = { 'home-assistant': 'HA', homelab: 'HL', 'claude-sessions': 'CS', kyu: 'KY' };
export const markOf = (/** @type {string} */ source) => /** @type {Record<string, string>} */ (MARKS)[source] ?? source.replace(/[^a-z0-9]/gi, '').slice(0, 2).toUpperCase();
