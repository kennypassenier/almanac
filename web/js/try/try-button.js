// The Try button: enabled only when a source is loaded and the event call is valid, with the reason on the button.
import { state } from './state.js';
import { tryButton } from './dom.js';

export function refreshTry() {
    const reason = !state.sources.length ? 'No source is loaded' : state.fieldReason;
    tryButton.disabled = reason !== '';
    if (reason) tryButton.setAttribute('data-kp-disabled-reason', reason);
    else tryButton.removeAttribute('data-kp-disabled-reason');
}
