// Try a source: the page's own wiring. Every component is the package's (kp-themes 11.0.0); this file hands them Almanac's
// real data from the two JSON routes of src/shell/ui/try_source.rs and wires the states (loading while asking, empty,
// failed with a working Retry). The sections live in js/try/*.js.
import { initSources, loadSources } from './try/sources.js';
import { initEditor } from './try/editor.js';

initSources();
initEditor();
await loadSources();
