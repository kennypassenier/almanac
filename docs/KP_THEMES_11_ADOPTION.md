# kp-themes 11.0.0 adoption map

Phase 1 of moving Almanac onto kp-themes 11.0.0 (requested by Kenny,
19/10/2026 01:11): every page, which 11.0.0 component and option fits
where, and the new use cases found on the way. Usability and cool factor
decide. Every visual choice below goes to Kenny as a review demo on the
kp-themes Agenda before it is built; nothing here is approved yet.

Today Almanac is pinned to chassis-rs v3.7.1, which embeds kp-themes
10.1.1. The bump itself (phase 2) waits for a chassis-rs release on
kp-themes 11.0.0.

## Who owns which page

| Page | Rendered by | Owner of the change |
|---|---|---|
| Shell (nav, theme menu, log out) | chassis-rs `layout.html` | chassis-rs |
| `/login` | chassis-rs `login.html` | chassis-rs |
| `/` Status: Service, Health, Updates cards, Problems | chassis-rs `status.html` | chassis-rs |
| `/` Status: **Journal** and **Sources** sections | Almanac `src/shell/kit.rs` | Almanac |
| `/clients` Sources (tokens, request log, manage dialog) | chassis-rs `clients.html` + `chassis.js` | chassis-rs |
| `/calendars` | Almanac `templates/calendars.html`, `src/shell/dashboard.rs` | Almanac |

The kit pages are listed here too, so the chassis-rs thread gets
Almanac's wishes; Almanac builds no UI of its own for them.

## Almanac's own surfaces

### Calendars page (`/calendars`)

| Today | 11.0.0 | Why |
|---|---|---|
| `h1` + `.explain` | `.kp-page-header` with primary action "New calendar" and a `data-kp-menu-button` overflow ("Reload profiles from disk") | UI_PRINCIPLES page anatomy: one header row, one primary action |
| nothing | `.kp-kpis` strip: calendars, sources, profile files not served | live page anatomy; counts link to the rows they count |
| `.kp-alert--danger` re-rendered after a failed POST | `.kp-attention` band for standing problems (Google unreachable, broken profile files) with a fix action each; `toast()` for a refused action | one alert per problem, worst first; no full re-render for a one-off refusal |
| plain `kp-table` | `table.kp-datatable[data-kp-datatable]`, `data-kp-remember`, plain-click multi-sort, `-cards` on a phone | tables are datatables |
| `<details>` "id" with inline styles | `.kp-copyable` `.kp-code--cell` | one click copies the id, no inline styles |
| "Used by" as comma text | one `.kp-tag` per source with a `data-kp-hovercard` (token state, last used, uses) | scannable, and the hover answers "which source is that?" |
| delete with `data-kp-confirm` | typed confirm `data-kp-confirm-type="<calendar name>"` with `-confirm-loses` ("every event on it"); when in use, the button stays and is disabled with `data-kp-disabled-reason="Used by …"` | DEMO_RULES 1: destructive confirm needs the exact name typed; same function in the same column on every row |
| create form inline at the bottom | `<dialog class="kp-dialog">` with `.kp-dialog__section`s and `data-kp-form` validation, opened from the header action | rare action leaves the page body |
| italic "no calendars yet" | `.kp-stateplate--empty` with `data-kp-art="none"` and the "New calendar" action | full-footprint empty state |
| Google call on every render, no loading state | page renders at once; the calendars card loads with `data-kp-state="loading"` (busy card) and fills from a JSON route; failed state is the theme's failed plate with Retry | data pages show loading from the first frame; a slow Google no longer blocks the page |
| "Not being served" table | `.kp-problems[data-kp-problems]` (what, why, remedy) with the delete action per file; the served files listed in the same datatable with a `.kp-health` dot | broken files read as problems, not as data |
| nothing | **uses map** `.kp-matrix--lines` (sources × calendars) or the network graph `.kp-graph` | see new use case feat-ui-1 (uses map) |

### Status page, Journal section (`kit.rs:62`)

| Today | 11.0.0 | Why |
|---|---|---|
| "Waiting to be delivered: N" row | `.kp-kpi` with `data-kp-tick`, plus `.kp-health` dot for the journal and the store subsystems | the one number that matters, visible at a glance |
| hand-written table When / Source / Result | `kp-datatable` with `.kp-ago` times, `.kp-tag` source, semantic `.kp-badge` result (created / updated / failed), expandable row with the error in `.kp-code-block` | sortable, filterable by result with `.kp-segmented--chips` + counts |
| nothing | `.kp-stackbar` of created / updated / failed since start | outcome mix at a glance |
| static until reload | `.kp-refresh` chooser + `.kp-ago` freshness line + `.kp-connection` banner, rows updated in place by id (`data-kp-follow` / `.kp-newpill` for new deliveries) | live pages keep state |

