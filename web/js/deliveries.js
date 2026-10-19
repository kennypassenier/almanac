// deliveries page: kp-themes 11.0.0 components wired to Almanac's /deliveries/data route.
// The route answers all three sections from the live state; this file words it (times in
// Europe/Brussels, dd/mm/yyyy HH:MM, 24 h) and hands it to the package's components.

import { attachGantts, setGanttKinds, setGanttPoints, setGanttState, setGanttWindow, setGanttZones } from '/static/kp/js/gantt.js';
import { attachPipelines, setPipeline, setPipelineState } from '/static/kp/js/pipeline.js';
import { attachProblems, setProblems } from '/static/kp/js/problems.js';
import { attachArt } from '/static/kp/js/art.js';
import { statePlate } from '/static/kp/js/state-plate.js';
import { openBusyCard } from '/static/kp/js/busy-card.js';
import { attachRuns, setRuns, setRunsState } from '/static/kp/js/runs.js';
import { setSegmentedValues } from '/static/kp/js/segmented.js';

/** Resolves once js/auto.js has attached everything the page carries. */
const ready = new Promise((resolve) => {
    const done = () => document.documentElement.hasAttribute('data-kp-auto-ready');
    if (done()) return resolve(undefined);
    new MutationObserver((_, watch) => {
        if (done()) {
            watch.disconnect();
            resolve(undefined);
        }
    }).observe(document.documentElement, { attributes: true, attributeFilter: ['data-kp-auto-ready'] });
});
await ready;

const SECOND = 1000;
const MINUTE = 60 * SECOND;
const POLL_MS = 5 * SECOND;
const STRIKES_LIMIT = 3;

/** dd/mm/yyyy HH:MM, 24 h, Europe/Brussels. @param {number} ms */
function moment(ms) {
    const p = Object.fromEntries(
        new Intl.DateTimeFormat('en-GB', {
            timeZone: 'Europe/Brussels',
            day: '2-digit',
            month: '2-digit',
            year: 'numeric',
            hour: '2-digit',
            minute: '2-digit',
            hourCycle: 'h23',
        })
            .formatToParts(ms)
            .map((part) => [part.type, part.value]),
    );
    return `${p.day}/${p.month}/${p.year} ${p.hour}:${p.minute}`;
}
const clock = (/** @type {number} */ ms) => moment(ms).slice(11);
const times = (/** @type {number} */ n) => `${n} ${n === 1 ? 'time' : 'times'}`;
const tries = (/** @type {number} */ n) => `${n} ${n === 1 ? 'try' : 'tries'}`;
const events = (/** @type {number} */ n) => `${n} ${n === 1 ? 'event' : 'events'}`;
const MARKS = { 'home-assistant': 'HA', homelab: 'HL', 'claude-sessions': 'CS', kyu: 'KY' };
const markOf = (/** @type {string} */ source) => MARKS[source] ?? source.replace(/[^a-z0-9]/gi, '').slice(0, 2).toUpperCase();

// The shape of the schedule a person chose stays.
const SHAPE_KEY = 'almanac-deliveries:shape';
const storedShape = () => {
    try {
        return localStorage.getItem(SHAPE_KEY) === 'gantt' ? 'gantt' : 'runs';
    } catch {
        return 'runs';
    }
};

const $ = (/** @type {string} */ id) => /** @type {HTMLElement} */ (document.getElementById(id));
const chain = $('ad-chain');
const chainSaid = $('ad-chain-said');
const runsEl = $('ad-runs');
const ganttEl = $('ad-gantt');
const schedSaid = $('ad-sched-said');
const asideEl = $('ad-aside');
const asideLoading = $('ad-aside-loading');
const asideEmpty = $('ad-aside-empty');
const asideFailed = $('ad-aside-failed');
const shapeEl = $('ad-shape');

attachPipelines(chain.parentElement ?? document);
attachRuns(runsEl.parentElement ?? document);
attachGantts(ganttEl.parentElement ?? document);
attachProblems(asideEl.parentElement ?? document);

