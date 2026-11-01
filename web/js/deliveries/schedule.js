// Section 2 of the deliveries page: the retry schedule, as runs or as a timeline (gantt), and the person's choice of shape.
import { setGanttKinds, setGanttPoints, setGanttState, setGanttWindow, setGanttZones, setRuns, setRunsState, setSegmentedValues } from '/static/kp/dist/kp-themes.min.js';
import { moment } from '../lib/format.js';
import { state, SECOND, MINUTE } from './state.js';
import { runsEl, ganttEl, schedSaid, shapeEl } from './dom.js';
import { events, tries, nothingWaiting } from './words.js';
import { LIT } from './chain.js';

/** Space left on both sides of the timeline's data, so the first and last points are not on its edge. */
const GANTT_PADDING_MS = 6 * MINUTE;

const KINDS = [
    { kind: 'received', label: 'received', shape: 'square', tone: 'muted' },
    { kind: 'refused', label: 'failed try', shape: 'dot', tone: 'destructive' },
    { kind: 'next', label: 'next try', shape: 'ring', tone: 'info' },
    { kind: 'aside', label: 'third refusal: set aside', shape: 'triangle', tone: 'destructive' },
];

// The shape of the schedule a person chose stays.
const SHAPE_KEY = 'almanac-deliveries:shape';

/** @returns {'runs' | 'gantt'} */
export function storedShape() {
    try {
        return localStorage.getItem(SHAPE_KEY) === 'gantt' ? 'gantt' : 'runs';
    } catch (error) {
        console.warn('deliveries: the remembered shape could not be read', error);
        return 'runs';
    }
}

/** The conflict words of each coming pass: the entries whose last strike falls on it. @param {any} schedule */
function conflicts(schedule) {
    const byPass = new Map();
    for (const entry of schedule.entries) {
        if (!entry.limited) continue;
        const pass = entry.left - 1;
        byPass.set(pass, [...(byPass.get(pass) ?? []), entry]);
    }
    return byPass;
}

/** @param {any} schedule */
function runsData(schedule) {
    const byPass = conflicts(schedule);
    return schedule.next.map((/** @type {number} */ at, /** @type {number} */ i) => {
        const hit = byPass.get(i);
        if (!hit) return { at };
        const text =
            hit.length === 1
                ? `Third refusal for the ${hit[0].source} entry "${hit[0].title}": it is set aside`
                : `Third refusal for ${hit.length} entries: they are set aside`;
        return { at, conflict: text };
    });
}

/** The planned points of one entry: its next tries, the last of a limited entry being the one that sets it aside. @param {any} entry @param {any} schedule */
function plannedPoints(entry, schedule) {
    const points = [];
    for (const [i, at] of schedule.next.entries()) {
        if (entry.limited && i > entry.left - 1) break;
        const last = entry.limited && i === entry.left - 1;
        points.push({
            id: `${entry.id}-n${i}`,
            lane: entry.id,
            at,
            kind: last ? 'aside' : 'next',
            label: last ? `${entry.title}: third refusal, set aside` : `${entry.title}: next try`,
            detail: `${moment(at)}${last ? '. After this Almanac stops trying and keeps the entry in the journal.' : ''}`,
            planned: true,
        });
    }
    return points;
}

/** The received point and the failed tries of one entry; `limit` is the server's strikes limit. @param {any} entry @param {number} limit */
function pastPoints(entry, limit) {
    const points = [{ id: `${entry.id}-in`, lane: entry.id, at: entry.received, kind: 'received', label: `${entry.title}: received ${moment(entry.received)}` }];
    const firstStrike = entry.tries.length - entry.strikes;
    entry.tries.forEach((/** @type {number} */ at, /** @type {number} */ i) =>
        points.push({
            id: `${entry.id}-t${at}`,
            lane: entry.id,
            at,
            kind: 'refused',
            label: `${entry.title}: try ${i + 1} failed`,
            detail: entry.limited && i >= firstStrike ? `Permanent refusal ${i - firstStrike + 1} of ${limit}.` : 'Almanac keeps trying.',
        }),
    );
    return points;
}

