// Words and escaping shared by the page scripts: HTML escaping, plurals, and the one date format Almanac shows
// (dd/mm/yyyy HH:MM, 24 h, in the household's time zone).

/** The household's time zone: every time the pages show is in it, whatever the browser's own zone is. */
export const TIME_ZONE = 'Europe/Brussels';

const STAMP = new Intl.DateTimeFormat('en-GB', {
    timeZone: TIME_ZONE,
    day: '2-digit',
    month: '2-digit',
    year: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
});

/** Text made safe for an HTML string or attribute. @param {unknown} text */
export const esc = (text) => String(text).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c] ?? c);

/** dd/mm/yyyy HH:MM, 24 h, Europe/Brussels. @param {number} ms */
export function moment(ms) {
    const p = Object.fromEntries(STAMP.formatToParts(ms).map((part) => [part.type, part.value]));
    return `${p.day}/${p.month}/${p.year} ${p.hour}:${p.minute}`;
}

/** `3 events`, `1 event`. @param {number} n @param {string} one @param {string} many */
export const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;
