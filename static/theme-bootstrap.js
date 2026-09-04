// Almanac's own glue between kp-themes and Bootstrap [K25].
//
// Not part of the package, and deliberately so: kp-themes sets
// `data-theme` and the `.dark` class on <html>, which is everything its
// own stylesheet needs. Bootstrap reads a third thing — `data-bs-theme`
// — and without it a dark theme paints dark tokens behind light cards.
//
// The dark list lives in Rust and is printed into the head snippet for
// the first paint; from the moment the picker module loads, `.dark` is
// authoritative and this reads it rather than keeping a second list.
(function () {
    function sync() {
        var root = document.documentElement;
        root.setAttribute('data-bs-theme', root.classList.contains('dark') ? 'dark' : 'light');
    }

    // The package announces every change as one DOM event, whichever
    // channel made it — a click here, or a picker on another part of the
    // page. Listening to that rather than to the button means nothing
    // breaks if the markup moves.
    document.addEventListener('kp-theme-change', sync);
    sync();
})();
