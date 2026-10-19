// The Calendars page (kp-themes 11.0.0). The page renders at once; this module asks the JSON routes in
// src/shell/ui/calendars.rs for the data, so a slow Google never holds the page back. Every card shows loading from the
// first frame, then ready, empty or failed with a working Retry.
import { attachKpiToggles } from '/static/kp/js/kpi.js';
import { attachCharts } from '/static/kp/js/chart.js';
import { statePlate } from '/static/kp/js/state-plate.js';
import { toast } from '/static/kp/js/overlays.js';
import { runFlashButton } from '/static/kp/js/flash.js';
import { setAttention } from '/static/kp/js/attention.js';
import { setProblems } from '/static/kp/js/problems.js';
import { setCardState } from '/static/kp/js/card-state.js';
import { dataTable } from '/static/kp/js/datatable.js';
import { attachDisabledReasons } from '/static/kp/js/reason.js';
import { attachCodes } from '/static/kp/js/code.js';
import { attachArt } from '/static/kp/js/art.js';
import { buildTypeConfirm } from '/static/kp/js/components.js';
import { form as formOf, VALID_EVENT, INVALID_EVENT } from '/static/kp/js/forms.js';
import { setMatrix } from '/static/kp/js/heat.js';
import { setGraphData, setGraphState } from '/static/kp/js/graph.js';
import { setSegmentedValues } from '/static/kp/js/segmented.js';

const $ = (/** @type {string} */ s, /** @type {ParentNode} */ r = document) => /** @type {HTMLElement | null} */ (r.querySelector(s));
const must = (/** @type {string} */ s, /** @type {ParentNode} */ r = document) => /** @type {HTMLElement} */ ($(s, r));

const esc = (/** @type {unknown} */ s) =>
    String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

const stamp = new Intl.DateTimeFormat('en-GB', {
    timeZone: 'Europe/Brussels',
    day: '2-digit',
    month: '2-digit',
    year: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
});
const clock = new Intl.DateTimeFormat('en-GB', { timeZone: 'Europe/Brussels', hour: '2-digit', hourCycle: 'h23' });
/** dd/mm/yyyy HH:MM, Europe/Brussels. @param {number} ms */
const when = (ms) => stamp.format(ms).replace(',', '');
const plural = (/** @type {number} */ n, /** @type {string} */ one, /** @type {string} */ many) => `${n} ${n === 1 ? one : many}`;

// ── the server ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────
/** A refusal in the shape the package shows: a sentence and what to do. @param {string} error @param {string} remedy */
const refusal = (error, remedy) => ({ error, remedy });

/** One JSON call. Rejects with `{ error, remedy }` (the server's own words when it sent them). @param {string} path @param {unknown} [body] */
async function api(path, body) {
    let res;
    try {
        res = await fetch(path, {
            method: body === undefined ? 'GET' : 'POST',
            headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
            body: body === undefined ? undefined : JSON.stringify(body),
            credentials: 'same-origin',
        });
    } catch {
        throw refusal('Almanac did not answer.', 'Check that the service is running, then try again.');
    }
    /** @type {any} */
    let json = null;
    try {
        json = await res.json();
    } catch {
        // an HTML answer is the login page: the session ended
    }
    if (!res.ok || json === null) {
        throw refusal(
            json?.error ?? (json === null ? 'Your session ended.' : 'Almanac could not do that.'),
            json?.remedy ?? (json === null ? 'Reload the page and log in again.' : 'Try again in a moment.'),
        );
    }
    return json;
}

/** The toast of one flavour (success, info, warning, error, destructive). @param {string} name @param {string} words */
const flavour = (name, words) =>
    toast(words, { className: `kp-toast--${name}`, ...(name === 'error' || name === 'destructive' ? { live: /** @type {const} */ ('assertive') } : {}) });
const said = (/** @type {{ error: string, remedy?: string }} */ e) => `${e.error.replace(/\.$/, '')}. ${e.remedy ?? ''}`.trim();

