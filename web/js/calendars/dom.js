// The Calendars page's elements and its two data tables. Importing this waits for the package to attach its parts.
import { dataTable } from '/static/kp/js/datatable.js';
import { setMatrix } from '/static/kp/js/heat.js';
import { autoReady } from '../lib/ready.js';

export const $ = (/** @type {string} */ s, /** @type {ParentNode} */ r = document) => /** @type {HTMLElement | null} */ (r.querySelector(s));
export const must = (/** @type {string} */ s, /** @type {ParentNode} */ r = document) => /** @type {HTMLElement} */ ($(s, r));

/** Skeleton rows shown in the uses map while it loads. */
export const USES_SKELETON_ROWS = 3;

export const page = must('[data-cal-page]');
export const strip = must('[data-cal-kpis]');
export const line = must('[data-cal-filter-line]');
export const band = must('[data-cal-attention]');
export const tableWrap = must('[data-cal-table]');
export const tbody = /** @type {HTMLElement} */ (tableWrap.querySelector('tbody'));
export const profileCard = must('[data-cal-profiles]');
export const problemsBox = must('[data-cal-problems]', profileCard);
export const allServed = must('[data-cal-allserved]', profileCard);
export const servedWrap = must('[data-cal-served]', profileCard);
export const servedBody = /** @type {HTMLElement} */ (servedWrap.querySelector('tbody'));
export const usesCard = must('[data-cal-section="uses"]');
export const lines = must('[data-cal-lines]', usesCard);
export const graph = must('[data-cal-graph]', usesCard);
export const switcher = must('[data-cal-view-switch]', usesCard);
export const menuWrap = must('[data-cal-more]');

// The uses map is drawn by script, so its loading state is the first thing this module does.
setMatrix(lines, { view: 'lines', rows: [], columns: [], state: 'loading', skeleton: USES_SKELETON_ROWS });

await autoReady;

export const table = /** @type {import('/static/kp/js/datatable.js').DataTableHandle} */ (dataTable(tableWrap));
export const servedTable = dataTable(servedWrap);
