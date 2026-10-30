// The Calendars page (kp-themes 11.0.0). The page renders at once; this module asks the JSON routes in
// src/shell/ui/calendars.rs for the data, so a slow Google never holds the page back. Every card shows loading from the
// first frame, then ready, empty or failed with a working Retry. The sections live in js/calendars/*.js.
import { must } from './calendars/dom.js';
import { flavour } from './calendars/api.js';
import { load } from './calendars/load.js';
import { initStrip } from './calendars/strip.js';
import { initFixes } from './calendars/fixes.js';
import { initTable } from './calendars/table.js';
import { initDeleteDialog } from './calendars/delete-dialog.js';
import { initNewDialog } from './calendars/new-dialog.js';
import { initProfiles } from './calendars/profiles.js';
import { initUses, showView } from './calendars/uses.js';

initStrip();
initFixes();
initTable();
initDeleteDialog();
initNewDialog();
initProfiles();
initUses();

const boot = JSON.parse(must('#cal-boot').textContent ?? '{}');
if (boot.notice) flavour('error', boot.notice);
showView('lines');
await load();
