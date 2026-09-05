// Pure since kp-themes 3.0.0: importing theme-picker.js attaches nothing
// by itself, so the call has to be made explicitly. Not js/auto.js — that
// one script also wires up datatables, comboboxes, date pickers and eight
// other components almanac's dashboard does not use; attaching only the
// picker keeps what almanac ships in step with what it actually shows.
// (3.0.0: a file, because the kit's CSP forbids inline module blocks.)
import { attachThemePickers } from '/static/theme-picker.js';
attachThemePickers();