/** @param {any} schedule */
function ganttData(schedule) {
    const lanes = schedule.entries.map((/** @type {any} */ entry) => ({
        id: entry.id,
        label: entry.title,
        sub: entry.limited ? `${entry.source} · ${tries(entry.left)} left` : `${entry.source} · no limit`,
    }));
    const points = schedule.entries.flatMap((/** @type {any} */ entry) => [...pastPoints(entry, schedule.strikes_limit), ...plannedPoints(entry, schedule)]);
    return { points, lanes };
}

function renderRuns() {
    if (state.status === 'loading') return setRunsState(runsEl, 'loading');
    if (state.status === 'error') return setRunsState(runsEl, 'error', { title: 'Could not read the retry schedule', text: 'The journal did not answer.' });
    const { schedule } = state.data;
    if (!schedule.entries.length) return setRuns(runsEl, [], { reason: nothingWaiting(state.data.counts, state.data.aside.length) });
    setRuns(runsEl, runsData(schedule));
}

function renderGantt() {
    if (state.status === 'loading') return setGanttState(ganttEl, 'loading');
    if (state.status === 'error') return setGanttState(ganttEl, 'error', 'Could not read the retry schedule: the journal did not answer.');
    const { schedule, chain: stages, now } = state.data;
    setGanttKinds(ganttEl, KINDS);
    if (!schedule.entries.length) {
        setGanttPoints(ganttEl, [], []);
        return setGanttState(ganttEl, 'empty', nothingWaiting(state.data.counts, state.data.aside.length));
    }
    const { points, lanes } = ganttData(schedule);
    const lit = stages.lit ? /** @type {any} */ (LIT)[stages.lit.reason] : null;
    setGanttZones(
        ganttEl,
        schedule.outage_since
            ? [{ from: schedule.outage_since, to: now, label: `${lit ? lit.name : 'Delivery'} failing since ${moment(schedule.outage_since).slice(11)}`, tone: 'warning' }]
            : [],
    );
    setGanttPoints(ganttEl, points, lanes);
    setGanttState(ganttEl, 'ready');
    const start = Math.min(...points.map((/** @type {{ at: number }} */ p) => p.at), schedule.outage_since ?? Infinity) - GANTT_PADDING_MS;
    const end = Math.max(...schedule.next, now) + GANTT_PADDING_MS;
    setGanttWindow(ganttEl, { span: Math.ceil((end - start) / SECOND), live: false, to: end });
}

/** The one line under the schedule. */
function saySchedule() {
    if (state.status !== 'ready') {
        schedSaid.textContent = state.status === 'loading' ? 'Reading the journal.' : 'The journal could not be read.';
        return;
    }
    const { schedule, counts } = state.data;
    schedSaid.textContent = schedule.entries.length
        ? `${events(schedule.entries.length)} waiting${schedule.outage_since ? `; deliveries have been failing since ${moment(schedule.outage_since)}` : ''}. All waiting events are tried together.`
        : `Nothing is waiting (${counts.pending} in the journal).`;
}

export function renderSchedule() {
    runsEl.hidden = state.shape !== 'runs';
    ganttEl.hidden = state.shape !== 'gantt';
    saySchedule();
    if (state.shape === 'runs') renderRuns();
    else renderGantt();
}

/** @param {() => void} onRetry reads the journal again */
export function initSchedule(onRetry) {
    state.shape = storedShape();
    setSegmentedValues(shapeEl, [state.shape]);
    runsEl.addEventListener('kp-runs-retry', onRetry);
    shapeEl.addEventListener('kp-segmented-change', (event) => {
        const value = /** @type {CustomEvent} */ (event).detail?.value;
        state.shape = value === 'gantt' ? 'gantt' : 'runs';
        try {
            localStorage.setItem(SHAPE_KEY, state.shape);
        } catch (error) {
            // Storage is off: the shape just does not outlive the page.
            console.warn('deliveries: the chosen shape could not be remembered', error);
        }
        renderSchedule();
    });
}
