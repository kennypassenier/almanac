// Section 3 of the deliveries page: the entries set aside. The states are the package's: the busy card over the
// skeleton, the empty plate, the failed plate with Try again.
import { setProblems, attachArt, statePlate, openBusyCard } from '/static/kp/dist/kp-themes.min.js';
import { moment } from '../lib/format.js';
import { state } from './state.js';
import { asideEl, asideLoading, asideEmpty, asideFailed } from './dom.js';

/** @type {import('/static/kp/dist/kp-themes.min.js').BusyCard | null} */
let asideBusy = null;

/** @param {() => void} onRetry reads the journal again */
export function initAside(onRetry) {
    statePlate(document, { kind: 'empty', title: 'Nothing has been set aside', text: 'Every event Almanac accepted is either delivered or still being tried.', into: asideEmpty });
    asideEmpty.setAttribute('data-kp-art', 'done');
    attachArt(asideEmpty.parentElement ?? document);
    statePlate(document, { kind: 'error', title: 'Could not read the entries set aside', text: 'The journal did not answer.', actionLabel: 'Try again', onAction: onRetry, into: asideFailed });
}

/** The busy card is open exactly while the first read is pending. */
function syncBusyCard() {
    const frame = /** @type {HTMLElement} */ (asideLoading.parentElement);
    if (state.status === 'loading' && !asideBusy) asideBusy = openBusyCard(frame, { words: 'Reading the journal', area: () => [asideLoading], arrive: false });
    else if (state.status !== 'loading' && asideBusy) {
        void asideBusy.close();
        asideBusy = null;
    }
}

export function renderAside() {
    syncBusyCard();
    asideLoading.hidden = state.status !== 'loading';
    asideFailed.hidden = state.status !== 'error';
    const list = state.status === 'ready' ? state.data.aside : [];
    asideEl.hidden = list.length === 0;
    asideEmpty.hidden = !(state.status === 'ready' && list.length === 0);
    setProblems(
        asideEl,
        list.map((/** @type {any} */ item) => ({
            key: item.id,
            severity: 'critical',
            what: `An event from ${item.source} was set aside: "${item.title}"`,
            why: `${item.cause} Almanac set it aside on ${moment(item.at)}.`,
            remedy: item.remedy,
        })),
    );
}