let shape = storedShape();
setSegmentedValues(shapeEl, [shape]);

/** The last answer, or null before the first. @type {any} */
let data = null;
/** 'loading' until the first answer, then 'ready' or 'error'. */
let status = 'loading';

/* ------------------------------------------------------------------ 1 · delivery chain */

const LANES = [
    { key: 'received', label: 'Received', tone: 1 },
    { key: 'journal', label: 'Journal', tone: 2 },
    { key: 'worker', label: 'Worker', tone: 3 },
    { key: 'google', label: 'Google Calendar', tone: 4 },
    { key: 'dead', label: 'Set aside · never retried', closed: true },
];
const LIT = {
    temporary: { stage: 'google', label: 'Google · refusing', said: 'Google is not accepting the events right now. The waiting ones are tried again.' },
    refused: { stage: 'google', label: 'Google · refusing', said: 'Google refuses at least one event for good. It is set aside after the third refusal; the others are tried again.' },
    token: { stage: 'worker', label: 'Worker · no token', said: 'The worker cannot get a Google token, so no call to Google is made. The events wait in the journal.' },
    local: { stage: 'worker', label: 'Worker · cannot build event', said: 'Almanac cannot turn a waiting event into a calendar entry. The events wait in the journal.' },
};

/** The lit lane's key from the chain, none when null. @type {string | null} */
let litKey = null;
/** Lights the lane `key` (red), none when null. @param {string | null} key */
function light(key) {
    litKey = key;
    for (const lane of chain.querySelectorAll('[data-kp-lane]')) {
        lane.toggleAttribute('data-ad-lit', lane.getAttribute('data-kp-lane') === key);
    }
}
// The board draws its lanes after a state change; light them again once they stand.
new MutationObserver(() => light(litKey)).observe(chain, { childList: true, subtree: true });

/** @param {any} card */
function noteOf(card) {
    if (card.lane === 'worker') return 'being delivered now';
    if (!card.tries) return 'waiting for its first try';
    if (card.kind === 'token') return `no token for ${tries(card.tries)}`;
    return `${card.strikes > 0 ? 'refused' : 'tried'} ${times(card.tries)}`;
}

function renderChain() {
    if (status === 'loading') return setPipelineState(chain, 'loading');
    if (status === 'error') {
        setPipeline(chain, { lanes: LANES, cards: [] });
        light(null);
        setPipelineState(chain, 'error', 'Could not read the journal.');
        chainSaid.textContent = 'Almanac could not read the journal, so no count is shown.';
        return;
    }
    const { chain: stages, counts } = data;
    const lit = stages.lit ? LIT[stages.lit.reason] : null;
    const lanes = LANES.map((lane) => (lit && lane.key === lit.stage ? { ...lane, label: lit.label } : lane));
    const cards = stages.cards.map((card) =>
        card.lane === 'google'
            ? { id: card.id, lane: 'google', title: card.title, sub: card.source, meta: moment(card.at), mark: markOf(card.source) }
            : {
                  id: card.id,
                  lane: card.lane,
                  title: card.title,
                  sub: `${card.source} · ${noteOf(card)}`,
                  meta: `Received ${moment(card.at)}`,
                  mark: markOf(card.source),
              },
    );
    setPipeline(chain, { lanes, cards });
    setPipelineState(chain, 'ready');
    light(lit ? lit.stage : null);
    const listed = cards.filter((card) => card.lane === 'google').length;
    chainSaid.textContent = [
        `Since Almanac started: ${events(counts.accepted)} accepted, ${counts.delivered} delivered, ${counts.failed} failed ${counts.failed === 1 ? 'try' : 'tries'}, ${counts.set_aside} set aside; ${counts.pending} waiting in the journal.`,
        counts.delivered > listed ? `Google Calendar lists the latest ${listed} deliveries.` : '',
        lit ? lit.said : '',
    ]
        .filter(Boolean)
        .join(' ');
}
chain.addEventListener('kp-pipeline-retry', () => load(true));

