// deliveries page: kp-themes 11.0.0 components wired to Almanac's /deliveries/data route.
// The route answers all three sections from the live state; the sections (js/deliveries/*.js) word it (times in
// Europe/Brussels, dd/mm/yyyy HH:MM, 24 h) and hand it to the package's components.
import './deliveries/dom.js';
import { initChain } from './deliveries/chain.js';
import { initSchedule } from './deliveries/schedule.js';
import { initAside } from './deliveries/aside.js';
import { load, render, startPolling } from './deliveries/load.js';

const retry = () => load(true);
initChain(retry);
initSchedule(retry);
initAside(retry, () => load());
render();
load();
startPolling();
