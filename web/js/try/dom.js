// The Try page's elements, looked up once. Importing this waits for the package to attach its parts.
import { autoReady } from '../lib/ready.js';

await autoReady;

export const $ = (/** @type {string} */ id) => /** @type {HTMLElement} */ (document.getElementById(id));

export const profileCard = $('profile-card');
export const profileToml = $('profile-toml');
export const tryButton = /** @type {HTMLButtonElement} */ ($('editor-try'));
export const area = /** @type {HTMLTextAreaElement} */ ($('editor-body'));
export const editorCard = $('editor-card');
export const editorForm = $('editor-form');
export const editorSource = $('editor-source');
export const traceEl = $('try-trace');
export const traceBody = $('trace-body');
export const traceHttp = $('trace-http');
export const resultCard = $('result-card');
export const badge = $('result-badge');
export const note = $('result-note');
export const specArea = $('result-spec-area');
export const diffArea = $('result-diff-area');
export const curlArea = $('result-curl-area');