/* ------------------------------------------------------------------ 2 · retry schedule */

const KINDS = [
    { kind: 'received', label: 'received', shape: 'square', tone: 'muted' },
    { kind: 'refused', label: 'failed try', shape: 'dot', tone: 'destructive' },
    { kind: 'next', label: 'next try', shape: 'ring', tone: 'info' },
    { kind: 'aside', label: 'third refusal: set aside', shape: 'triangle', tone: 'destructive' },
];

/** The conflict words of each coming pass: the entries whose last strike falls on it. */
function conflicts(schedule) {
    const byPass = new Map();
    for (const entry of schedule.entries) {
        if (!entry.limited) continue;
        const pass = entry.left - 1;
        byPass.set(pass, [...(byPass.get(pass) ?? []), entry]);
    }
    return byPass;
}

function runsData(schedule) {
    const byPass = conflicts(schedule);
    return schedule.next.map((at, i) => {
        const hit = byPass.get(i);
        if (!hit) return { at };
        const text =
            hit.length === 1
                ? `Third refusal for the ${hit[0].source} entry "${hit[0].title}": it is set aside`
                : `Third refusal for ${hit.length} entries: they are set aside`;
        return { at, conflict: text };
    });
}

function ganttData(schedule) {
    const points = [];
    const lanes = [];
    for (const entry of schedule.entries) {
        lanes.push({
            id: entry.id,
            label: entry.title,
            sub: entry.limited ? `${entry.source} · ${tries(entry.left)} left` : `${entry.source} · no limit`,
        });
        points.push({ id: `${entry.id}-in`, lane: entry.id, at: entry.received, kind: 'received', label: `${entry.title}: received ${moment(entry.received)}` });
        const firstStrike = entry.tries.length - entry.strikes;
        entry.tries.forEach((at, i) =>
            points.push({
                id: `${entry.id}-t${at}`,
                lane: entry.id,
                at,
                kind: 'refused',
                label: `${entry.title}: try ${i + 1} failed`,
                detail: entry.limited && i >= firstStrike ? `Permanent refusal ${i - firstStrike + 1} of ${STRIKES_LIMIT}.` : 'Almanac keeps trying.',
            }),
        );
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
    }
    return { points, lanes };
}

const NOTHING_WAITING = 'Nothing is waiting. Every event Almanac accepted has reached its calendar.';

function renderSchedule() {
    runsEl.hidden = shape !== 'runs';
    ganttEl.hidden = shape !== 'gantt';
    if (status === 'ready') {
        const { schedule, counts } = data;
        schedSaid.textContent = schedule.entries.length
            ? `${events(schedule.entries.length)} waiting${schedule.outage_since ? `; deliveries have been failing since ${moment(schedule.outage_since)}` : ''}. All waiting events are tried together.`
            : `Nothing is waiting (${counts.pending} in the journal).`;
    } else {
        schedSaid.textContent = status === 'loading' ? 'Reading the journal.' : 'The journal could not be read.';
    }
    if (shape === 'runs') renderRuns();
    else renderGantt();
}

function renderRuns() {
    if (status === 'loading') return setRunsState(runsEl, 'loading');
    if (status === 'error') return setRunsState(runsEl, 'error', { title: 'Could not read the retry schedule', text: 'The journal did not answer.' });
    const { schedule } = data;
    if (!schedule.entries.length) return setRuns(runsEl, [], { reason: NOTHING_WAITING });
    setRuns(runsEl, runsData(schedule));
}
runsEl.addEventListener('kp-runs-retry', () => load(true));

