// Almanac's own glue between kp-themes and Bootstrap [K25].
//
// Not part of the package, and deliberately so: kp-themes sets
// `data-theme` and the `.dark` class on <html>, which is everything its
// own stylesheet needs. Bootstrap reads a third thing — `data-bs-theme`
// — and without it a dark theme paints dark tokens behind light cards.
//
// It keeps no list of which themes are dark. Every theme declares its
// own `color-scheme`, so this asks the browser what is actually applied
// rather than holding an answer that a palette change upstream would
// quietly falsify. The head does the same thing once, before the first
// paint; this is what keeps it true afterwards.
(function () {
    function sync() {
        var root = document.documentElement;
        var dark = getComputedStyle(root).colorScheme.indexOf('dark') !== -1;
        root.setAttribute('data-bs-theme', dark ? 'dark' : 'light');
    }

    // The package announces every change as one DOM event, whichever
    // channel made it — a click here, or a picker elsewhere on the page.
    // Listening to that rather than to the button means nothing breaks
    // if the markup moves.
    document.addEventListener('kp-theme-change', sync);
    sync();
})();
