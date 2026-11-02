// The deliveries page's wording: counts with their nouns, and the two-letter mark of a source.
export const times = (/** @type {number} */ n) => `${n} ${n === 1 ? 'time' : 'times'}`;
export const tries = (/** @type {number} */ n) => `${n} ${n === 1 ? 'try' : 'tries'}`;
export const events = (/** @type {number} */ n) => `${n} ${n === 1 ? 'event' : 'events'}`;

/** A source's mark: the one its profile sets (`marks`, from the server), else the first two letters of its id. */
export const markOf = (/** @type {string} */ source, /** @type {Record<string, string>} */ marks = {}) =>
    marks[source] ?? source.replace(/[^a-z0-9]/gi, '').slice(0, 2).toUpperCase();

/**
 * What the empty schedule says. It depends on what was delivered and what was set aside, not only on what waits:
 * with every event set aside, "every event has reached its calendar" would be false.
 * @param {{ delivered: number }} counts @param {number} setAside
 */
export function nothingWaiting(counts, setAside) {
    const aside = `${events(setAside)} ${setAside === 1 ? 'is' : 'are'} set aside (see below)`;
    if (setAside > 0 && counts.delivered === 0) return `Nothing is waiting, but nothing has reached the calendar either: ${aside}.`;
    if (setAside > 0) return `Nothing is waiting. ${counts.delivered} delivered; ${aside}.`;
    if (counts.delivered > 0) return 'Nothing is waiting. Every event Almanac accepted has reached its calendar.';
    return 'Nothing is waiting and nothing has been delivered yet.';
}

/** How long a set-aside event still has before it is removed. */
export const daysLeft = (/** @type {number} */ n) => (n === 0 ? 'Removed today' : `${n} ${n === 1 ? 'day' : 'days'} left`);
