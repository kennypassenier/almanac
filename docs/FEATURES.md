# Features — Almanac

Phase 2 output. Ratings use the fixed scale **Essential · Desired ·
Later · Don't do**, confirmed by Kenny across two gate rounds on
2026-08-28 (round 1: existing/Kenny's features, IDs `K*`; round 2:
Claude's own proposals — gaps, hardening, quality-of-life, IDs `M*`).
IDs are permanent: they appear in commits, test names, docs, and forms
from here on. Changes after the freeze go through a mini-round only
(`FORM_PROTOCOL.md` §5).

## Round 1 — existing / Kenny's features

| ID | Feature | Rating | Test expectation |
|---|---|---|---|
| K1 | Calendar CRUD core — create/update (full replace)/delete on Google Calendar events, plus the typed `GoogleEvent` model | Essential | E2E against a real test calendar: create → read → modify → delete round-trip. |
| K2 | Upsert via external ID — find an event by a private extended property so a repeated source update modifies the existing event instead of duplicating it | Essential | Automated test sending the same source event twice; asserts exactly one Google event exists. |
| K3 | Multiple calendars — create/list/target several calendars (e.g. "infra", "hobbies"); each mapping profile picks its own target calendar | Essential | Test with two profiles writing to two different calendars; asserts no cross-contamination. |
| K4 | Automatic OAuth2 token refresh — fixes the current one-shot-token defect (dies after ~1h uptime) | Essential | Test with an expired/near-expired token asserting refresh happens before the next Google call; plus a test for initial-token-fetch failure. |
| K5 | Generic mapping-profile engine — declarative per-source field mapping (title/time window/color, upsert key, target calendar) replacing hardcoded Vikunja-specific Rust | Essential | Test loading a profile from a sample file and correctly translating a sample payload into a `GoogleEvent`, independent of source. |
| K6 | Per-source bearer tokens on every inbound endpoint (Latch-issued, independently revocable) | Essential | Test asserting requests without/with a wrong token fail (401/403) and a valid token succeeds; assertion that tokens never appear in logs. |
| K7 | Source: Home Assistant (`rest_command`-compatible ingest endpoint) | Essential | E2E test with a sample HA payload producing an event on the correct calendar. |
| K8 | Source: Kenny's Claude sessions via a token-scoped REST API (Almanac is the only thing that ever holds the Google service account credentials) | Essential | Test creating/updating/deleting an event via the REST API using a session token. |

*K8 amendment, 2026-08-29:* the delete verb its acceptance criterion asks for was never built. The Phase 7 gap audit found it; Kenny asked for it during the closing form ("delete moet er uiteraard nog bij"). `DELETE /v1/ingest/{source_id}/events/{external_id}` now exists, addressed by the external id the source itself used rather than by Google's event id — the caller never has to have kept it. A source can only ever address keys under its own prefix, so one source cannot delete another's events even knowing the external id.
| K9 | Source: alert systems (Uptime Kuma, Grafana webhooks) → dedicated infra calendar | Essential | E2E test per system with a sample webhook payload. |
| K10 | Source: Super Productivity mini-plugin | **Later** — explicitly deferred, lowest priority, possibly the last thing added | Defined only if/when picked up. |
| K11 | Debug/introspection surface — structured logs plus a status/query endpoint showing what came in, which profile routed it, what went to Google (no UI) | Essential | Test querying the debug surface for a processed event and getting back the expected routing info. |
| K12 | Secrets via Latch — full replacement of Infisical, local and CI | Essential | Test asserting no secret appears in plaintext in logs, new commit history, or process arguments. |
| K13 | CI: full test suite gates every push, red blocks merge | Essential | The CI setup itself is the evidence: red on a deliberately broken test, green on a healthy commit. |
| K14 | All-day events — a profile can produce a day marker rather than a timed block | Essential | A profile with `all_day = true` produces a Google event carrying `start.date`/`end.date` and never `dateTime`; a timed profile produces the opposite. Both asserted, since sending both is what Google rejects. *(Added 2026-08-29 via mini-round.)* |
| K15 | Location reachable from a mapping profile | Essential | A profile naming `location_field` puts that payload field on the event; the pinned regression fixture shows it. Closes a half-built field that was serialized and always empty. *(Added 2026-08-29 via mini-round.)* |
| K16 | Reminders per profile — a set of overrides, or deliberate silence | Gewenst | A profile asking for reminders produces them on the event; one asking for silence produces `useDefault: false` with no overrides; one saying nothing omits the block so the calendar's own default applies. *(Added 2026-08-29 via mini-round.)* |
| K17 | Free/busy and event status per profile | Gewenst | A profile with `busy = false` produces `transparency: "transparent"`, so an infra incident does not mark Kenny busy; `status_by` maps a payload field onto Google's three statuses and rejects any other value at startup. *(Added 2026-08-29 via mini-round.)* |
| K18 | Event length from the payload — a profile may name the field holding the end time instead of a fixed duration | Essential | A profile with `end_field` produces an event ending where the payload says; setting it alongside `duration_minutes` or `duration_days` is refused at startup. *(Added 2026-08-29 via mini-round.)* |
| K19 | `almanac update` — one supervised update, no restart, for a manager that owns the restart and the rollback | Essential | Installing under supervision leaves no probation state, while the ordinary path still writes one; both asserted. *(Added 2026-08-30 at Kenny's instruction — see amendment.)* |
| K20 | One documented knob for the whole state tree — `ALMANAC_STATE_DIR`, with every path derived from it | Essential | A profile tree and a data tree both move by setting one variable; the four existing per-path settings still win where present, asserted against the live deployment's exact configuration. *(Added 2026-09-01 — standing rule 28.)* |
| K21 | Add a source from the dashboard — submit its mapping profile, validated by the same rules startup uses, written to the profiles directory and live without a restart; plus a reload for profiles placed by hand | Essential | Round trip: a profile saved through the surface is read back by `load_all`; an invalid one writes nothing; a duplicate `source_id` is refused before it can break the next start; a `source_id` that would escape the profiles directory is refused and writes nothing outside it; an existing file is never overwritten. *(Added 2026-09-02 via mini-round — see amendment note below.)* |

## Round 2 — Claude's proposals (gaps, hardening, quality-of-life)

| ID | Feature | Rating | Test expectation |
|---|---|---|---|
| M1 | Health/readiness endpoint (`GET /healthz` or similar) | Desired | Test asserting the route returns 200 without requiring auth (deliberate exception to K6 — health checks carry no secrets). |
| M2 | Graceful shutdown — SIGTERM/SIGINT drains in-flight requests before stopping | Essential | Test sending a shutdown signal during an in-flight request, asserting it still completes cleanly. |
| M3 | Retry with backoff on transient Google API errors (429/5xx) | Essential | Test simulating a 429/503 followed by success; asserts the event still lands without the caller retrying itself. |
| M4 | Config & mapping-profile validation at startup, with actionable error messages | Essential | Test with a deliberately broken profile asserting startup fails with a message naming the specific problem. |
| M5 | Rate limiting / request body size caps on inbound endpoints | **Later** | Defined only if/when picked up. |
| M6 | Per-source timezone support (currently hardcoded UTC) | Desired | Defined during design — no current known practical failure, revisit if a source needs it. |
| M7 | Idempotency-key support for sources/payloads without a natural external ID (fills the gap K2's upsert leaves for ad-hoc, ID-less calls) | Essential | Test sending the same call with the same idempotency key twice, asserting one event results. |
| M8 | One coherent versioning/tagging scheme; Docker image tagged to match the git version (fixes the current drift between manual and CI auto-tagging, and `:latest`-only image pushes) | Essential | CI check asserting the pushed image tag exactly matches the git tag from the same run. |
| M9 | Dry-run/validation tool for a mapping profile — shows the `GoogleEvent` a profile+sample payload would produce, without writing to Google | Desired | Test feeding a profile + sample payload, asserting the expected (unsent) `GoogleEvent` structure comes back. |
| M12 | Management dashboard — login (remember-me cookie + logout, not browser basic auth), register a source and generate/revoke its token, copy-paste commands carrying a real token (masked, reveal for 10s, copy without displaying), plus the K11/M11 status views. `/healthz` and any metrics stay open so monitoring cannot fail closed. | Essential | Every page rendered with seeded state; a revoked token stops working *immediately*; the printed command carries a working token; plaintext-scan proving no token reaches logs, metrics or any page except behind the reveal control. *(Added 2026-08-28 via mini-round during the L3 report — see amendment note below.)* |
| M13 | Prometheus metrics endpoint — delivered events, failed deliveries, journal depth, entries set aside, token refreshes, exposed in the Prometheus text format | Gebouwd | Scraped successfully by the real Prometheus on CT 113; a test asserting no token, calendar id or payload content appears in the output. *(Added 2026-08-29 via mini-round — see amendment note below.)* |
| M11 | Raw request capture — a debug endpoint that accepts any inbound request, stores it verbatim (headers + full body, in memory, capped and expiring), and hands it back on request, so an undocumented webhook's real shape can be observed before a mapping profile is written for it | Essential | Test posting an arbitrary payload to the capture endpoint and reading back the exact headers and body; test that the cap and expiry both hold. *(Added 2026-08-28 via mini-round during L2 — see amendment note below.)* |
| M10 | Full self-update — the running service checks for, verifies (checksum manifest), and applies new versions itself; keeps the previous binary, verify-before-replace, clean handover of port and in-flight requests | Essential | E2E test against a local mock release: old binary updates to new, health endpoint answers throughout minus the swap window, rollback works when the new binary fails verification. *(Added 2026-08-28 via mini-round during Phase 4 — see amendment note below.)* |

## Tally

| Rating | Count | IDs |
|---|---|---|
| Essential | 20 | K1–K9, K11–K13 (12), M2, M3, M4, M7, M8, M10, M11, M12 (8) |
| Desired | 3 | M1, M6, M9 |
| Later | 2 | K10, M5 |
| Don't do | 0 | — |
| **Total** | **25** | |

No items were flagged as missing in either round; both open-items fields
were left blank.

## Freeze

**Frozen 2026-08-28.** Kenny confirmed the original tally (22 features)
via the Phase 2 report form. Changes from here on go through a
mini-round only (`FORM_PROTOCOL.md` §5).

**Amendment 2026-08-28 (mini-round, during Phase 4):** while deciding
AR19 (update mechanism) Kenny challenged the assumption that the
CI/Docker flow counts as self-update and asked for real self-updating
software. A mini-round added **M10 · Full self-update**, which Kenny
rated **Essential**. The tally above includes it. Consequences for
built work: none (nothing built yet); consequences for planning: the
release flow must produce a checksum manifest before M10 can be built,
and M10's design is coupled to M2 (graceful shutdown) and AR16 (journal
buffers during the swap) — see `ARCHITECTURE_DECISIONS.md` AR19.

**Amendment 2026-08-28 (mini-round, during the L2 report):** Kenny
raised that many apps ship webhooks without documenting their payload
shape, so writing a mapping profile for one means guessing. A
mini-round added **M11 · Raw request capture**, which he rated
**Essential**. It is distinct from K11 (which shows what happened
*after* an existing profile processed an event): M11 captures verbatim
what arrived *before* any profile exists for that source. Consequences
for built work: none; planned into L4 alongside K11, with which it
shares the admin-token surface.

**Amendment 2026-08-28 (mini-round, during the L3 report):** three of
Kenny's four L3 follow-up questions assumed a UI that did not exist.
The underlying need turned out to be concrete rather than cosmetic:
tokens for every service have to be manageable without SSH-ing into
the LXC for each one. A mini-round added **M12 · Management
dashboard**, rated **Essential**, modelled on `kyu`'s W2 so the
two services are managed the same way. It carries a matching change to
AR17 (tokens encrypted at rest rather than hashed, and a single
authentication path — Kenny rejected Claude's proposal to keep
hand-managed profile hashes alongside the store, on the grounds that
two parallel paths drift apart). Consequences for built work: the
profile schema loses `token_hash`; `examples/issue_token.rs` is
superseded by the dashboard. Bootstrap CSS is vendored into the repo
and image rather than loaded from a CDN, since a LAN-only service must
not need the internet to render its own status page.

A general, cross-service key manager — Kenny's larger ambition — is
deliberately **not** folded into Almanac. Almanac repeats the kyu
pattern; the central-issuer idea is recorded as an ecosystem candidate
for its own project so it gets its own scope phase rather than being
smuggled in here.

## M13 amendment (mini-round, 2026-08-29)

The frozen list had no metrics feature. The word appeared exactly once,
in passing inside M12: "`/healthz` and any metrics stay open so
monitoring cannot fail closed", and again in its acceptance criterion
"no token reaches logs, metrics or any page". Anticipated in the design,
never specified, never built.

**The new insight:** a Prometheus now runs on CT 113 and already scrapes
kyu and the Proxmox fleet, and Kenny named Almanac as a target in
his metrics form. Without this endpoint Almanac would be the only
service in the fleet with no metrics — and the numbers already exist
inside it, kept in memory for the debug page and thrown away on every
restart.

A detour worth recording: the first suggestion was to point Prometheus
at `/healthz`. The homelab session corrected that — Prometheus parses
its own exposition format, not JSON, so the target would sit permanently
"down" on a service that is running perfectly. That correction also
matches AR21, which already names Uptime Kuma as the watcher of
`/healthz`. Liveness there, metrics here.

**Consequences for what is already built:** none. A new endpoint beside
the existing ones on the same port, nothing rebuilt.

**Decision:** adopted as Desired, 2026-08-29 — after the deployment
drills (Traefik route, reboot, self-update on hardware), not before. The
same no-tokens rule that the dashboard and the log already enforce with
tests applies to it.

## Google Calendar field coverage (mini-round, 2026-08-29)

Kenny asked whether Almanac can use everything a Google Calendar event
offers. It could not, and nowhere said so: the event model carried seven
of Google's fields and `docs/SCOPE.md` never listed that as a limit.

**Tested against reality first, and only half of it could be.** The
three sources that exist today all send point-in-time incidents — the
pinned fixtures show Grafana sending `startsAt`/`status`/summary and
Uptime Kuma sending `time`/`status`/monitor name. None of them needs a
day marker, a reminder or a repeat. Everything asked about lives on the
household side of Almanac, and nothing is connected there yet: a search
of Home Assistant found no waste sensors and no calendar entities. The
recommendations for K14, K16 and K17 were therefore presented as
hypotheses rather than as tested proposals, and Kenny rated them
without a worked example of his own ("weet ik zelf nog niet").

**Adopted:** K14 all-day (Essential), K15 location (Essential), K16
reminders (Desired), K17 free/busy and status (Desired).

**Declined, with reasons worth keeping:**

*Recurrence (`RRULE`)* — **Don't do.** Not for lack of value: it has a
genuine design problem that deserved its own round rather than a field.
Almanac's whole model is one payload, one event, and K2's upsert rests
on it. A recurring event is one Google event with instances beneath it,
so an update from a source either rewrites the series or one occurrence,
and both answers are defensible. Half-building it is how a source
silently overwrites a whole series one day. The workaround is real: a
source posts each occurrence, e.g. a week ahead every Monday.

*Attendees* — **Don't do.** Adding guests means Almanac starts sending
mail to people. A profile mistake stops being a wrong calendar entry and
becomes an invitation to the wrong person, which cannot be taken back —
a different class of consequence from everything else here. Sharing the
calendar already solves the household case.

*Attachments, Meet links, visibility* — **Don't do.** Attachments need
Drive scopes Almanac deliberately does not hold; Meet links belong to
meetings with people, not to bin day; per-event visibility only matters
on one calendar shared with several people, and a second calendar is
simpler and already supported (K3).

**Consequences for what is already built:** none of the four additions
changes an existing profile. Every new key is optional and absent means
today's behaviour. `duration_minutes` becomes optional rather than
required, because an all-day profile has no minutes to give — existing
profiles that supply it are unaffected.

## Variable event length (mini-round, 2026-08-29)

Found while building Almanac's first real source rather than by
testing — the third time in one day that using the thing found what 267
green tests did not.

**The case.** Kenny's Home Assistant knows when electricity is cheap:
the EPEX sensor carries all 96 quarter-hours of the day with a price
position for each, so the actual cheap windows can be computed rather
than guessed. Verified against live data before proposing anything: on
2026-08-29 that yields one contiguous window from 08:45 to 16:45.

**What pinched.** A mapping profile can say "start at this field" but
only "and last this many minutes" as a constant. A cheap-power window is
480 minutes today and might be 45 tomorrow. The length is in the
payload and no profile could reach it.

The workaround was available and rejected: post a fixed hour and put the
real window in the title. That puts a one-hour block on the calendar for
an eight-hour window — a calendar showing something other than what it
says, which is worse than no calendar entry.

**Decision:** `end_field` adopted as Essential. Exactly one of
`duration_minutes`, `duration_days` and `end_field` may be set, checked
at startup like the existing all-day contradiction. Absent, a profile
behaves exactly as before.

**Consequences for what is already built:** none. Every existing profile
uses `duration_minutes` and is untouched.

**Why Essential rather than Desired:** every source that reports a
*period* rather than a *moment* hits this, and that is not only energy
prices — "the washing machine ran from X to Y", "away from Monday to
Friday", "the backup took three hours" are all the same shape.

## Supervised updates (2026-08-30)

Kenny: *"Zorg dat het Homelab Rust dit project binnenkort kan beheren,
dan kan dit gesprek gearchiveerd worden."* Recorded as an amendment
rather than a mini-round form because the instruction *is* the decision;
what follows is only how it was carried out.

**The state before.** The homelab adopted CT 112 on 2026-08-29 and backs
it up nightly, but `stacks/almanac/service.yml` deliberately carried no
`update_cmd`, with the note: *"the app has (or may have) its own
complete rollback mechanism, and two systems restoring binaries can
fight each other. Ownership is Kenny's call (form pending)."*

**Why almanac could not simply be handed the job.** The homelab's
supervised update preserves the binary, runs `update_cmd`, and restarts
**only if the binary actually changed** — then health-checks and, on
failure, restores the preserved copy from outside. Almanac's own updater
restarts itself and arms its own revert. Pointing `update_cmd` at that
would give two systems a rollback each, and they would race.

**The split that resolves it.** Almanac knows how to find a release,
verify its signature and checksum, and prove the new binary starts on
this machine. The homelab knows how to restart a unit and restore a
binary when the process is dead — something a dead process cannot do for
itself. So: `almanac update` does the first half and stops, writing no
probation state; the homelab does the second half.

`ALMANAC_SELF_UPDATE=off` on the deployment stops the periodic updater,
so only the supervisor initiates. The explicit `update` command still
works with that set — the variable governs the background loop, not an
instruction from whoever is in charge.

**Consequences for what is already built:** none. The unsupervised path
is unchanged and still arms AR23's revert, with a test asserting it, so
a machine running almanac without a supervisor keeps exactly today's
behaviour.

## State has an address (2026-09-01)

Requested by the homelab session on Kenny's instruction (his form item
A1, 2026-08-31), and now a standing requirement rather than a one-off:
dev-procedure **rule 28**, *state has an address, and Kenny owns it*,
with a mandatory Phase 2 item behind it. Verified in that repo before
acting on the report rather than taken on trust.

**How it was found.** The homelab is moving the four native Rust
services onto bind-mounted host paths, so a container can be destroyed
and recreated for nothing and the host's restic job can reach the state.
It tried almanac on 2026-08-31 and the attempt failed live — eight
minutes down, reverted, nothing lost. Almanac was the one service in the
house that could not be moved.

**What was actually wrong.** Almanac had four independent settings —
`ALMANAC_PROFILES_DIR`, `ALMANAC_DATA_DIR`, `ALMANAC_JOURNAL`,
`ALMANAC_TOKEN_STORE` — whose *defaults happened to* form a coherent
tree. Happening to agree is not being derived. There was no single thing
to move, and the deployment set all four absolutely, so relocating meant
editing four values in agreement and hoping.

**Decision.** `ALMANAC_STATE_DIR` names one root; `profiles/` and
`data/` are derived from it, and the journal and token store from the
resolved data directory rather than from the root — someone who moves
only the data directory means the journal too, and a journal separated
from the lock that guards it is two processes away from a corrupted log.

The four per-path settings stay, and a specific one wins over the root.
Deployments already set them, and a release that silently relocated a
live journal because a tidier knob had appeared would be the worst kind
of upgrade.

**No cache is excluded because there is no cache.** Rule 28 asks for
regenerable state to live outside the backed-up root; almanac keeps none
on disk, and saying so is more useful than inventing a directory to
satisfy the shape of the rule.

**Consequences for what is already built:** none, deliberately. The
default root is `.`, which reproduces the previous relative defaults
exactly, and there is a test asserting CT 112's four absolute settings
resolve unchanged. The migration is the homelab's to perform when it
chooses.
## K21 amendment (mini-round, 2026-09-02)

**Where it came from.** Kenny opened `/dashboard/sources` to add a
source and could not find the button. He was not misreading the page:
the dashboard listed the loaded profiles and offered *Issue*,
*Re-issue* and *Revoke* per profile, and nothing else. Adding a source
meant logging into CT 112, writing a `.toml` file by hand, and
restarting the service — because profiles were read exactly once, at
startup.

The user guide meanwhile said the dashboard would "register the source",
which is why he went looking. That sentence has been corrected in the
same change.

**What was decided.** A profile editor rather than a field-by-field
wizard. The profile format has fourteen settings and several that
exclude one another — `duration_minutes`, `duration_days` and
`end_field` may never appear in pairs — and all of those rules already
live in `Profile::parse`, which runs at startup. A wizard would need a
second copy of them in the browser, and the copy that drifts is always
the one that says "fine" to something the service then refuses. A
textarea validated by the real loader has one set of rules by
construction.

**What it changed in what was already built.** Profiles moved behind a
lock so the set can be swapped while the service runs; readers take an
`Arc` and drop the guard immediately, so a reload never blocks a
request. `source_id` gained a character rule — it was only ever checked
for being non-empty, and it is now also a filename, so
`"../../etc/cron.d/x"` had to stop being a legal value. The three
deployed source ids are unaffected and a test says so.

**Deliberately not built:** editing or deleting an existing profile from
the dashboard. This page adds sources. Replacing a working profile
because a `source_id` was retyped is the one mistake that could not be
undone from the same page, so a save that would overwrite is refused
outright.