// ── what the page knows ─────────────────────────────────────────────────────────────────────────────────────────────────────────
/** @typedef {{ id: string, name: string }} Calendar */
/** @typedef {{ id: string, calendar_id: string, delivered: number, last_at_ms: number | null, last_failed: boolean }} Source */
/** @type {{ ok: boolean, can_create: boolean, calendars: Calendar[], error?: string, remedy?: string } | null} null while it is being read */
let google = null;
/** @type {{ ok: boolean, folder_ok: boolean, served: any[], unserved: any[], sources: Source[], trend: any, error?: string } | null} */
let profiles = null;
/** The count that filters the rows now, or null. @type {string | null} */
let filterKey = null;
let view = 'lines';

const page = must('[data-cal-page]');
const strip = must('[data-cal-kpis]');
const line = must('[data-cal-filter-line]');
const band = must('[data-cal-attention]');
const tableWrap = must('[data-cal-table]');
const tbody = /** @type {HTMLElement} */ (tableWrap.querySelector('tbody'));
const profileCard = must('[data-cal-profiles]');
const problemsBox = must('[data-cal-problems]', profileCard);
const allServed = must('[data-cal-allserved]', profileCard);
const servedWrap = must('[data-cal-served]', profileCard);
const servedBody = /** @type {HTMLElement} */ (servedWrap.querySelector('tbody'));
const usesCard = must('[data-cal-section="uses"]');
const lines = must('[data-cal-lines]', usesCard);
const graph = must('[data-cal-graph]', usesCard);
const switcher = must('[data-cal-view-switch]', usesCard);
const menuWrap = must('[data-cal-more]');

// The uses map is drawn by script, so its loading state is the first thing this module does.
setMatrix(lines, { view: 'lines', rows: [], columns: [], state: 'loading', skeleton: 3 });

await new Promise((resolve) => {
    const done = () => document.documentElement.hasAttribute('data-kp-auto-ready');
    if (done()) return resolve(undefined);
    new MutationObserver((_, watch) => {
        if (done()) {
            watch.disconnect();
            resolve(undefined);
        }
    }).observe(document.documentElement, { attributes: true, attributeFilter: ['data-kp-auto-ready'] });
});

const table = /** @type {import('/static/kp/js/datatable.js').DataTableHandle} */ (dataTable(tableWrap));
const servedTable = dataTable(servedWrap);

// ── derived ─────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
const sources = () => profiles?.sources ?? [];
/** @param {string} calendarId */
const sourcesOf = (calendarId) => sources().filter((s) => s.calendar_id === calendarId);
/** @param {string} calendarId */
function calName(calendarId) {
    if (!google) return '…';
    return google.calendars.find((c) => c.id === calendarId)?.name ?? 'A calendar Almanac cannot see';
}

/** The words a hover card says about a source. @param {Source} s */
function sourceCardText(s) {
    if (!s.delivered) return `${s.id}: nothing delivered since Almanac started.${s.last_failed ? ' The latest delivery failed.' : ''}`;
    const last = s.last_at_ms ? ` Last delivery ${when(s.last_at_ms)}.` : '';
    return `${s.id}: ${plural(s.delivered, 'event', 'events')} delivered recently.${last}${s.last_failed ? ' The latest delivery failed.' : ''}`;
}

// ── 1 · the counts ──────────────────────────────────────────────────────────────────────────────────────────────────────────────
const waiting = (/** @type {string} */ label) =>
    `<div class="kp-kpi" aria-busy="true"><span class="kp-kpi__label">${label}</span><span class="kp-kpi__value"><span class="kp-skeleton cal-skeleton-value"></span></span><span class="kp-kpi__trend"><span class="kp-skeleton cal-skeleton-line"></span></span></div>`;

