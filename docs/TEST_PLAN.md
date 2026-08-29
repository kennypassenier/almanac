# Test plan

What is proven, where, and — just as important — what is deliberately
not. Written at the end of Phase 7, from the test-gap audit and the
mandatory security review, with Kenny's decision recorded for every gap
either of them found.

The rule this document exists to enforce: **an accepted limitation is a
decision, written down. A gap nobody decided about is a hole.**

## The suites

| Suite | What it covers |
|---|---|
| unit tests in `src/core/*` | The pure logic: error classification, mapping, the journal's replay model, version comparison, the worker's pacing state machine, token hashing, sealing, HTML escaping, profile validation. No I/O, so these are fast and total. |
| unit tests in `src/shell/*` | The I/O side against local stubs: the Calendar client's retry loop, token refresh, per-source auth, the encrypted store, the self-updater, the delivery path's calendar routing. |
| `tests/ingest_http.rs` | The ingest surface as real HTTP: authentication, status codes, journalling, both alert sources' real payloads, and an unwritable journal answering 500. |
| `tests/admin_http.rs` | The debug and capture surfaces: the admin guard, the capture-only credential's boundary, the capture cap through the real endpoints, header redaction. |
| `tests/dashboard_http.rs` | The operator UI: login, logout, session expiry, forged cookies, the endpoints that mutate, cookie attributes, and that no page ever renders a token in the clear. |
| `tests/self_update.rs` | Self-update end to end against a local release host with real minisign verification: install, tampered binary, tampered manifest, foreign signing key, unstartable version, downgrade, incomplete release, unreachable host. |
| `tests/process_lifecycle.rs` | The real binary as a process: SIGTERM draining cleanly, the startup retry after an unreachable Google, a broken key exiting, two processes on one data directory, `--check` against a live instance. |
| `tests/mapping_regression.rs` | Each source's real payload byte-compared against a pinned event, so a mapping change that alters output is visible in the diff. |
| `tests/no_secrets_in_logs.rs` | Every one of the secrets, plus process arguments. |
| `tests/calendar_e2e.rs`, `tests/power_loss_drill.rs` | **Live**, against a real calendar. `#[ignore]`d locally; run by the `live-tests` workflow. |

## Where each Essential feature is proven

| Feature | Proven by |
|---|---|
| K1 calendar CRUD | `calendar_e2e` (live) |
| K2 upsert, no duplicates | `calendar_e2e` (live), `core::upsert` unit tests |
| K3 multiple calendars | `shell::delivery` — two profiles, two calendars, plus the pre-upsert lookup |
| K4 token refresh | `shell::auth` — reuse, refresh, cold start, and AR18's single-flight |
| K5 mapping engine | `core::mapping` + the three pinned fixtures |
| K6 per-source tokens | `shell::ingest`, `tests/ingest_http.rs` |
| K7 durable ingest | `shell::journal`, `tests/ingest_http.rs`, the power-loss drills |
| K8 synchronous API | `shell::ingest` — delivery, auth, 502-with-payload-kept, and delete including cross-source isolation |
| K9 alert sources | `mapping_regression` + `tests/ingest_http.rs` at the HTTP layer |
| K11 debug surface | `tests/admin_http.rs` |
| K12 secrets via Latch | `tests/no_secrets_in_logs.rs` |
| K13 health endpoint | `tests/admin_http.rs`, `tests/dashboard_http.rs` |
| M2 graceful shutdown | `tests/process_lifecycle.rs` — a real SIGTERM to a serving process |
| M3 retry with backoff | `shell::calendar_client` — 503-then-success, and a 403 tried once |
| M4 startup validation | `core::profile`, including the IANA timezone check |
| M7 idempotency keys | `shell::delivery`, `tests/ingest_http.rs` |
| M8 one version | `scripts/check-version.sh` in CI and the commit hooks |
| M10 self-update | `tests/self_update.rs`, `core::update`, `shell::update` |
| M11 raw capture | `tests/admin_http.rs` — cap, expiry and redaction through the endpoints |
| M12 dashboard | `tests/dashboard_http.rs`, `shell::token_store` |

## Not covered, by decision

Each of these was put to Kenny as a gap with its concrete failure mode,
and each answer is his.

### T18 · Google's 403 reason strings are a spot check

*Accepted as a known limitation, 2026-08-29.*

The transient/permanent classification is exhaustively tested across
status codes — 5xx and 4xx are complete ranges. But a 403 carries no
information in its status code: Google uses the same code for "you are
going too fast" and "you may not touch this calendar", and the
difference is a reason string in the body. That list is a spot check of
five values.

An undocumented or newly-added reason — `sharingRateLimitExceeded`, for
instance — is therefore treated as permanent. **The direction is safe**:
we give up rather than hammer. The cost is an occasional event that
would have succeeded on a retry, and which now goes to the dead-letter
after three attempts instead.

Why accepted rather than closed: Google changes this list without
announcement, so a test pinned to today's list ages into a false sense
of completeness. The impact is one delayed event, not a wrong or lost
one.

### S1 · The published Home Assistant webhook id

*Accepted, 2026-08-29. Kenny's words: "het is maar een logkanaal".*

A live Home Assistant webhook URL was committed to this public
repository. It has been removed from the working tree, but it is in the
history and must be considered public.

The exposure, stated plainly so the acceptance is informed: a webhook
id is the whole of that automation's authentication, and `local_only`
bounds it to the LAN rather than to people who should have it. Anyone
who can reach the Home Assistant host can therefore post forged homelab
events. Because `op` doubles as the deduplication and acknowledgement
key, a forged event replaying a real `op` — `almanac-update-unverified`,
say — can pre-acknowledge or collapse the genuine alert. So the
accepted risk is not only "false lines in a log": it includes an
attacker being able to suppress a real notification.

Kenny weighed that and chose not to rotate, because the channel carries
no secrets and acts on nothing. Nothing in Almanac depends on the
webhook being authentic.

What follows from the decision, and is now enforced: the URL is not in
any tracked file, `.env.example` marks it a secret, and the systemd unit
takes it from Latch rather than carrying it inline.

## Known limitations that are not test gaps

- **No real reboot or self-update has been run on hardware.** The
  mechanism is proven end to end against a local release host, and
  `--check` is proven against the real binary with the real Latch
  secrets. What is unproven is the last step — SIGTERM, systemd, the new
  binary — on the actual LXC. That belongs to the deployment drill,
  which Kenny holds behind a go per action (D9).
- **Coverage measurement is informational.** The CI job reports it and
  does not gate on it. The number that matters is which files are never
  touched at all — which is how the gaps this phase closed went
  unnoticed — not a percentage.

## What Phase 7 changed

The audit found 24 gaps and the security review 2. Four of the 24 were
not gaps but live defects, each now fixed with a regression test that
fails against the old code:

1. A forgotten capture disabled self-update permanently, because expiry
   only ran while somebody had a capture page open.
2. A login racing a token issue could deadlock the whole service,
   ingest included, while `/healthz` kept answering 200.
3. A dropped connection to the Calendar API was classified permanent,
   so a two-second blip surfaced as a failure.
4. The runbook's first-install step named a directory that does not
   exist.

Both security findings were also mine, from the same day: the published
webhook id above, and a capture endpoint whose only credential was the
one that opens everything.

Everything else was closed as tests, except T18 and S1 above.
