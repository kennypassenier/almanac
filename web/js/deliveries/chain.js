// Section 1 of the deliveries page: the delivery chain (received, journal, worker, Google), with the failing lane lit.
import { setPipeline, setPipelineState } from '/static/kp/js/pipeline.js';
import { moment } from '../lib/format.js';
import { state } from './state.js';
import { chain, chainSaid } from './dom.js';
import { events, tries, times, markOf } from './words.js';

const LANES = [
    { key: 'received', label: 'Received', tone: 1 },
    { key: 'journal', label: 'Journal', tone: 2 },
    { key: 'worker', label: 'Worker', tone: 3 },
    { key: 'google', label: 'Google Calendar', tone: 4 },
    { key: 'dead', label: 'Set aside · never retried', closed: true },
];
/** What the server's reason for a stalled chain lights, and what it says about it. */
export const LIT = {
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

/** @param {any} card */
function noteOf(card) {
    if (card.lane === 'worker') return 'being delivered now';
    if (!card.tries) return 'waiting for its first try';
    if (card.kind === 'token') return `no token for ${tries(card.tries)}`;
    return `${card.strikes > 0 ? 'refused' : 'tried'} ${times(card.tries)}`;
}

/** @param {any} card */
function cardOf(card) {
    if (card.lane === 'google') return { id: card.id, lane: 'google', title: card.title, sub: card.source, meta: moment(card.at), mark: markOf(card.source) };
    return { id: card.id, lane: card.lane, title: card.title, sub: `${card.source} · ${noteOf(card)}`, meta: `Received ${moment(card.at)}`, mark: markOf(card.source) };
}

/** @param {any} counts @param {number} listed @param {{ said: string } | null} lit */
function counted(counts, listed, lit) {
    return [
        `Since Almanac started: ${events(counts.accepted)} accepted, ${counts.delivered} delivered, ${counts.failed} failed ${counts.failed === 1 ? 'try' : 'tries'}, ${counts.set_aside} set aside; ${counts.pending} waiting in the journal.`,
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
        light(null);
        setPipelineState(chain, 'error', 'Could not read the journal.');
        chainSaid.textContent = 'Almanac could not read the journal, so no count is shown.';
        return;
    }
    const { chain: stages, counts } = state.data;
    const lit = stages.lit ? /** @type {any} */ (LIT)[stages.lit.reason] : null;
    const lanes = LANES.map((lane) => (lit && lane.key === lit.stage ? { ...lane, label: lit.label } : lane));
    const cards = stages.cards.map(cardOf);
    setPipeline(chain, { lanes, cards });
    setPipelineState(chain, 'ready');
    light(lit ? lit.stage : null);
    chainSaid.textContent = counted(counts, cards.filter((card) => card.lane === 'google').length, lit);
}

/** @param {() => void} onRetry reads the journal again */
export function initChain(onRetry) {
    // The board draws its lanes after a state change; light them again once they stand.
    new MutationObserver(() => light(litKey)).observe(chain, { childList: true, subtree: true });
    chain.addEventListener('kp-pipeline-retry', onRetry);
}