const toggleTile = (/** @type {string} */ key, /** @type {string} */ label, /** @type {number} */ value, /** @type {string} */ trend, tone = '') =>
    `<button type="button" class="kp-kpi kp-kpi--toggle" aria-pressed="${filterKey === key}" data-cal-kpi="${key}"${tone ? ` data-kp-tone="${tone}"` : ''}><span class="kp-kpi__label">${label}</span><span class="kp-kpi__hint" aria-hidden="true"><span data-kp-when="off">Filter</span><span data-kp-when="on">Filtering ×</span></span><span class="kp-kpi__value">${value}</span><span class="kp-kpi__trend">${esc(trend)}</span></button>`;

/** The fourth tile: deliveries per hour, from the route ring (the latest deliveries since Almanac started). */
function trendTile() {
    const t = /** @type {any} */ (profiles).trend;
    const note = t.truncated ? 'per hour, latest 100 deliveries' : 'per hour';
    const busiest = t.busiest ? `Busiest hour ${plural(t.busiest.count, 'event', 'events')} at ${clock.format(t.busiest.hour_ms)}:00` : 'Nothing delivered yet';
    const data = JSON.stringify({ unit: 'events', digits: 0, series: [{ label: 'Events delivered', start: t.start_ms, step: t.step_ms, values: t.values }] });
    const chart = t.total
        ? `<figure class="kp-kpi__chart" data-kp-chart="spark" data-kp-spark-head="none" data-kp-spark-axis="relative" aria-label="Events delivered per hour, last 24 hours"><script type="application/json" data-kp-chart-data>${data}</script></figure>`
        : '';
    return `<div class="kp-kpi kp-kpi--trend"><span class="kp-kpi__label">Events delivered <span class="kp-kpi__label-note">${note}</span></span><span class="kp-kpi__value">${t.total}<small>last 24 h</small></span><span class="kp-kpi__trend">${busiest}</span>${chart}</div>`;
}

/** What each count shows when it filters. */
function filterWords(/** @type {string | null} */ key) {
    const calendars = google?.calendars.length ?? 0;
    const files = profiles?.unserved.length ?? 0;
    if (key === 'calendars') return `Showing the ${plural(calendars, 'calendar', 'calendars')} only.`;
    if (key === 'sources') return `Showing the ${plural(sources().length, 'source', 'sources')} only.`;
    if (key === 'unserved') return `Showing the ${plural(files, 'profile file that is not served', 'profile files that are not served')} only.`;
    return 'Showing every row. Press a count to show only the rows it counts.';
}

function applyFilter() {
    if (filterKey) page.setAttribute('data-cal-filter', filterKey);
    else page.removeAttribute('data-cal-filter');
    line.textContent = filterWords(filterKey);
}

function renderStrip() {
    strip.removeAttribute('aria-busy');
    if (!google || !profiles) {
        strip.setAttribute('aria-busy', 'true');
        strip.innerHTML = waiting('Calendars') + waiting('Sources') + waiting('Profile files not served') + waiting('Events delivered');
        line.textContent = 'Reading the counts.';
        return;
    }
    if (!google.ok || !profiles.ok) {
        strip.replaceChildren(
            statePlate(document, {
                kind: 'error',
                title: 'The counts could not be read',
                text: 'Almanac could not ask Google or read the profiles folder.',
                actionLabel: 'Try again',
                onAction: () => void load(),
            }),
        );
        line.textContent = 'Nothing to filter.';
        return;
    }
    const calendars = google.calendars;
    const unusedCalendars = calendars.filter((c) => !sourcesOf(c.id).length).length;
    const silent = sources().filter((s) => !s.delivered).length;
    const files = profiles.unserved;
    strip.innerHTML = [
        toggleTile(
            'calendars',
            'Calendars',
            calendars.length,
            !calendars.length ? 'Make the first one' : unusedCalendars ? `${unusedCalendars} ${unusedCalendars === 1 ? 'is' : 'are'} used by no source` : 'Every calendar is used',
        ),
        toggleTile('sources', 'Sources', sources().length, !sources().length ? 'Add one on the Sources page' : silent ? `${silent} delivered nothing since the start` : 'Every source has delivered'),
        toggleTile('unserved', 'Profile files not served', files.length, files.length ? files[0].file : 'Every profile file is served', files.length ? 'warning' : ''),
        trendTile(),
    ].join('');
    attachKpiToggles(strip);
    attachCharts(strip);
    applyFilter();
}

