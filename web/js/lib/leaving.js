// Shared by the page scripts: a read the browser cancels because the page is being left is not a fault, so its
// warning stays out of the console (Firefox rejects such a fetch with a plain NetworkError, not an AbortError).

let leaving = false;
addEventListener('beforeunload', () => (leaving = true));
addEventListener('pagehide', () => (leaving = true));
// A navigation that was cancelled, or a page restored from the back-forward cache, is staying after all.
addEventListener('pageshow', () => (leaving = false));
addEventListener('focus', () => (leaving = false));

/** Logs a failed read unless the page is being left. @param {string} message @param {unknown} error */
export function warnRead(message, error) {
    if (!leaving) console.warn(message, error);
}
