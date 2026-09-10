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
| Current phase | done — **chassis-rs 2.0.0 adopted 2026-09-10** (`9bee012`, PR #3): `Client::adopted(...)` replaces the struct literal (`Client` is `#[non_exhaustive]`), the Sources page's own calendar column renamed **Calendar name** (the kit now auto-renders a column per declared client-issue-form field). Followed by a full `chassis sync --write` (kp-themes 5.1.0, static musl build on `gcr.io/distroless/static`, CI back to every-branch triggers, `tests/kit_smoke.rs` added) with `.githooks/commit-msg` and `.claude/hooks/check-commit.sh` hand-restored afterward, since chassis-rs's own vendored copy still predates Almanac's 2026-09-10 ID-scheme rollout. Tests and gates green throughout. Not yet released — v4.0.3 (chassis-rs 1.8.0) is still the last tagged version. |
| Last completed gate | scaffold-sync decision (2026-09-10): "Dichten" — close the scaffold gap now rather than defer it |
| Next gate | Kenny runs `scripts/sign-release.sh v4.0.3` (uploads `SHA256SUMS.minisig` + `VERSION`) for the still-unsigned 4.0.3; then Homelab Rust's supervised `almanac update` on CT 112; a new release carrying the 2.0.0/scaffold-sync work waits on that |
| AFK mode | off since 2026-08-28 |
| Updates | **the homelab owns them** since 2026-08-30. `ALMANAC_SELF_UPDATE=off` on CT 112; `stacks/almanac/service.yml` carries `update_cmd: runuser -u almanac -- /opt/almanac/almanac update`. Exactly one of the two may ever be armed |
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
second layer. CI re-runs the same gates on every push; branch
protection on `main` requires the `gates` check.