// A count is a toggle that filters the rows it counts: one at a time, and the line under the strip says which rows are shown.
strip.addEventListener('click', (event) => {
    const tile = event.target instanceof Element ? /** @type {HTMLElement | null} */ (event.target.closest('[data-cal-kpi]')) : null;
    if (!tile) return;
    setTimeout(() => {
        const on = tile.getAttribute('aria-pressed') === 'true';
        for (const other of strip.querySelectorAll('[data-cal-kpi]')) if (other !== tile) other.setAttribute('aria-pressed', 'false');
        filterKey = on ? tile.getAttribute('data-cal-kpi') : null;
        applyFilter();
    }, 0);
});

// ── 2 · the attention band ──────────────────────────────────────────────────────────────────────────────────────────────────────
const fix = (/** @type {string} */ key, /** @type {string} */ label) => {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'kp-button kp-button--sm';
    b.setAttribute('data-cal-fix', key);
    b.textContent = label;
    return b;
};

function renderAttention() {
    /** @type {import('/static/kp/js/attention.js').AttentionItem[]} */
    const items = [];
    if (google && !google.ok) {
        items.push({
            key: 'google',
            severity: 'critical',
            title: 'Google Calendar cannot be reached',
            text: 'Calendars cannot be listed or made, and new events wait in the journal until Google answers again.',
            action: fix('google', 'Check again'),
        });
    }
    const files = profiles?.unserved ?? [];
    if (files.length) {
        const first = files.length === 1 ? files[0].file : `${files[0].file} and ${files.length - 1} more`;
        items.push({
            key: 'file',
            severity: 'warning',
            title: `${plural(files.length, 'profile file is', 'profile files are')} not being served`,
            text: `${first}: the ${files.length === 1 ? 'source it describes receives' : 'sources they describe receive'} nothing, and a post to ${files.length === 1 ? 'it' : 'them'} answers 401.`,
            action: fix('file', 'Reload profiles from disk'),
        });
    }
    setAttention(band, items);
}

/** Reloads the profile files; resolves with what the server says. */
const reloadFiles = () => /** @type {Promise<{ served: number, unserved: number }>} */ (api('/sources/reload', {}));

band.addEventListener('click', (event) => {
    const button = event.target instanceof Element ? /** @type {HTMLElement | null} */ (event.target.closest('[data-cal-fix]')) : null;
    if (!button) return;
    const key = button.getAttribute('data-cal-fix');
    /** @type {(() => void) | null} */
    let after = null;
    runFlashButton(
        button,
        async () => {
            if (key === 'google') {
                await loadGoogle(true);
                if (!google?.ok) return refusal(google?.error ?? 'Google did not answer.', google?.remedy ?? 'Try again in a moment.');
                after = () => flavour('success', 'Google Calendar answers again; the waiting events are being delivered.');
                return 'Done';
            }
            const before = { served: profiles?.served.length ?? 0, unserved: profiles?.unserved.length ?? 0 };
            /** @type {{ served: number, unserved: number }} */
            let now;
            try {
                now = await reloadFiles();
            } catch (e) {
                return /** @type {any} */ (e);
            }
            await loadProfiles(true);
            if (!now.unserved) {
                after = () => flavour('success', `Profile files read again: ${now.served} served, 0 not served.`);
                return 'Done';
            }
            if (now.unserved === before.unserved && now.served === before.served) {
                after = () => flavour('info', 'Reloaded: the same files are served, nothing was changed.');
                return { word: 'Same' };
            }
            after = () => flavour('warning', `Reloaded, but ${plural(now.unserved, 'profile file is', 'profile files are')} still not served.`);
            return { word: 'Partly' };
        },
        { refusal: 'toast', busyLabel: key === 'google' ? 'Checking' : 'Reloading', width: 'keep' },
    ).then(() => after?.());
});

