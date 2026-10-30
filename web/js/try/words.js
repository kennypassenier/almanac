// Small pieces of markup and wording shared by the Try page's sections: fact rows, alerts, refusals.
import { esc } from '../lib/format.js';

let serial = 0;

/** A fact row on the form grid's tracks: label | value (control) | Copy (actions, only when `copy` is set). @param {string} label @param {string} value @param {boolean} copy */
export function factRow(label, value, copy) {
    const id = `try-fact-${(serial += 1)}`;
    const body = copy ? `<span class="kp-copyable__value" id="${id}-v">${esc(value)}</span>` : esc(value);
    const action = copy
        ? `<div class="kp-formgrid__actions"><button type="button" class="kp-button kp-button--sm kp-button--ghost kp-copyable__button" data-kp-copy="${id}-v" aria-describedby="${id}-l">Copy</button></div>`
        : '';
    return `<div class="kp-formgrid__row"><span class="kp-formgrid__label" id="${id}-l">${esc(label)}</span><div class="kp-formgrid__control"><span>${body}</span></div>${action}</div>`;
}

/** @param {HTMLElement} alert @param {'info' | 'success' | 'warning' | 'destructive'} tone @param {string} label @param {string} text */
export function setAlert(alert, tone, label, text) {
    alert.className = `kp-alert kp-alert--${tone}`;
    alert.setAttribute('role', tone === 'destructive' ? 'alert' : 'status');
    alert.innerHTML = `<span><span class="kp-alert__label">${esc(label)}: </span>${esc(text)}</span>`;
}

/** A refusal in two plain parts: what is wrong, and what to do (the API repeats the remedy inside `message`). @param {{ message: string, remedy: string }} body */
export function refusalWords(body) {
    const what = body.message.endsWith(` — ${body.remedy}`) ? body.message.slice(0, -(body.remedy.length + 3)) : body.message;
    return `${what.replace(/, expected one of .*$/, '')}. ${body.remedy[0].toUpperCase()}${body.remedy.slice(1)}`;
}
