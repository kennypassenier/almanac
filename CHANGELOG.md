# Changelog

All notable changes to Almanac. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); versions follow
[semver](https://semver.org/).

Releases are signed. Every published release carries `SHA256SUMS` and
`SHA256SUMS.minisig`, signed offline with the key whose public half is
compiled into the binary — which is what the self-updater verifies
against before it installs anything.

## [Unreleased]

### Added

- **Add a source from the dashboard** (K21). `/dashboard/sources` now
  opens with an editable starter profile: save it and the source is
  live, no restart. A profile placed on the machine by hand is picked up
  by *Reload profiles from disk* on the same page.

  Kenny went looking for that button and it was not there — while the
  user guide said the dashboard would "register the source". Adding one
  meant logging into the container, writing a file and restarting the
  service, because profiles were read exactly once at startup. The
  sentence in the guide is corrected in the same change.

  The submitted text is checked by `Profile::parse` — the function
  startup uses — rather than by a second copy of the rules in the
  browser. Fourteen settings, several mutually exclusive; two lists of
  the same constraints drift, and the half that drifts is the one that
  says "fine" to what the service then refuses.

  Nothing is overwritten: this page adds sources. A save whose
  `source_id` matches an existing profile is refused, and so is one
  whose file already exists, because replacing a working profile over a
  retyped id is the mistake that could not be undone from the same page.

- **Retire a source from the dashboard** (K21), on kyu's model at
  Kenny's request: revoking an app there keeps its row with a badge
  rather than erasing it. *Retire* revokes the source's token and
  renames its profile to `<source_id>.toml.retired` — which the loader
  does not read — so the source leaves the running set while the file,
  and the row, stay as the record that it existed. Renaming the file
  back and reloading undoes it.

  Refused while that source still has undelivered events, and the
  refusal says how many. The worker resolves an entry's calendar
  through its profile and the journal never drops an entry, so retiring
  first would strand them: unreachable, erroring on every pass, forever.

  *Revoke* is now labelled *Revoke token*, because it always meant "take
  the key away, leave the source" and there are two destructive buttons
  on that row now.

  Neither adding nor retiring touches events already on the calendar.

### Security

- **`source_id` is now checked for the characters it contains**, not
  only for being non-empty. It has always been a URL segment; with K21
  it also names the file the profile is written to, so
  `"../../etc/cron.d/x"` had to stop being a legal value. Letters,
  digits, `.`, `-` and `_`, not starting with a dot. The three deployed
  source ids are unaffected, asserted by a test.

### Added

- **`ALMANAC_STATE_DIR`** (K20) — one setting moves Almanac's whole
  state tree. `profiles/` and `data/` derive from it, and the journal
  and token store from the resolved data directory. Unset, the root is
  the working directory, which is exactly what almanac did before.

  Asked for by the homelab, which is moving the native services onto
  bind-mounted host paths so a container can be destroyed and recreated
  for nothing. It tried almanac on 2026-08-31 and could not: four
  independent path settings whose defaults *happened* to form a coherent
  tree, with nothing to move. Now a standing requirement in the dev
  procedure — rule 28, "state has an address, and Kenny owns it".

  The four per-path settings remain and still win where present, with a
  test asserting the live deployment's exact configuration resolves
  unchanged. Adopting this release changes nothing anywhere; moving is a
  separate, deliberate act.

### Fixed

- **`almanac update` would have done nothing under the homelab, and
  reported success.** The command read the release URL from the
  environment, but the homelab runs `update_cmd` outside systemd and so
  never sees the unit's `Environment=` lines — and with
  `ALMANAC_SELF_UPDATE=off`, which the supervised arrangement requires,
  the updater refused to build at all. Both paths ended in "not
  configured", exit 0, nothing changed, and a supervisor reading that as
  a successful update.

  The command now falls back to a compiled-in release URL — a property
  of the project, like the signing key — and ignores the
  `ALMANAC_SELF_UPDATE` switch, which governs the background loop rather
  than an explicit instruction. The periodic updater is unchanged: an
  unset URL there still means "this machine does not self-update".

  Caught between publishing 1.3.0 and switching the deployment over,
  which is the only window in which it was findable.

### Added

- **`almanac update`** (K19) — one update, no restart, for a supervisor
  that owns both. Fetches, verifies, probes and installs, then exits;
  writes no probation state, because the thing that called it preserved
  its own copy of the binary and can roll back from outside a process
  that never starts, which this process cannot.

  Built so the homelab can manage almanac's updates. Its supervised
  update preserves the binary, runs `update_cmd`, restarts only if the
  binary actually changed, health-checks and restores on failure —
  which is why its stack file deliberately carried no `update_cmd`
  until now: two systems each holding a rollback would race. The split
  is along what each can actually do.

  `ALMANAC_SELF_UPDATE=off` stops the periodic updater; the explicit
  command still works with it set, because the variable governs the
  background loop, not an instruction from whoever is supervising.

## [1.2.1] — 2026-08-30

### Fixed

- **The dashboard's copy-token button could never work.**
  `navigator.clipboard` exists only in a secure context — https, or
  localhost — and the dashboard is served over plain HTTP on the LAN,
  which is neither. The button died with "navigator.clipboard is
  undefined" every time, in the only way the page is ever opened, and
  said so only in the browser console. Now: the modern API when it is
  genuinely present, `execCommand` next, and failing both the command
  appears already selected to be copied by hand.

### Added

- `examples/show_events.rs` reads a calendar back from Google —
  summary, start, end, free/busy marker and the private property the
  upsert matches on. Almanac's log says what it sent; this says what
  Google kept, and those are different claims.

### Added

- **Event length from the payload** (K18). `end_field` names the payload
  field holding the end, for sources that report a period rather than a
  moment. Exactly one of `duration_minutes`, `duration_days` and
  `end_field` may be set, refused at load time rather than resolved by
  read order. An end at or before the start is refused as well — Google
  accepts it and the result appears on no calendar, which is the worst
  kind of accepted.

  Found while building the first real source, not by testing: 267 tests
  were green and the four fields added hours earlier were proven against
  the real Google API, but a profile could only state a *constant*
  length. A cheap-power window is 480 minutes today and might be 45
  tomorrow. Third time in one day that using the thing found what
  testing did not.

### Added

- **All-day events** (K14). A profile with `all_day = true` produces a
  day marker rather than a timed block, which is what bin day, a
  birthday or a week away actually is. Accepts either a plain date or a
  timestamp from the source, so an existing sensor does not have to
  change to become an all-day source. Google's end date is exclusive
  and has its own test, because getting it wrong produces an event of
  zero length that shows up nowhere.
- **Location** (K15). `location_field` in a profile. The event model
  already had the field and already serialized it; it was hardcoded
  empty and unreachable — the second instance in one day of the thing
  the retrospective had just made a rule about.
- **Reminders** (K16). `[mapping.reminders]` with popup and email
  minutes, or `silent = true`. Omitting the block inherits the
  calendar's default, which is a third and different outcome from
  silence. Google's limits — five reminders, four weeks — are checked
  when the profile loads.
- **Free/busy and status** (K17). `busy = false` stops an infra
  incident from marking Kenny unavailable, which is the one addition
  here that met real data before it was recommended: both alert sources
  already send a status. `status_by` maps a payload field onto Google's
  three statuses in the same shape as `color_by`, and a fourth value is
  refused at startup.

`duration_minutes` becomes optional — an all-day profile has no minutes
to give — and a timed profile that omits it now fails at startup rather
than defaulting to a length nobody chose. Every existing profile is
unaffected.

**Declined in the same mini-round:** recurring events, attendees,
attachments, Meet links, per-event visibility. See `docs/FEATURES.md`
for why each; the recurrence reasoning in particular is worth reading
before anyone proposes it again.

## [1.0.0] — 2026-08-29

Almanac is finished in the sense that matters for a version number: the
interface it offers is settled, and it will be honoured. Everything
rated Essential is built and proven, the mapping-profile format is
pinned by a regression test that fails loudly if its shape changes, and
the guarantees that can only be shown on real hardware — power loss,
reboot, self-update, delete — have been shown there.

What 1.0.0 does not claim is mileage. At the time of release nothing
was posting to Almanac on its own; every payload it had handled was put
there deliberately. That is a statement about use, not about
readiness, and it is recorded here rather than glossed over.

### Fixed

- The debug surface reported `upsert_key: null` for every routing
  decision, including ones that had plainly deduplicated against a key.
  Found by using Almanac once as a source really would (Phase 9
  field test), and it mattered: it is the field someone reads when chasing
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

[Unreleased]: https://github.com/kennypassenier/almanac/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/kennypassenier/almanac/releases/tag/v1.0.0
[0.1.4]: https://github.com/kennypassenier/almanac/releases/tag/v0.1.4
[0.1.3]: https://github.com/kennypassenier/almanac/releases/tag/v0.1.3
[0.1.2]: https://github.com/kennypassenier/almanac/releases/tag/v0.1.2
[0.1.1]: https://github.com/kennypassenier/almanac/releases/tag/v0.1.1
[0.1.0]: https://github.com/kennypassenier/almanac/releases/tag/v0.1.0
