// Small pieces of markup and wording shared by the Try page's sections: fact rows, alerts, refusals.
import { esc } from '../lib/format.js';


/** A fact row on the form grid's tracks: label | value. Copying is the card's Copy menu. @param {string} label @param {string} value */
export function factRow(label, value) {
    return `<div class="kp-formgrid__row"><span class="kp-formgrid__label">${esc(label)}</span><div class="kp-formgrid__control"><span>${esc(value)}</span></div></div>`;
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
