// Section 5 of the Calendars page: the new-calendar dialog.
import { attachDisabledReasons, form as formOf, VALID_EVENT, INVALID_EVENT } from '/static/kp/dist/kp-themes.min.js';
import { api, flavour, said } from './api.js';
import { state, hooks } from './state.js';
import { must } from './dom.js';

const newDialog = /** @type {HTMLDialogElement} */ (must('[data-cal-new-dialog]'));
const newForm = /** @type {HTMLFormElement} */ (must('[data-cal-new-form]', newDialog));
const nameInput = /** @type {HTMLInputElement} */ (must('input[name="name"]', newForm));

export function openNew() {
    const goBtn = /** @type {HTMLButtonElement} */ (must('[data-cal-new-go]', newForm));
    const sharing = must('[data-cal-sharing]', newForm);
    const canCreate = state.google?.can_create !== false;
    nameInput.disabled = !canCreate;
    goBtn.disabled = !canCreate;
    if (!canCreate) {
        sharing.textContent = 'Almanac cannot make calendars: ALMANAC_CALENDAR_OWNER is not set, so a new calendar would belong to the service account and be visible to nobody.';
    }
    newDialog.showModal();
}

/** The form passed validation: make the calendar and say how it went. @param {Event} event */
async function onValid(event) {
    const detail = /** @type {CustomEvent} */ (event).detail;
    try {
        const made = await api('/calendars/make', { name: nameInput.value.trim() });
        const google = state.google;
        if (google?.ok && !google.calendars.some((c) => c.id === made.id)) {
            google.calendars = [...google.calendars, { id: made.id, name: made.name }].sort((a, b) => a.name.localeCompare(b.name));
            hooks.render();
        }
        flavour('success', `Made ${made.name} and shared it with you.`);
        nameInput.value = '';
        newDialog.close();
    } catch (e) {
        flavour('error', said(/** @type {any} */ (e)));
    }
    detail.done();
}

export function initNewDialog() {
    formOf(newForm);
    newForm.addEventListener(VALID_EVENT, onValid);
    newForm.addEventListener(INVALID_EVENT, () => nameInput.focus({ preventScroll: true }));
    must('[data-cal-new-cancel]', newDialog).addEventListener('click', () => newDialog.close());
    for (const b of document.querySelectorAll('[data-cal-open-new]')) b.addEventListener('click', openNew);
    attachDisabledReasons(newDialog);
}
