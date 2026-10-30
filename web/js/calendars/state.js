// What the Calendars page knows (the two answers, the active filter, the view) and the values derived from them.
// `hooks` lets the renderers ask for a redraw or a reload without importing the loader, which imports them.
import { moment, plural } from '../lib/format.js';

/** @typedef {{ id: string, name: string }} Calendar */
/** @typedef {{ id: string, calendar_id: string, delivered: number, last_at_ms: number | null, last_failed: boolean }} Source */

/**
 * `google` and `profiles` are null while being read. `filterKey` is the count that filters the rows now, or null.
 * @type {{
 *   google: { ok: boolean, can_create: boolean, calendars: Calendar[], error?: string, remedy?: string } | null,
 *   profiles: { ok: boolean, folder_ok: boolean, served: any[], unserved: any[], sources: Source[], trend: any, error?: string } | null,
 *   filterKey: string | null,
 *   view: string,
 * }}
 */
export const state = { google: null, profiles: null, filterKey: null, view: 'lines' };

/** Filled in by load.js once it has loaded; renderers call these instead of importing it (that would be a cycle). */
export const hooks = {
    render: () => {},
    /** @param {boolean} [quiet] */
    load: (quiet) => /** @type {Promise<unknown>} */ (Promise.resolve(quiet)),
    /** @param {boolean} [quiet] */
    loadGoogle: (quiet) => /** @type {Promise<void>} */ (Promise.resolve(quiet).then(() => undefined)),
    /** @param {boolean} [quiet] */
    loadProfiles: (quiet) => /** @type {Promise<void>} */ (Promise.resolve(quiet).then(() => undefined)),
};

export const sources = () => state.profiles?.sources ?? [];
/** @param {string} calendarId */
export const sourcesOf = (calendarId) => sources().filter((s) => s.calendar_id === calendarId);
/** @param {string} calendarId */
export function calName(calendarId) {
    if (!state.google) return '…';
    return state.google.calendars.find((c) => c.id === calendarId)?.name ?? 'A calendar Almanac cannot see';
}

/** The words a hover card says about a source. @param {Source} s */
export function sourceCardText(s) {
    const failed = s.last_failed ? ' The latest delivery failed.' : '';
    if (!s.delivered) return `${s.id}: nothing delivered since Almanac started.${failed}`;
    const last = s.last_at_ms ? ` Last delivery ${moment(s.last_at_ms)}.` : '';
    return `${s.id}: ${plural(s.delivered, 'event', 'events')} delivered recently.${last}${failed}`;
}
