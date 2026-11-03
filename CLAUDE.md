# Almanac

A Rust hub that receives events from other systems (task managers, home
automation, monitoring, AI sessions) and translates them into calendar
entries across purpose-specific Google Calendars — a single readable
plan-and-log for the household and the homelab.

This project follows the dev procedure in `~/Projects/dev-procedure/`
(`/project-flow`). Standing rules apply to every change:
`~/Projects/dev-procedure/STANDING_RULES.md`.
Enforcement is **git-native** (`.githooks/` via `core.hooksPath`), so
gates hold from any session or terminal. After a fresh clone, run:
`git config core.hooksPath .githooks`. (Not yet installed — lands in
Phase 5.)

## Procedure status

| Field | Value |
|---|---|
| Current phase | done — **v4.0.5 tagged, signed and confirmed live on CT 112, 2026-09-10** (chassis-rs 2.0.2, `d199ead`): Homelab Rust verified `/healthz` reports `{"status":"ok","version":"4.0.5"}` and `ldd` on the installed binary says `statically linked` — not assumed. No public-API change in 2.0.2 over 2.0.0 (823 items unchanged); `chassis sync` left the shared hooks alone this time instead of reverting them (CF-16, chassis-rs 2.0.1). Chain run twice this session on 2.0.0: `Client::adopted(...)` replaces the struct literal (`Client` is `#[non_exhaustive]`), the Sources page's own calendar column renamed **Calendar name**, and the full scaffold sync (static musl build on `gcr.io/distroless/static`, CI back to every-branch triggers, kp-themes 5.1.0). **Correction (2026-09-10): v4.0.3 and v4.0.4 were in fact installed on CT 112 by Homelab Rust, on their own initiative, before this table was last updated** — the earlier claim that 4.0.2 was "still live" and that 4.0.3 was "never installed anywhere" was stale, caught only when Homelab Rust checked the live endpoint instead of trusting what this table said. |
| Last completed gate | the maintenance form of 2026-09-26, answered 2026-09-27: release 4.0.6 **Later**, latch push done by Claude (secrets `bb23888`) |
| Next gate | none pressing. Release 4.0.6 is deferred by Kenny (Later); `main` carries chassis-rs 2.2.1 (kp-themes 7.2.0, adopted 2026-09-27), the sync endpoint's delivery count, the rustls RUSTSEC-2026-0285 bump and the `backoff` → `backon` swap past v4.0.5, CHANGELOG ready. Signing needs the minisign key on WSL (workstation `GARUDA.md` step 1) |
| Next action | **Kenny chose 7.0.0 (form release-700, 2026-11-03 06:20); the started release was stopped in the live test on Kenny's order (06:21): every child waits for chassis-rs 4.5.1. Then bump, and post a fresh release form.** main 33e7970 on chassis-rs 4.5.0 (pages on kp dist/js/index.js + kp_modules); chassis 4.5.1 (main 81227fb, after Kenny's go there) fixes the bundle hang in the kit: bump to 4.5.1 before releasing once it is out. **On hold (Kenny 2026-11-03 03:37) until the next chassis-rs release (4.5.0)**: bump WIP d3120e6 on chassis-420-prep (pins v4.4.0, History uses data-kp-date-nav); the Firefox page hang on 4.4.0 is chassis' stale-import guard, fixed on chassis main 60e72ad for 4.5.0 (test workaround `window.__chassisStaleImportGuard = true`); re-check after the 4.5.0 bump, then the release form. **Journal in the database (Kenny, form journal-store 2026-11-03, "Eén rij per event"; legacy imports "Laten staan")**: built on `chassis-420-prep` (e0ac280 code, 4b00b2d docs, AR16 amended): `journal` table in almanac.db, one-time import of journal.jsonl with .pre-db copy, file cap/compaction gone; measured on a copied state folder. Ships with the chassis-rs bump, no release now. **Review round 2026-11-02 on main** (04c0695 fixes + f3358d7 docs, local, no release; targeted tests green); form "almanac-review-keuzes" answered 2026-11-02 03:42 (all recommendations, key-namespace Opnemen) and built (fix-review-1102 on main, targeted tests green; accessibility fixes in the same commit). **No release now (Kenny, 2026-11-01 22:35):** kp-themes first, then chassis-rs, then almanac moves to the new chassis-rs and only then releases (6.1.0 changelog is under [Unreleased]; a started 6.1.0 release was stopped before tagging); bump checklist for kp-themes 12.2.0 in ~/Projects/kp-gaps/almanac.md under gap 36: drop the history.css stopgaps for gaps 34/35, adopt `data-kp-date-nav`; PWA: prepared on branch `chassis-420-prep` (48cb331, against chassis main 20223c1: only `Pwa::start_url("/")`; the kit layout carries tags and worker, so never add `pwa_page_head()` to layout pages; one manifest and one worker script on all 8 pages, worker registered from `/`, measured in Firefox) — at the bump, rebase it and swap the rev for the 4.2.0 tag). **UX round of 2026-11-01 built on main** (all 16 choices from ~/Projects/ux-review/almanac.md, last commit `bb116e3`, local, not pushed, no release; suite not run, only targeted tests: journal 13+30, deliveries 1, web embed 1; Firefox smoke of 8 pages 0 errors; kp gaps 33–36 queued in ~/Projects/kp-gaps/almanac.md). Also waiting on Kenny: his go for the install of **almanac:v6.0.0** on CT 112, **signed 2026-10-31** (SHA256SUMS.minisig verified against the release key, trusted comment "kennypassenier/almanac v6.0.0", binary checksum OK, marked latest). Published 2026-10-31, commit 481d870, chassis-rs 4.0.0 / kp-themes 12 bundle; gate 255 passed, 4 skipped; live test 4 passed; exception RUSTSEC-2026-0387 by Kenny's go). 5.0.0 was never installed, so 6.0.0 replaces it; then the install on CT 112. **main carries the engineering-rules split** (`65d8e11`, 2026-10-30, local, not pushed, no release): 0 files over 250 lines; suite not run yet (259 tests, all binaries build); at that install the live profiles on CT 112 need `mark = "HA"/"HL"/"CS"/"KY"` (home-assistant, homelab, claude-sessions, kyu) to keep their Deliveries marks, a machine change to ask Kenny for. Same install: add the source `medicine-cabinet` on the Sources page (Huishouden calendar, `mark = "MC"`) and hand its token to Medicine Cabinet's latch env (coordinator, 2026-10-31). Before that, still: the combined signing command at the end of the round, with almanac:v5.0.0 (published 2026-10-22 19:42; chassis-rs 3.13.0, kp-themes 11.2.0; suite 263 passed, 4 skipped; live test 4 passed). Never sign 4.3.0/4.4.0. At the install on CT 112 (one by one, after the signing): Claude sets ALMANAC_TRUSTED_ADMINS=10.10.10.10 in CT 112's latch env (Kenny: Opnemen, 2026-10-22 17:15), then checks /healthz and that history.json became history.json.migrated and almanac.db holds the rows. Open: 11 queued package gaps still open (2 visible, both worked around) in ~/Projects/kp-gaps/almanac.md, remaining workarounds in docs/KP_THEMES_11_ADOPTION.md, the kp-themes verdict files for research/almanac-* not written |
| AFK mode | off since 2026-08-28 |
| Updates | **the homelab owns them** since 2026-08-30. `ALMANAC_SELF_UPDATE=off` on CT 112; `stacks/almanac/service.yml` carries `update_cmd: runuser -u almanac -- /opt/almanac/almanac update`. Exactly one of the two may ever be armed |
| Open, gated on Kenny | nothing (4.0.6 signed and live 2026-09-27). Blocked elsewhere: `ALMANAC_TRUSTED_PROXIES` on CT 112 (Homelab Rust). Release path (2026-09-29): `make release VERSION=x.y.z` → `chassis release`, built and published locally, no GitHub Actions; needs chassis-rs >= 3.0.0 — the pin is older, so `chassis upgrade 3.0.0` + `chassis sync --write` come first once the kit is released |

