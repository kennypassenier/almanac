// What each Copy menu on the Try page copies: the profile's ids and file, the event and curl of the last Try, its answer.
import { state, sourceById } from './state.js';
import { specRows, curlFor } from './spec.js';
import { initCopyMenu } from './copy-menu.js';

const menu = (/** @type {string} */ key) => /** @type {HTMLElement} */ (document.querySelector(`[data-try-copy="${key}"]`));
const profile = () => sourceById(state.profileSource);
const accepted = () => (state.last?.data.http === 200 ? state.last.data : null);

export function initCopies() {
    initCopyMenu(menu('profile'), {
        'source-id': () => profile()?.source_id ?? '',
        'calendar-id': () => profile()?.calendar_id ?? '',
        'profile-file': () => profile()?.toml ?? '',
    });
    initCopyMenu(menu('result'), {
        'event-text': () => {
            const data = accepted();
            return data ? specRows(data.answer.event, data.calendar_name ?? '').map(([label, value]) => `${label}: ${value}`).join('\n') : '';
        },
        curl: () => (state.last ? curlFor(state.last.data.urls.dry_run, state.last.payload) : ''),
    });
    initCopyMenu(menu('trace'), {
        answer: () => (state.last ? JSON.stringify(state.last.data.answer, null, 2) : ''),
    });
}