// "Reload profiles from disk" in the header menu: the same call, and the page stays where it is.
menuWrap.addEventListener('kp-menu-select', async () => {
    try {
        const now = await reloadFiles();
        await loadProfiles(true);
        flavour('success', `Profile files read again: ${now.served} served, ${now.unserved} not served.`);
    } catch (e) {
        flavour('error', said(/** @type {any} */ (e)));
    }
});

// ── 3 · the calendars table ─────────────────────────────────────────────────────────────────────────────────────────────────────
const slot = must('[data-kp-datatable-empty]', tableWrap);
{
    const plate = statePlate(document, {
        kind: 'empty',
        title: 'No calendars yet',
        text: 'The service account sees none. Make the first one and Almanac shares it with you.',
        actionLabel: 'New calendar',
        onAction: () => openNew(),
    });
    plate.setAttribute('data-kp-art', 'none');
    slot.className = plate.className;
    slot.setAttribute('data-kp-art', 'none');
    slot.replaceChildren(...plate.childNodes);
    attachArt(slot.parentElement ?? document);
}

/** @param {Calendar} c */
function calendarRow(c) {
    const used = sourcesOf(c.id);
    const tags = used.length
        ? `<ul class="kp-tag-list" aria-label="Sources that write to ${esc(c.name)}">${used.map((s) => `<li class="kp-tag" tabindex="0" data-kp-hovercard="${esc(sourceCardText(s))}">${esc(s.id)}</li>`).join('')}</ul>`
        : '<span class="kp-text-muted">No source writes here</span>';
    const base = `type="button" class="kp-button kp-button--destructive kp-button--sm" data-kp-destructive data-kp-confirm-owner data-cal-delete="${esc(c.id)}"`;
    const names = used.map((s) => s.id).join(', ');
    const del = used.length
        ? `<button ${base} disabled data-kp-disabled-reason="Used by ${esc(names)}. Delete ${used.length === 1 ? 'that source' : 'those sources'} first, on the Sources page.">Delete</button>`
        : `<button ${base}>Delete</button>`;
    return `<tr data-kp-row-key="${esc(c.id)}">
<td data-label="Name" data-kp-sort-value="${esc(c.name)}">${esc(c.name)}</td>
<td data-label="Calendar id"><figure class="kp-code kp-code--cell" data-kp-code aria-label="Calendar id of ${esc(c.name)}"><pre class="kp-code__body"><code>${esc(c.id)}</code></pre></figure></td>
<td data-label="Used by" data-kp-sort-value="${used.length}">${tags}</td>
<td data-label="Actions" class="kp-text-end"><span class="kp-row-actions">${del}</span></td>
</tr>`;
}

function renderTable() {
    if (!google) {
        table.busy({ text: 'Reading the calendars', overlay: true });
        table.state('loading');
    } else if (!google.ok) {
        table.busy(null);
        table.fail(google.error);
    } else {
        tbody.innerHTML = google.calendars.map(calendarRow).join('\n');
        attachDisabledReasons(tbody);
        attachCodes(tbody);
        table.refresh();
        table.busy(null);
        table.state('ready');
    }
}

tableWrap.addEventListener('kp-datatable-retry', () => void loadGoogle());