**Live since 2026-08-29:** CT 112 on Proxmox, `10.10.10.12:8080`, systemd
under `latch run`, self-update armed against GitHub Releases. Two real
calendars — Almanac · Huishouden and Almanac · Infra — created by the
service account and shared with Kenny. The full chain is proven on that
machine: an event created, redelivered without duplicating, and deleted.
Real calendar ids live only in the deployment, never in this repository.

**M13 (Prometheus metrics)** is built and serving: `/metrics` on
`10.10.10.12:8080`, unauthenticated like `/healthz`, six `almanac_`
series plus `almanac_build_info`. The homelab's Prometheus on CT 113
can uncomment its scrape job.

**Both drills passed on 2026-08-29.** The reboot drill (hard power
cut) replayed an undelivered event and fired AR21's startup retry.
The self-update drill went 0.1.2 → 0.1.3 over the air in five
minutes — and found a real bug first: the six-hour check interval
was scheduled from process start rather than from the end of the
startup delay, so the first check landed six hours out while every
unit test passed. Fixed in 0.1.2, proven on hardware, and a check
now logs a line either way so a silently dead updater is visible.

Per standing rule 19: work happens in a session opened in this project
directory (`~/Projects/almanac`). L0 is done: renamed to almanac
throughout, hooks/CI live and proven (a bad commit was physically
blocked), `src/core`/`src/shell` split in place per AR13.

