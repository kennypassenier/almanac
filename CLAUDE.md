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
| Current phase | 9 · Release & lifecycle — deployed and serving, drills outstanding |
| Last completed gate | Phase 7 closed (22 of 24 gaps closed, 2 accepted); deployment report signed off — 2026-08-29 |
| Next gate | Phase 8 documentation (both drills passed) |
| AFK mode | off since 2026-08-28 |
| Open, gated on Kenny | the reboot and self-update drills, the Traefik route (deliberately not assumed — every source is on the LAN), the service account's `cal-stacean` display name, and who owns updates once the homelab supervises CT 112 |

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
| docs/INVENTORY.md | existing cal-stacean behaviour, brownfield sweep (Phase 1) — done, 268 lines, 19 defects logged |
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
second layer. CI re-runs the same gates on every push; branch
protection on `main` requires the `gates` check.