// ── 4 · the typed delete ────────────────────────────────────────────────────────────────────────────────────────────────────────
{
    const dialog = /** @type {HTMLDialogElement} */ (must('[data-cal-delete-dialog]'));
    const blockEl = must('[data-cal-type-block]', dialog);
    const go = /** @type {HTMLButtonElement} */ (must('[data-cal-del-go]', dialog));
    const result = must('[data-cal-result]', dialog);
    const title = must('.kp-dialog__title', dialog);
    /** @type {Calendar | null} */
    let target = null;

    tbody.addEventListener('click', (event) => {
        const button = event.target instanceof Element ? /** @type {HTMLButtonElement | null} */ (event.target.closest('[data-cal-delete]')) : null;
        if (!button || button.disabled || !google) return;
        target = google.calendars.find((c) => c.id === button.getAttribute('data-cal-delete')) ?? null;
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
    });
    must('[data-cal-del-cancel]', dialog).addEventListener('click', () => dialog.close());

    go.addEventListener('click', () => {
        const c = target;
        if (!c) return;
        /** @type {{ error: string, remedy?: string } | null} */
        let refused = null;
        runFlashButton(
            go,
            async () => {
                try {
                    await api('/calendars/remove', { id: c.id });
                } catch (e) {
                    refused = /** @type {any} */ (e);
                    // the set-up changed under the page (a source started writing here): show it as it is now
                    void load(true);
                    return /** @type {any} */ (e);
                }
                result.textContent = `Deleted ${c.name}.`;
                return { word: 'Deleted' };
            },
            { refusal: 'button', busyLabel: 'Deleting' },
        ).then((outcome) => {
            if (refused) return void flavour('destructive', `${c.name} was not deleted: ${said(refused)}`);
            if (!outcome?.ok || !google) return;
            google.calendars = google.calendars.filter((x) => x.id !== c.id);
            render();
            flavour('success', `Deleted ${c.name} and every event on it.`);
            setTimeout(() => dialog.close(), 600);
        });
    });
}

// ── 5 · the new calendar dialog ─────────────────────────────────────────────────────────────────────────────────────────────────
const newDialog = /** @type {HTMLDialogElement} */ (must('[data-cal-new-dialog]'));
function openNew() {
    const f = /** @type {HTMLFormElement} */ (must('[data-cal-new-form]', newDialog));
    const name = /** @type {HTMLInputElement} */ (must('input[name="name"]', f));
    const goBtn = /** @type {HTMLButtonElement} */ (must('[data-cal-new-go]', f));
    const sharing = must('[data-cal-sharing]', f);
    const canCreate = google?.can_create !== false;
    name.disabled = !canCreate;
    goBtn.disabled = !canCreate;
    if (!canCreate) {
        sharing.textContent = 'Almanac cannot make calendars: ALMANAC_CALENDAR_OWNER is not set, so a new calendar would belong to the service account and be visible to nobody.';
    }
    newDialog.showModal();
}
{
    const f = /** @type {HTMLFormElement} */ (must('[data-cal-new-form]', newDialog));
    const name = /** @type {HTMLInputElement} */ (must('input[name="name"]', f));
    formOf(f);
    f.addEventListener(VALID_EVENT, async (event) => {
        const detail = /** @type {CustomEvent} */ (event).detail;
        const typed = name.value.trim();
        try {
            const made = await api('/calendars/make', { name: typed });
            if (google?.ok && !google.calendars.some((c) => c.id === made.id)) {
                google.calendars = [...google.calendars, { id: made.id, name: made.name }].sort((a, b) => a.name.localeCompare(b.name));
                render();
            }
            flavour('success', `Made ${made.name} and shared it with you.`);
            name.value = '';
            newDialog.close();
        } catch (e) {
            flavour('error', said(/** @type {any} */ (e)));
        }
        detail.done();
    });
    f.addEventListener(INVALID_EVENT, () => name.focus({ preventScroll: true }));
    must('[data-cal-new-cancel]', newDialog).addEventListener('click', () => newDialog.close());
    for (const b of document.querySelectorAll('[data-cal-open-new]')) b.addEventListener('click', openNew);
    attachDisabledReasons(newDialog);
}

// ── 6 · the profile files ───────────────────────────────────────────────────────────────────────────────────────────────────────
/** The what-is-lost list of a file's typed delete, kept as a template the button names. @param {{ file: string }} u */
function losesFor(u) {
    const id = `cal-lose-file-${u.file.replace(/[^a-z0-9]/gi, '-')}`;
    if (!document.getElementById(id)) {
        const t = document.createElement('template');
        t.id = id;
        t.innerHTML = `<ul class="kp-type-confirm__list"><li>The file ${esc(u.file)} from the profiles folder</li><li>Nothing else: it is not served, so no source uses it</li></ul>`;
        profileCard.append(t);
    }
    return id;
}