<!-- Update this block after every completed gate. -->

## Project documents

| Doc | Purpose |
|---|---|
| docs/SCOPE.md | goals, non-goals, success criteria, constraints (Phase 0) — done |
| docs/USER_GUIDE.md | how to connect a source, shape events, update and delete them (Phase 8) |
| docs/DEBUGGING_GUIDE.md | the evidence trail and symptom→cause tables (Phase 8) |
| docs/ARCHITECTURE_REFERENCE.md | the system as built (Phase 8) |
| docs/legacy/ | INVENTORY.md and AFK_QUEUE.md — history, not maintained |
| docs/FEATURES.md | rated feature list with permanent IDs (Phase 2) — done, frozen, 24 features (M10, M11 added via mini-rounds) |
| docs/ARCHITECTURE_DECISIONS.md | frozen AR decisions incl. tech choice (Phases 3-4) — done, AR1–AR19 frozen |
| docs/REALIZATION_PLAN.md | milestones + status table (Phase 5) — done, L0–L5 approved |
| docs/TEST_PLAN.md | what is proven where + accepted limitations (Phase 7) — done |
| docs/OPERATIONS_RUNBOOK.md | releasing, installing, and what to do about each notification |

## History

Renamed from `cal-stacean` on 2026-08-28 (directory and repo only —
internal references like the Cargo package name, binary name, and CI
workflow still say `cal-stacean` pending Phase 1 inventory and later
deliberate rename). Former scope was a Google Calendar gateway with a
hardcoded Vikunja webhook integration; Vikunja is no longer used and
that integration is dropped, though its event-mapping/upsert pattern
is kept as the template for Almanac's general mapping-profile design.
See `docs/SCOPE.md` for the full picture.

## Gates (enforced, live since L0)

Commits are blocked by `.githooks/pre-commit` (`core.hooksPath`,
runs `.claude/hooks/gates.sh`: fmt, clippy -D warnings, tests, AR13
core/shell boundary check) and `.githooks/commit-msg` (requires
bracketed IDs, e.g. `[K5]` or `[meta]`) from any session or terminal.
The Claude Code PreToolUse hook (`.claude/hooks/check-commit.sh`) is a
second layer. There is no GitHub Actions CI: the full gate (plus
cargo-deny, the image smoke test and informational coverage) runs
locally inside `chassis release`, and `make release-dry VERSION=x.y.z`
runs it without releasing. The live suites run only by hand
(`make live-test`); nothing runs them nightly any more.

## When tests run (Kenny, 2026-10-04, test report)

A commit runs fmt and clippy only, and skips them when their input did not
move. The whole suite runs once, at the release (`chassis release`, or
`scripts/release-kit.sh` in chassis-rs), side by side under cargo-nextest
when it is installed; suites that share ports or fixtures are grouped in
`.config/nextest.toml`. When Kenny says a release goes without tests, it
does. A test that waits on the clock gets a short test value instead of the
production one; no test may cost development time it does not need.
