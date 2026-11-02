// Section 4 of the Calendars page: the typed delete of a calendar.
import { runFlashButton, buildTypeConfirm } from '/static/kp/dist/kp-themes.min.js';
import { api, flavour, said } from './api.js';
import { state, hooks } from './state.js';
import { must, tbody } from './dom.js';

/** How long the dialog stays open after a delete, so the person sees "Deleted" before it closes. */
const CLOSE_AFTER_DELETE_MS = 600;

const dialog = /** @type {HTMLDialogElement} */ (must('[data-cal-delete-dialog]'));
const blockEl = must('[data-cal-type-block]', dialog);
const go = /** @type {HTMLButtonElement} */ (must('[data-cal-del-go]', dialog));
const result = must('[data-cal-result]', dialog);
const title = must('.kp-dialog__title', dialog);
/** The calendar the dialog is about. @type {import('./state.js').Calendar | null} */
let target = null;

/** Opens the dialog for the row's Delete button. @param {HTMLButtonElement} button */
function openFor(button) {
    target = state.google?.calendars.find((c) => c.id === button.getAttribute('data-cal-delete')) ?? null;
    if (!target) return;
    title.textContent = `Delete the calendar ${target.name}?`;
    result.textContent = '';
    go.disabled = true;
    buildTypeConfirm(document, {
        name: target.name,
        loses: ['Every event on it', 'The calendar itself, for everyone it is shared with'],
        block: blockEl,
        onChange: (matched) => {
            go.disabled = !matched;
        },
    });
    dialog.showModal();
}

/** Deletes the calendar on the server. Returns the refusal as the button's outcome, and reloads when the set-up moved. @param {import('./state.js').Calendar} c */
async function removeOnServer(c) {
    try {
        await api('/calendars/remove', { id: c.id });
    } catch (e) {
        // the set-up changed under the page (a source started writing here): show it as it is now
        void hooks.load(true);
        return { refused: /** @type {{ error: string, remedy?: string }} */ (e) };
    }
    result.textContent = `Deleted ${c.name}.`;
    return { refused: null };
}

function confirmDelete() {
    const c = target;
    if (!c) return;
    /** @type {{ error: string, remedy?: string } | null} */
    let refused = null;
    runFlashButton(
        go,
        async () => {
            const done = await removeOnServer(c);
            refused = done.refused;
            return done.refused ? /** @type {any} */ (done.refused) : { word: 'Deleted' };
        },
        { refusal: 'button', busyLabel: 'Deleting' },
    )
        .then((outcome) => {
            if (refused) return void flavour('destructive', `${c.name} was not deleted: ${said(refused)}`);
            if (!outcome?.ok || !state.google) return;
            state.google.calendars = state.google.calendars.filter((x) => x.id !== c.id);
            hooks.render();
            flavour('success', `Deleted ${c.name} and every event on it.`);
            setTimeout(() => {
                dialog.close();
                // The row that opened the dialog is gone, so focus goes to the section's heading.
                must('#cal-cal-title').focus();
            }, CLOSE_AFTER_DELETE_MS);
        })
        .catch((e) => flavour('error', `Deleting ${c.name} failed: ${e instanceof Error ? e.message : String(e)}`));
}

export function initDeleteDialog() {
    tbody.addEventListener('click', (event) => {
        const button = event.target instanceof Element ? /** @type {HTMLButtonElement | null} */ (event.target.closest('[data-cal-delete]')) : null;
        if (!button || button.disabled || !state.google) return;
        openFor(button);
    });
    must('[data-cal-del-cancel]', dialog).addEventListener('click', () => dialog.close());
    go.addEventListener('click', confirmDelete);
}
