// Loading for the deliveries page: polling /deliveries/data and drawing the three sections from the answer.
import { state, SECOND } from './state.js';
import { renderChain } from './chain.js';
import { renderSchedule } from './schedule.js';
import { renderAside } from './aside.js';
import { warnRead } from '../lib/leaving.js';

/** How often the page reads the route while it is visible. */
const POLL_MS = 5 * SECOND;

export function render() {
    renderChain();
    renderSchedule();
    renderAside();
}

let inFlight = false;
/** The last answer's text, to tell a poll that changed nothing. */
let lastBody = '';
/** Whether the last answer has something that moves with the clock. */
let moving = false;

/** One read of the route into `state`. */
async function readRoute() {
    const response = await fetch('/deliveries/data', { headers: { 'Content-Type': 'application/json' }, cache: 'no-store' });
    if (!response.ok) throw new Error(`/deliveries/data answered ${response.status}`);
    const body = await response.text();
    lastBody = body.replace(/"now":\d+,?/, '');
    state.data = JSON.parse(body);
    // The timeline's window moves with the clock while anything waits, so then every answer draws.
    moving = state.data.schedule.entries.length > 0 || state.data.schedule.outage_since !== null;
    state.status = 'ready';
}

/** Reads the route. @param {boolean} [showLoading] lay the loading state first (Try again) */
export async function load(showLoading = false) {
    if (inFlight) return;
    inFlight = true;
    if (showLoading) {
        state.status = 'loading';
        render();
    }
    const before = `${state.status}:${lastBody}`;
    try {
        await readRoute();
    } catch (error) {
        warnRead('deliveries: the journal could not be read', error);
        state.status = 'error';
        lastBody = '';
        moving = false;
    } finally {
        inFlight = false;
    }
    // A poll that read the same answer draws nothing again: a state plate (and its Retry) stays put, and a focused one keeps its focus.
    if (!showLoading && !moving && before === `${state.status}:${lastBody}`) return;
    render();
}

export function startPolling() {
    setInterval(() => {
        if (!document.hidden) load();
    }, POLL_MS);
    document.addEventListener('visibilitychange', () => {
        if (!document.hidden) load();
    });
}
