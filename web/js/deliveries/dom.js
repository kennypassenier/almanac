// The deliveries page's elements and the package parts attached to them. Importing this waits for the package.
import { attachGantts, attachPipelines, attachProblems, attachRuns } from '/static/kp/dist/kp-themes.min.js';
import { autoReady } from '../lib/ready.js';

await autoReady;

export const $ = (/** @type {string} */ id) => /** @type {HTMLElement} */ (document.getElementById(id));
export const chain = $('ad-chain');
export const chainSaid = $('ad-chain-said');
export const runsEl = $('ad-runs');
export const ganttEl = $('ad-gantt');
export const schedSaid = $('ad-sched-said');
export const asideEl = $('ad-aside');
export const asideLoading = $('ad-aside-loading');
export const asideEmpty = $('ad-aside-empty');
export const asideFailed = $('ad-aside-failed');
export const shapeEl = $('ad-shape');

attachPipelines(chain.parentElement ?? document);
attachRuns(runsEl.parentElement ?? document);
attachGantts(ganttEl.parentElement ?? document);
attachProblems(asideEl.parentElement ?? document);
