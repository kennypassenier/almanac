// Shared by the page scripts served from the kit's auto.js: one promise that settles when the package has attached
// everything the page carries.

/** Resolves once js/auto.js has attached everything the page carries. */
export const autoReady = new Promise((resolve) => {
    const done = () => document.documentElement.hasAttribute('data-kp-auto-ready');
    if (done()) return resolve(undefined);
    new MutationObserver((_, watch) => {
        if (done()) {
            watch.disconnect();
            resolve(undefined);
        }
    }).observe(document.documentElement, { attributes: true, attributeFilter: ['data-kp-auto-ready'] });
});
