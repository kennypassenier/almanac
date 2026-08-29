# Changelog

All notable changes to Almanac. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); versions follow
[semver](https://semver.org/).

Releases are signed. Every published release carries `SHA256SUMS` and
`SHA256SUMS.minisig`, signed offline with the key whose public half is
compiled into the binary — which is what the self-updater verifies
against before it installs anything.

## [Unreleased]

### Fixed

- The debug surface reported `upsert_key: null` for every routing
  decision, including ones that had plainly deduplicated against a key.
  Found by using Almanac once as a source really would (Phase 9
  dogfood), and it mattered: it is the field someone reads when chasing
  a duplicate, and it sent them to edit a profile that was already
  correct.

## [0.1.4] — 2026-08-29

### Added

- Self-update now refuses to run inside a Docker or Podman image and
  says why, rather than depending on `ALMANAC_SELF_UPDATE=off` being
  present in the compose file. AR20 had always required this; only now
  does the binary enforce it. LXC deliberately does not count as an
  image — that is where self-update is meant to run.

### Documentation

- `docs/USER_GUIDE.md`, `docs/DEBUGGING_GUIDE.md` and
  `docs/ARCHITECTURE_REFERENCE.md` written.
- README honesty pass; runbook renumbered through R15, including how to
  replace the Google service account and the three traps in doing it.
- `docs/legacy/` for the Phase 1 inventory and the closed AFK queue.

## [0.1.3] — 2026-08-29

### Added

- **Prometheus metrics** at `GET /metrics` (M13). Six counters, journal
  depth, and a version label; no authentication, because a scraper that
  cannot log in reports a healthy service as down. No per-source
  labels, deliberately. An unreadable journal reports itself as
  unreadable rather than as empty.

### Fixed

- **The first self-update check happened six hours after start, not
  five minutes.** The interval was created before the startup delay and
  its immediate first tick consumed, so the delay achieved nothing.
  Every one of the nine self-update tests passed, because they all call
  `check_once` directly and none go near the loop. Found by running the
  drill on real hardware.
- A completed update check now logs a line either way. At debug level a
  working updater and a silently dead one produced identical logs,
  which is how the bug above stayed hidden.

## [0.1.2] — 2026-08-29

Superseded within the hour by 0.1.3; carried the interval fix only.

## [0.1.1] — 2026-08-29

First release published as a signed GitHub Release with all four
assets, and the first one a running instance was asked to install by
itself.

## [0.1.0] — 2026-08-29

First release of Almanac as a rebuilt service, deployed to CT 112 and
serving.

Almanac replaces `cal-stacean`, which was a single 1,681-line
`src/main.rs` with a hardcoded Vikunja integration, Infisical for
secrets, and no durability. Nothing of that shape survives; the
event-mapping and upsert pattern was kept as the template for the
general mapping-profile design. See `docs/legacy/INVENTORY.md` for what
was there and the 19 defects that shaped the rewrite.

### Added

- **Many sources, one hub** — per-source ingest endpoints, each with its
  own independently revocable bearer token (K6). Almanac is the only
  thing in the homelab holding Google credentials (K12).
- **Many calendars** (K3) — each mapping profile names its own target,
  and Almanac creates and shares the calendars itself rather than
  needing them made by hand.
- **A durable journal** (AR16) — every accepted payload is fsynced
  before the 202 is answered, replayed on start, and compacted as it
  grows. A power cut costs nothing.
- **Upsert by external id** (K2) — redelivery converges on the same
  event instead of duplicating, with `Idempotency-Key` (M7) for sources
  with no natural id.
- **A generic mapping-profile engine** (K5) — declarative per-source
  TOML, validated at startup so a bad timezone is caught then rather
  than by Google days later (M4).
- **Delete by external id** (K8) — a source can remove what it created,
  and only what it created.
- **Self-update from signed releases** (M10) — verify, probe with
  `--check`, keep the previous binary, revert if the new one does not
  become healthy.
- **An operator dashboard** (M12), **dry-run** (M9), **raw request
  capture** (M11) and a **debug status surface** (K11).
- **Graceful shutdown** (M2), **retry with backoff** (M3), and a dead
  letter for entries that can never be delivered (T1).
- Secrets from Latch, injected into the process and never written to
  disk — asserted by tests that run the real binary and grep its output.

[Unreleased]: https://github.com/kennypassenier/almanac/compare/v0.1.4...HEAD
[0.1.4]: https://github.com/kennypassenier/almanac/releases/tag/v0.1.4
[0.1.3]: https://github.com/kennypassenier/almanac/releases/tag/v0.1.3
[0.1.2]: https://github.com/kennypassenier/almanac/releases/tag/v0.1.2
[0.1.1]: https://github.com/kennypassenier/almanac/releases/tag/v0.1.1
[0.1.0]: https://github.com/kennypassenier/almanac/releases/tag/v0.1.0