function renderProfiles() {
    const p = profiles;
    if (!p) return setCardState(profileCard, 'loading', { words: 'Reading the profile files' });
    if (!p.ok || !p.folder_ok) {
        return setCardState(profileCard, 'error', { title: 'The profiles folder could not be read', text: 'Almanac has no access to it right now.' });
    }
    if (!p.served.length && !p.unserved.length) {
        const add = document.createElement('a');
        add.className = 'kp-button kp-button--sm kp-button--primary';
        add.href = '/clients';
        add.textContent = 'Add a source';
        return setCardState(profileCard, 'empty', { title: 'No profile files yet', text: 'Add a source on the Sources page and Almanac writes its profile here.', action: add });
    }
    setCardState(profileCard, 'ready');
    setProblems(
        problemsBox,
        p.unserved.map((u) => {
            const b = document.createElement('button');
            b.type = 'button';
            b.className = 'kp-button kp-button--destructive kp-button--sm';
            b.setAttribute('data-kp-destructive', '');
            b.setAttribute('data-kp-confirm', `Delete the file ${u.file}?`);
            b.setAttribute('data-kp-confirm-type', u.file);
            b.setAttribute('data-kp-confirm-loses', losesFor(u));
            b.setAttribute('data-kp-confirm-label', 'Delete file');
            b.setAttribute('data-cal-delete-file', u.file);
            b.textContent = 'Delete the file…';
            return { key: u.file, what: `${u.file} is not being served`, why: u.why, remedy: u.remedy, severity: /** @type {const} */ ('warning'), action: b };
        }),
    );
    allServed.hidden = p.unserved.length > 0;
    servedWrap.hidden = !p.served.length;
    servedBody.innerHTML = p.served
        .map((s) => {
            const warn = s.health === 'warn';
            return `<tr data-kp-row-key="${esc(s.file)}">
<td data-label="File"><code class="kp-mono">${esc(s.file)}</code></td>
<td data-label="Source">${esc(s.source)}</td>
<td data-label="Calendar">${esc(calName(s.calendar_id))}</td>
<td data-label="Health" data-kp-sort-value="${warn ? 1 : 0}"><span class="kp-health" data-state="${warn ? 'warn' : 'ok'}"><span class="kp-health__dot" aria-hidden="true"></span> ${warn ? 'Served, latest delivery failed' : 'Served'}</span></td>
</tr>`;
        })
        .join('\n');
    servedTable?.refresh();
}

profileCard.addEventListener('kp-card-retry', () => void loadProfiles());

problemsBox.addEventListener('click', (event) => {
    const button = event.target instanceof Element ? /** @type {HTMLElement | null} */ (event.target.closest('[data-cal-delete-file]')) : null;
    if (!button) return;
    const file = button.getAttribute('data-cal-delete-file') ?? '';
    runFlashButton(
        button,
        async () => {
            try {
                await api('/profiles/remove', { file });
            } catch (e) {
                return /** @type {any} */ (e);
            }
            return { word: 'Deleted' };
        },
        { refusal: 'toast', busyLabel: 'Deleting' },
    ).then((result) => {
        if (!result?.ok || !profiles) return;
        profiles.unserved = profiles.unserved.filter((u) => u.file !== file);
        render();
        toast(`Deleted ${file}.`, { className: 'kp-toast--success' });
    });
});