### Status page, Sources section (`kit.rs:120`)

| Today | 11.0.0 | Why |
|---|---|---|
| key/value rows source → calendar | `.kp-statuslist--rows` (source, calendar, health) or `.kp-entity-grid` cards | shows health per source, not only the mapping |
| two link buttons in a `<p>` | dropped: the nav already holds both pages | no duplicate navigation |
| "Reload profiles from disk" posting and redirecting to `/calendars` | `data-kp-flash-button` with `data-kp-post="/sources/reload"`, busy → done word, stays on the page, `toast()` with the count reloaded | no surprise page jump |

## Wishes for the kit pages (to chassis-rs)

- Shell: `.kp-tabbar` on a phone (Status, Sources, Calendars) with `setTabbarBadge` showing the exact pending/failed count; `.kp-palette` (Ctrl+K: go to a page, new calendar, reload profiles, new source) with `.kp-shortcuts` chords (`g s`, `g c`); `data-kp-nav-fold`.
- Status: `.kp-ops` frame (`__head`, `__kpis`, `__attention`, `__body`); `.kp-badge--version` version chip for "Latest release" against the running version; `.kp-problems` instead of the alert list; `.kp-uptime` strip for "Up since".
- Sources (`/clients`): `.kp-inspector[data-kp-requests]` replaces the hand-built request log; `.kp-secret[data-kp-secret]` for the token; `.kp-command[data-kp-command="bash"]` for "Copy command"; typed confirm on Delete; calendar `<select>` via `attachSelect` / combobox with `data-kp-usual`; `kp-datatable` for the list; `.kp-dialog__section`s in the manage dialog; `.kp-idmark` per source.
- Login: `.kp-secret` input, `.kp-switch` for "Keep me logged in".

## New use cases

| Id | What | Components | Data source |
|---|---|---|---|
| feat-ui-1 | **Uses map**: which source writes to which calendar, one picture | `.kp-matrix--lines` uses-map, or `.kp-graph` | profiles (in memory) |
| feat-ui-2 | **Try a source**: paste an event, see exactly what Almanac would put in the calendar without writing it | `.kp-jsonfield` with schema, `.kp-trace` (ingest → profile → mapping → calendar), result as `.kp-spec` / `.kp-diff` against the current event | existing `POST /v1/debug/dry-run/{source_id}`, no UI today |
| feat-ui-3 | **Delivery chain**: ingest → journal → worker → Google with live counts and the failing stage lit | `.kp-pipeline` | journal + route ring buffer |
| feat-ui-4 | **Retry schedule**: when each undelivered entry is tried next | `.kp-runs` or `.kp-gantt` | journal + retry/pacing |
| feat-ui-5 | **Deliveries per day**: month heatmap of what Almanac wrote | `.kp-calendar--busy-days` | needs a small daily counter kept on disk |
| feat-ui-6 | **This week in the calendars**: the events Almanac wrote this week, per calendar colour | `.kp-week[data-kp-week]` | Google events of the managed calendars (read only) |
| feat-ui-7 | **Source profile view**: the mapping a source uses, readable | `.kp-code-block` (TOML) + `.kp-kv--columns` summary | profile files |

## Workarounds still in the code (5.0.0, chassis-rs 3.13.0 / kp-themes 11.2.0)

Checked against kp-themes v11.2.0 and chassis-rs v3.13.0 on 2026-10-22:
18 of the 32 entries in `~/Projects/kp-gaps/almanac.md` are fixed in the
packages, and the page CSS that stood in for them is gone (dialog section
padding, the 30rem table height, the phone-card subgrid, the Copy and Try
again `nowrap`). What stays, each for an entry that is still open:

1. `holdTableFootprint()` in `web/js/status.js`: a table driven into empty or
   failed after it loaded still shrinks (gap 32).
2. The phone-width aside rule beside `--kp-calendar-aside` in
   `web/css/history.css` (gap 14).
3. `joinChips()` in `web/js/status.js`: no chips-plus-table filter part
   (gap 11).
4. The attention band's form-grid tracks in `web/css/calendars.css` (gap 21;
   the package's `data-kp-subgrid` may make it unnecessary, not yet tried).
5. In Rust, for chassis-rs gaps 28, 29 and 31: the import hook split as
   `import_from(tx, dir)`, the hard-coded `almanac-kit.db`, and the 400-day
   rule built with `format!`.