function renderGantt() {
    if (status === 'loading') return setGanttState(ganttEl, 'loading');
    if (status === 'error') return setGanttState(ganttEl, 'error', 'Could not read the retry schedule: the journal did not answer.');
    const { schedule, chain: stages } = data;
    setGanttKinds(ganttEl, KINDS);
    if (!schedule.entries.length) {
        setGanttPoints(ganttEl, [], []);
        return setGanttState(ganttEl, 'empty', NOTHING_WAITING);
    }
    const { points, lanes } = ganttData(schedule);
    const lit = stages.lit ? LIT[stages.lit.reason] : null;
    setGanttZones(
        ganttEl,
        schedule.outage_since
            ? [{ from: schedule.outage_since, to: data.now, label: `${lit ? lit.label.split(' · ')[0] : 'Delivery'} failing since ${clock(schedule.outage_since)}`, tone: 'warning' }]
            : [],
    );
    setGanttPoints(ganttEl, points, lanes);
    setGanttState(ganttEl, 'ready');
    const start = Math.min(...points.map((p) => p.at), schedule.outage_since ?? Infinity) - 6 * MINUTE;
    const end = Math.max(...schedule.next, data.now) + 6 * MINUTE;
    setGanttWindow(ganttEl, { span: Math.ceil((end - start) / SECOND), live: false, to: end });
}

shapeEl.addEventListener('kp-segmented-change', (event) => {
    const value = /** @type {CustomEvent} */ (event).detail?.value;
    shape = value === 'gantt' ? 'gantt' : 'runs';
    try {
        localStorage.setItem(SHAPE_KEY, shape);
    } catch {
        /* storage off: the shape just does not outlive the page */
    }
    renderSchedule();
});

/* ------------------------------------------------------------------ 3 · set-aside entries */

// The set-aside box's states are the package's: the busy card over the skeleton, the empty plate, the failed plate with Try again.
const asideFrame = /** @type {HTMLElement} */ (asideLoading.parentElement);
statePlate(document, { kind: 'empty', title: 'Nothing has been set aside', text: 'Every event Almanac accepted is either delivered or still being tried.', into: asideEmpty });
asideEmpty.setAttribute('data-kp-art', 'done');
attachArt(asideEmpty.parentElement ?? document);
statePlate(document, { kind: 'error', title: 'Could not read the entries set aside', text: 'The journal did not answer.', actionLabel: 'Try again', onAction: () => load(true), into: asideFailed });
/** @type {import('/static/kp/js/busy-card.js').BusyCard | null} */
let asideBusy = null;

function renderAside() {
    if (status === 'loading' && !asideBusy) asideBusy = openBusyCard(asideFrame, { words: 'Reading the journal', area: () => [asideLoading], arrive: false });
    else if (status !== 'loading' && asideBusy) {
        void asideBusy.close();
        asideBusy = null;
    }
    asideLoading.hidden = status !== 'loading';
    asideFailed.hidden = status !== 'error';
    const list = status === 'ready' ? data.aside : [];
    asideEl.hidden = list.length === 0;
    asideEmpty.hidden = !(status === 'ready' && list.length === 0);
    setProblems(
        asideEl,
        list.map((item) => ({
            key: item.id,
            severity: 'critical',
            what: `An event from ${item.source} was set aside: "${item.title}"`,
            why: `${item.cause} Almanac set it aside on ${moment(item.at)}.`,
            remedy: item.remedy,
        })),
    );
}

/* ------------------------------------------------------------------ data */

function render() {
    renderChain();
    renderSchedule();
    renderAside();
}

let inFlight = false;
/** Reads the route. @param {boolean} [showLoading] lay the loading state first (Try again) */
async function load(showLoading = false) {
    if (inFlight) return;
    inFlight = true;
    if (showLoading) {
        status = 'loading';
        render();
    }
    try {
        const response = await fetch('/deliveries/data', { headers: { 'Content-Type': 'application/json' }, cache: 'no-store' });
        if (!response.ok) throw new Error(String(response.status));
        data = await response.json();
        status = 'ready';
    } catch {
        status = 'error';
    } finally {
        inFlight = false;
    }
    render();
}

render();
load();
setInterval(() => {
    if (!document.hidden) load();
}, POLL_MS);
document.addEventListener('visibilitychange', () => {
    if (!document.hidden) load();
});
