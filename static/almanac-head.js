// Runs in <head> AFTER the stylesheets, before anything painted (3.0.0: a
// file, because the kit's CSP forbids inline scripts).
//
// Bootstrap decides light or dark from data-bs-theme, which the kp-themes
// package knows nothing about and should not. Almanac has to set it, and
// the answer is already in the stylesheet: every theme declares its own
// color-scheme. So ask, rather than keep a list of dark theme names that
// goes stale the day a palette changes sides. theme-bootstrap.js keeps it
// in step from here on.
(function () {
  var root = document.documentElement;
  var dark = getComputedStyle(root).colorScheme.indexOf('dark') !== -1;
  root.classList.toggle('dark', dark);
  root.setAttribute('data-bs-theme', dark ? 'dark' : 'light');
})();