// ── 7 · the uses map ────────────────────────────────────────────────────────────────────────────────────────────────────────────
function renderUses() {
    if (!google || !profiles) {
        setMatrix(lines, { view: 'lines', rows: [], columns: [], state: 'loading', skeleton: 3 });
        setGraphState(graph, 'loading', 'Reading the set-up');
        return;
    }
    const ids = new Set(google.calendars.map((c) => c.id));
    const writing = sources().filter((s) => ids.has(s.calendar_id));
    if (!google.ok || !profiles.ok) {
        setMatrix(lines, { view: 'lines', rows: [], columns: [], state: 'error', message: 'The set-up could not be read' });
        setGraphState(graph, 'error', 'The set-up could not be read');
        return;
    }
    if (!writing.length) {
        setMatrix(lines, { view: 'lines', rows: [], columns: [], state: 'empty', message: 'No source writes to a calendar yet' });
        setGraphState(graph, 'empty', 'No source writes to a calendar yet');
        return;
    }
    setMatrix(lines, {
        corner: 'Calendar ↓ source →',
        rows: google.calendars.map((c) => ({ key: c.id, label: c.name })),
        columns: writing.map((s) => ({ key: s.id, label: s.id })),
        label: 'Which source writes to which calendar',
        cell: (row, column) => (writing.find((s) => s.id === column)?.calendar_id === row ? { tone: 'ok', label: 'writes here' } : { tone: 'none' }),
        countLabel: (n) => `${n} ${n === 1 ? 'source' : 'sources'}`,
        sourcesLabel: 'Calendars',
        usersLabel: 'Sources',
        view: 'lines',
    });
    const hub = [...google.calendars].sort((a, b) => sourcesOf(b.id).length - sourcesOf(a.id).length)[0];
    const draw = () =>
        setGraphData(graph, {
            hub: hub?.id,
            nodes: [
                ...google.calendars.map((c) => ({ id: c.id, label: c.name, kind: 'calendar', description: `Written by ${sourcesOf(c.id).map((s) => s.id).join(', ') || 'no source'}` })),
                ...writing.map((s) => ({ id: s.id, label: s.id, kind: 'source', description: sourceCardText(s) })),
            ],
            edges: writing.map((s) => ({ from: s.id, to: s.calendar_id, kind: 'writes', detail: `${plural(s.delivered, 'event', 'events')} delivered recently` })),
            kinds: [{ kind: 'writes', label: 'Writes to', hint: 'A source posts its events into this calendar', style: 'solid', colour: 'var(--chart-1)' }],
        });
    // A graph drawn while hidden has no size: it is drawn when it can be measured.
    if (view === 'graph') requestAnimationFrame(draw);
    else draw();
}

function showView(/** @type {string} */ next) {
    view = next;
    lines.hidden = next !== 'lines';
    graph.hidden = next !== 'graph';
    setSegmentedValues(switcher, [next]);
    if (next === 'graph') renderUses();
}
switcher.addEventListener('kp-segmented-change', (event) => showView(String(/** @type {CustomEvent} */ (event).detail.value)));
lines.addEventListener('kp-uses-retry', () => void load());

// ── loading ─────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
function render() {
    renderStrip();
    renderAttention();
    renderTable();
    renderProfiles();
    renderUses();
}

/** @param {boolean} [quiet] keep what is shown while the answer comes (a fix button is running, or a refusal is being checked) */
async function loadGoogle(quiet = false) {
    if (!quiet) {
        google = null;
        render();
    }
    try {
        google = await api('/calendars/data/google');
    } catch (e) {
        const err = /** @type {any} */ (e);
        google = { ok: false, can_create: true, calendars: [], error: err.error, remedy: err.remedy };
    }
    render();
}

/** @param {boolean} [quiet] as in loadGoogle */
async function loadProfiles(quiet = false) {
    if (!quiet) {
        profiles = null;
        render();
    }
    try {
        profiles = { ok: true, .../** @type {any} */ (await api('/calendars/data/profiles')) };
    } catch (e) {
        const err = /** @type {any} */ (e);
        profiles = { ok: false, folder_ok: false, served: [], unserved: [], sources: [], trend: null, error: err.error };
    }
    render();
}

/** Asks for everything at once; each card fills as its own answer arrives. */
function load(quiet = false) {
    return Promise.all([loadGoogle(quiet), loadProfiles(quiet)]);
}

const boot = JSON.parse(must('#cal-boot').textContent ?? '{}');
if (boot.notice) flavour('error', boot.notice);
showView('lines');
await load();
