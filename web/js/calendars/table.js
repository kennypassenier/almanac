// Section 3 of the Calendars page: the calendars table, with its empty plate.
import { statePlate, attachDisabledReasons, attachCodes, attachArt } from '/static/kp/dist/js/index.js';
import { esc } from '../lib/format.js';
import { state, hooks, sourcesOf, sourceCardText } from './state.js';
import { must, table, tableWrap, tbody } from './dom.js';
import { openNew } from './new-dialog.js';

/** @param {import('./state.js').Calendar} c */
function calendarRow(c) {
    const used = sourcesOf(c.id);
    const tags = used.length
        ? `<ul class="kp-tag-list" aria-label="Sources that write to ${esc(c.name)}">${used.map((s) => `<li class="kp-tag" tabindex="0" aria-label="${esc(sourceCardText(s))}" data-kp-hovercard="${esc(sourceCardText(s))}">${esc(s.id)}</li>`).join('')}</ul>`
        : '<span class="kp-text-muted">No source writes here</span>';
    const base = `type="button" class="kp-button kp-button--destructive kp-button--sm" data-kp-destructive data-kp-confirm-owner data-cal-delete="${esc(c.id)}"`;
    const names = used.map((s) => s.id).join(', ');
    const del = used.length
        ? `<button ${base} disabled aria-label="Delete ${esc(c.name)}" data-kp-disabled-reason="Used by ${esc(names)}. Delete ${used.length === 1 ? 'that source' : 'those sources'} first, on the Sources page.">Delete</button>`
        : `<button ${base} aria-label="Delete ${esc(c.name)}">Delete</button>`;
    return `<tr data-kp-row-key="${esc(c.id)}">
<td data-label="Name" data-kp-sort-value="${esc(c.name)}">${esc(c.name)}</td>
<td data-label="Calendar id"><figure class="kp-code kp-code--cell" data-kp-code aria-label="Calendar id of ${esc(c.name)}"><pre class="kp-code__body"><code>${esc(c.id)}</code></pre></figure></td>
<td data-label="Used by" data-kp-sort-value="${used.length}">${tags}</td>
<td data-label="Actions" class="kp-text-end"><span class="kp-row-actions">${del}</span></td>
</tr>`;
}

export function renderTable() {
    const google = state.google;
    if (!google) {
        table.busy({ text: 'Reading the calendars', overlay: true });
        table.state('loading');
    } else if (!google.ok) {
        // One outage, one alert: the band above holds Check again, so the table only says why it is empty.
        tbody.innerHTML = '';
        drawEmptyPlate(GOOGLE_DOWN);
        table.refresh();
        table.busy(null);
        table.state('ready');
    } else {
        drawEmptyPlate(NO_CALENDARS);
        tbody.innerHTML = google.calendars.map(calendarRow).join('\n');
        attachDisabledReasons(tbody);
        attachCodes(tbody);
        table.refresh();
        table.busy(null);
        table.state('ready');
    }
}

/** The words of the table's empty plate: no calendars at all, or Google not answering. */
const NO_CALENDARS = { title: 'No calendars yet', text: 'The service account sees none. Make the first one and Almanac shares it with you.', action: true };
const GOOGLE_DOWN = { title: 'The calendars cannot be listed right now', text: 'Google Calendar does not answer; the alert above checks again.', action: false };

/** The table's own empty plate; with a New calendar button only when Google answers. @param {{ title: string, text: string, action: boolean }} words */
function drawEmptyPlate(words) {
    const slot = must('[data-kp-datatable-empty]', tableWrap);
    const plate = statePlate(document, {
        kind: 'empty',
        title: words.title,
        text: words.text,
        ...(words.action ? { actionLabel: 'New calendar', onAction: () => openNew() } : {}),
    });
    plate.setAttribute('data-kp-art', 'none');
    slot.className = plate.className;
    slot.setAttribute('data-kp-art', 'none');
    slot.replaceChildren(...plate.childNodes);
    slot.querySelector('.kp-stateplate__title')?.classList.add('kp-empty__title');
    attachArt(slot.parentElement ?? document);
}

export function initTable() {
    drawEmptyPlate(NO_CALENDARS);
    tableWrap.addEventListener('kp-datatable-retry', () => void hooks.loadGoogle());
}
