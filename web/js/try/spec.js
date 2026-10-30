// Pure helpers that turn a dry-run answer into what the result card shows: the calendar spec rows, a line diff of two
// documents, and the curl command that repeats the call.

const MINUTES_PER_HOUR = 60;
const MINUTES_PER_DAY = 24 * MINUTES_PER_HOUR;
const DAY_MS = MINUTES_PER_DAY * 60 * 1000;

/** Google Calendar's event colour ids and their names, as the Google Calendar app words them. */
const PALETTE = { 1: 'lavender', 2: 'sage', 3: 'grape', 4: 'flamingo', 5: 'banana', 6: 'tangerine', 7: 'peacock', 8: 'graphite', 9: 'blueberry', 10: 'basil', 11: 'tomato' };

/** @param {string} date `yyyy-mm-dd` @param {number} days */
const addDays = (date, days) => {
    const [y, m, d] = date.split('-').map(Number);
    return new Date(Date.UTC(y, m - 1, d + days)).toISOString().slice(0, 10);
};

/** A start or end as dd/mm/yyyy HH:MM (24 h) in its own zone, or dd/mm/yyyy for an all-day date. @param {any} point */
function stamp(point) {
    if (point.date) return point.date.split('-').reverse().join('/');
    const parts = Object.fromEntries(
        new Intl.DateTimeFormat('en-GB', { timeZone: point.timeZone, year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' })
            .formatToParts(new Date(point.dateTime))
            .map((p) => [p.type, p.value]),
    );
    return `${parts.day}/${parts.month}/${parts.year} ${parts.hour}:${parts.minute}`;
}

/** @param {any} event */
function whenWords(event) {
    if (!event.start.date) return `${stamp(event.start)} to ${stamp(event.end).slice(11)} (${event.start.timeZone})`;
    const last = addDays(event.end.date, -1);
    const days = (Date.parse(event.end.date) - Date.parse(event.start.date)) / DAY_MS;
    return `${stamp(event.start)}${days > 1 ? ` to ${stamp({ date: last })}` : ''} (all day, ${days} ${days === 1 ? 'day' : 'days'})`;
}

/** @param {number} m minutes */
const reminderLength = (m) => (m % MINUTES_PER_DAY === 0 ? `${m / MINUTES_PER_DAY} ${m === MINUTES_PER_DAY ? 'day' : 'days'}` : m % MINUTES_PER_HOUR === 0 ? `${m / MINUTES_PER_HOUR} h` : `${m} min`);

/** @param {any} reminders */
function reminderWords(reminders) {
    if (!reminders) return "The calendar's defaults";
    const o = reminders.overrides ?? [];
    return o.length ? o.map((/** @type {any} */ r) => `${r.method} ${reminderLength(r.minutes)} before`).join(', ') : 'None (silent)';
}

/** The calendar spec of an event as label/value rows, in the words a person reads (dd/mm/yyyy HH:MM, 24 h). @param {any} event @param {string} calendarName @returns {string[][]} */
export function specRows(event, calendarName) {
    const rows = [['Title', event.summary], ['When', whenWords(event)], ['Calendar', calendarName]];
    if (event.location) rows.push(['Location', event.location]);
    if (event.description) rows.push(['Description', event.description]);
    rows.push(['Shows as', event.transparency === 'transparent' ? 'Free (does not block your time)' : 'Busy']);
    if (event.colorId) rows.push(['Colour', `${/** @type {any} */ (PALETTE)[event.colorId] ?? event.colorId} (id ${event.colorId})`]);
    if (event.status) rows.push(['Status', event.status]);
    rows.push(['Reminders', reminderWords(event.reminders)]);
    if (event.extendedProperties) rows.push(['Marker', event.extendedProperties.private.almanac_source_id]);
    return rows;
}

/** A line diff of two JSON documents (LCS), as kp-diff rows: `{ kind, oldNo, newNo, text }`. @param {any} before @param {any} after */
export function diffLines(before, after) {
    const a = before == null ? [] : JSON.stringify(before, null, 2).split('\n');
    const b = JSON.stringify(after, null, 2).split('\n');
    const n = a.length;
    const m = b.length;
    const table = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
    for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) table[i][j] = a[i] === b[j] ? table[i + 1][j + 1] + 1 : Math.max(table[i + 1][j], table[i][j + 1]);
    /** @type {{ kind: string, oldNo: number | null, newNo: number | null, text: string }[]} */
    const rows = [];
    let i = 0;
    let j = 0;
    while (i < n && j < m) {
        if (a[i] === b[j]) rows.push({ kind: 'same', oldNo: ++i, newNo: ++j, text: b[j - 1] });
        else if (table[i + 1][j] >= table[i][j + 1]) rows.push({ kind: 'removed', oldNo: ++i, newNo: null, text: a[i - 1] });
        else rows.push({ kind: 'added', oldNo: null, newNo: ++j, text: b[j - 1] });
    }
    while (i < n) rows.push({ kind: 'removed', oldNo: ++i, newNo: null, text: a[i - 1] });
    while (j < m) rows.push({ kind: 'added', oldNo: null, newNo: ++j, text: b[j - 1] });
    return rows;
}

/** @param {string} url @param {unknown} payload */
export function curlFor(url, payload) {
    const body = JSON.stringify(payload).replaceAll("'", "'\\''");
    return [`curl -sS -X POST "${url}" \\`, '  -H "Authorization: Bearer $ALMANAC_TOKEN" \\', '  -H "Content-Type: application/json" \\', `  -d '${body}'`].join('\n');
}
