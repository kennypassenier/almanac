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

## Workarounds to drop in the next kp-themes / chassis-rs cycle

Almanac runs on chassis-rs 3.9.0 (kp-themes 11.1.1) since 4.4.0. These page
workarounds stand in for package or kit gaps queued in
`~/Projects/kp-gaps/almanac.md`; drop each one when its fix ships and
measure again (gaps, align and states; formal and dark; 1280 and 390):

1. `holdTableFootprint()` in `web/js/status.js`: the deliveries table's
   failed and empty height (gaps 24, 26, 27).
2. The attention band's form-grid tracks in `web/css/calendars.css`
   (`.cal-attention-grid > .kp-attention[data-kp-subgrid]`, gap 21; 11.1.x
   names icon | words | actions tracks, so check whether the package's own
   tracks are enough).
3. `.cal-page .kp-dialog__section` inline padding in `web/css/calendars.css`
   (gap 12).
4. The empty and failed table height `min-block-size: 30rem` in
   `web/css/calendars.css` (gap 7).
5. `white-space: nowrap` on buttons in narrow cells: Copy in
   `web/css/try.css` (gap 15), the fix buttons in `web/css/calendars.css`,
   Try again in `web/css/history.css` (gap 7).
6. The phone-width aside rule beside `--kp-calendar-aside: 30rem` in
   `web/css/history.css` (gap 14), and the now-empty `.hist-month` wrapper
   in `templates/history.html`.

Kit wishes for chassis-rs (no blocker): an html-only section body without
the empty `dl[data-chassis-rows]`, and a hook so `settleCards` does not set
an html section back to ready (status.js's `hold()` exists for it).
