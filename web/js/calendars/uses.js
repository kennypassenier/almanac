// Section 7 of the Calendars page: the uses map, as a matrix ("lines") or a graph.
import { setMatrix } from '/static/kp/js/heat.js';
import { setGraphData, setGraphState } from '/static/kp/js/graph.js';
import { setSegmentedValues } from '/static/kp/js/segmented.js';
import { plural } from '../lib/format.js';
import { state, hooks, sources, sourcesOf, sourceCardText } from './state.js';
import { lines, graph, switcher, USES_SKELETON_ROWS } from './dom.js';

/** Shows a non-ready state on both views. @param {'error' | 'empty'} kind @param {string} message */
function usesMessage(kind, message) {
    setMatrix(lines, { view: 'lines', rows: [], columns: [], state: kind, message });
    setGraphState(graph, kind, message);
}

/** @param {import('./state.js').Calendar[]} calendars @param {import('./state.js').Source[]} writing */
function drawMatrix(calendars, writing) {
    setMatrix(lines, {
        corner: 'Calendar ↓ source →',
        rows: calendars.map((c) => ({ key: c.id, label: c.name })),
        columns: writing.map((s) => ({ key: s.id, label: s.id })),
        label: 'Which source writes to which calendar',
        cell: (row, column) => (writing.find((s) => s.id === column)?.calendar_id === row ? { tone: 'ok', label: 'writes here' } : { tone: 'none' }),
        countLabel: (n) => plural(n, 'source', 'sources'),
        sourcesLabel: 'Calendars',
        usersLabel: 'Sources',
        view: 'lines',
    });
}

/** @param {import('./state.js').Calendar[]} calendars @param {import('./state.js').Source[]} writing */
function drawGraph(calendars, writing) {
    const hub = [...calendars].sort((a, b) => sourcesOf(b.id).length - sourcesOf(a.id).length)[0];
    const draw = () =>
        setGraphData(graph, {
            hub: hub?.id,
            nodes: [
                ...calendars.map((c) => ({ id: c.id, label: c.name, kind: 'calendar', description: `Written by ${sourcesOf(c.id).map((s) => s.id).join(', ') || 'no source'}` })),
                ...writing.map((s) => ({ id: s.id, label: s.id, kind: 'source', description: sourceCardText(s) })),
            ],
            edges: writing.map((s) => ({ from: s.id, to: s.calendar_id, kind: 'writes', detail: `${plural(s.delivered, 'event', 'events')} delivered recently` })),
            kinds: [{ kind: 'writes', label: 'Writes to', hint: 'A source posts its events into this calendar', style: 'solid', colour: 'var(--chart-1)' }],
        });
    // A graph drawn while hidden has no size: it is drawn when it can be measured.
    if (state.view === 'graph') requestAnimationFrame(draw);
    else draw();
}

export function renderUses() {
    const { google, profiles } = state;
    if (!google || !profiles) {
        setMatrix(lines, { view: 'lines', rows: [], columns: [], state: 'loading', skeleton: USES_SKELETON_ROWS });
        setGraphState(graph, 'loading', 'Reading the set-up');
        return;
    }
    const ids = new Set(google.calendars.map((c) => c.id));
    const writing = sources().filter((s) => ids.has(s.calendar_id));
    if (!google.ok || !profiles.ok) return usesMessage('error', 'The set-up could not be read');
    if (!writing.length) return usesMessage('empty', 'No source writes to a calendar yet');
    drawMatrix(google.calendars, writing);
    drawGraph(google.calendars, writing);
}

/** @param {string} next */
export function showView(next) {
    state.view = next;
    lines.hidden = next !== 'lines';
    graph.hidden = next !== 'graph';
    setSegmentedValues(switcher, [next]);
    if (next === 'graph') renderUses();
}

export function initUses() {
    switcher.addEventListener('kp-segmented-change', (event) => showView(String(/** @type {CustomEvent} */ (event).detail.value)));
    lines.addEventListener('kp-uses-retry', () => void hooks.load());
}
