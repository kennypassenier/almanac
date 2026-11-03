// Section 1 of the deliveries page: the delivery chain (received, journal, worker, Google), with the failing lane flagged.
import { setPipeline, setPipelineState } from '/static/kp/dist/js/index.js';
import { moment } from '../lib/format.js';
import { state } from './state.js';
import { chain, chainSaid, say } from './dom.js';
import { events, tries, times, markOf } from './words.js';

const LANES = [
    { key: 'received', label: 'Received', tone: 1 },
    { key: 'journal', label: 'Journal', tone: 2 },
    { key: 'worker', label: 'Worker', tone: 3 },
    { key: 'google', label: 'Google Calendar', tone: 4 },
    { key: 'dead', label: 'Set aside · not tried again', closed: true },
];
/** What the server's reason for a stalled chain lights (`stage`), what the stage is called in a sentence, and what it says about it. */
export const LIT = {
    temporary: { stage: 'google', name: 'Google', said: 'Google is not accepting the events right now. The waiting ones are tried again.' },
    refused: { stage: 'google', name: 'Google', said: 'Google refuses at least one event for good. It is set aside after the third refusal; the others are tried again.' },
    token: { stage: 'worker', name: 'The worker', said: 'The worker cannot get a Google token, so no call to Google is made. The events wait in the journal.' },
    local: { stage: 'worker', name: 'The worker', said: 'Almanac cannot turn a waiting event into a calendar entry. The events wait in the journal.' },
};

/** @param {any} card */
function noteOf(card) {
    if (card.lane === 'worker') return 'being delivered now';
    if (!card.tries) return 'waiting for its first try';
    if (card.kind === 'token') return `no token for ${tries(card.tries)}`;
    return `${card.strikes > 0 ? 'refused' : 'tried'} ${times(card.tries)}`;
}

/** @param {any} card */
function cardOf(card) {
    if (card.lane === 'dead') return { id: card.id, lane: 'dead', title: card.title, sub: `${card.source} · ${card.short.toLowerCase()}`, meta: `Set aside ${moment(card.at)}`, mark: markOf(card.source, state.data.marks) };
    if (card.lane === 'google') return { id: card.id, lane: 'google', title: card.title, sub: card.source, meta: moment(card.at), mark: markOf(card.source, state.data.marks) };
    return { id: card.id, lane: card.lane, title: card.title, sub: `${card.source} · ${noteOf(card)}`, meta: `Received ${moment(card.at)}`, mark: markOf(card.source, state.data.marks) };
}

/** What the empty board says, from the counts since Almanac started. @param {any} counts */
function emptyBoard(counts) {
    if (counts.delivered > 0) return `${events(counts.delivered)} delivered since Almanac started; none are listed on the board now.`;
    return 'Nothing has reached Almanac since it started.';
}

/** @param {any} counts @param {number} listed @param {number} setAside @param {{ said: string } | null} lit */
function counted(counts, listed, setAside, lit) {
    return [
        `Since Almanac started: ${events(counts.accepted)} accepted, ${counts.delivered} delivered, ${counts.failed} failed ${counts.failed === 1 ? 'try' : 'tries'}. Now: ${counts.pending} waiting in the journal, ${setAside} set aside.`,
        counts.delivered > listed ? `Google Calendar lists the latest ${listed} deliveries.` : '',
        lit ? lit.said : '',
    ]
        .filter(Boolean)
        .join(' ');
}

export function renderChain() {
    if (state.status === 'loading') return setPipelineState(chain, 'loading');
    if (state.status === 'error') {
        setPipeline(chain, { lanes: LANES, cards: [] });
        setPipelineState(chain, 'error', 'Could not read the journal.');
        say(chainSaid, 'Almanac could not read the journal, so no count is shown.');
        return;
    }
    const { chain: stages, counts, aside } = state.data;
    const lit = stages.lit ? /** @type {any} */ (LIT)[stages.lit.reason] : null;
    // The failing stage carries the package's own flag; its count stays the number of cards that really sit in it.
    const lanes = LANES.map((lane) => (lit && lane.key === lit.stage ? { ...lane, danger: true } : lane));
    const cards = stages.cards.map(cardOf);
    setPipeline(chain, { lanes, cards });
    setPipelineState(chain, cards.length ? 'ready' : 'empty', emptyBoard(counts));
    say(chainSaid, counted(counts, cards.filter((card) => card.lane === 'google').length, aside.length, lit));
}

/** @param {() => void} onRetry reads the journal again */
export function initChain(onRetry) {
    chain.addEventListener('kp-pipeline-retry', onRetry);
}
