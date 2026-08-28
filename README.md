# Almanac

An adapter that turns webhooks from anything into Google Calendar
events, across several calendars.

## Why "Almanac"

An almanac is not a calendar. A calendar is the grid; an almanac is the
book that gathers what is *going to happen* — from many unrelated
sources, each with its own way of saying it — and lays it out on one
set of dates. Tide tables, planting dates, eclipses, feast days:
different observers, different formats, one volume.

That is exactly this service. Home Assistant, Grafana, Uptime Kuma and
a Claude session each speak their own webhook dialect; Almanac
translates each one and writes it onto the right calendar. It is a
gatherer, not the calendar itself — the calendar stays at Google.

The name also survives the project growing. "cal-stacean" was a pun on
Rust's crab tied to one integration that no longer exists; "Almanac"
still fits the day a fifth source is added.

## What it does

- **Many sources, one hub.** Each source gets its own ingest endpoint
  and its own bearer token, so one can be revoked without touching the
  others.
- **Many calendars.** A source's mapping profile decides which calendar
  its events land on — "infra", "hobbies", whatever the split is. This
  is the point of the project, not a feature bolted on.
- **Nothing is lost.** Every accepted payload is written to a durable
  journal and fsynced *before* the request is answered, so a crash or a
  power cut costs nothing. Undelivered entries go out on the next start.
- **Redelivery converges.** Events are upserted by a private property
  on the Google event, so retrying never produces a duplicate.
- **It explains itself.** A dry-run endpoint shows what a payload would
  become without writing it, and a capture surface records incoming
  requests verbatim — so a new source is reverse-engineered from what
  it actually sends rather than from a guess.
- **It runs unattended.** It survives reboots and power cuts, retries
  through outages instead of giving up, updates itself from signed
  releases, and puts the previous version back if a new one does not
  come up.

Google's credentials live in exactly one place — this service. Nothing
else in the homelab ever holds them.

## Documentation

| Document | What is in it |
|---|---|
| [docs/SCOPE.md](docs/SCOPE.md) | What Almanac is for, and what it deliberately is not |
| [docs/FEATURES.md](docs/FEATURES.md) | The frozen feature list with acceptance criteria |
| [docs/ARCHITECTURE_DECISIONS.md](docs/ARCHITECTURE_DECISIONS.md) | Every architectural decision and the objection that forced it |
| [docs/REALIZATION_PLAN.md](docs/REALIZATION_PLAN.md) | Milestones, status, and the decisions taken at each gate |
| [docs/OPERATIONS_RUNBOOK.md](docs/OPERATIONS_RUNBOOK.md) | Releasing, installing, and what to do when a notification arrives |
| [docs/integrations/home-assistant.md](docs/integrations/home-assistant.md) | The Home Assistant side, including the generic retrying HTTP helper |

## HTTP surface

| Method | Path | What it is |
|---|---|---|
| `POST` | `/v1/ingest/{source_id}` | Accept a payload, journal it durably, answer 202 |
| `POST` | `/v1/ingest/{source_id}/sync` | The same, but wait for delivery and return the Google event id |
| `GET` | `/healthz` | Liveness, no authentication — this is what Uptime Kuma watches |
| `GET` | `/v1/debug/status` | Profiles, journal depth and recent routing decisions |
| `GET` | `/v1/debug/capture` | Recently captured requests, verbatim |
| `GET` | `/dashboard` | Operator UI: status, sources and tokens, captures |

Ingest endpoints authenticate with that source's own bearer token. The
debug endpoints and the dashboard use the operator's credential, and
refuse every request when none is configured — an unconfigured admin
surface closes rather than opens.

## Configuration

Secrets come from [Latch](https://github.com/kennypassenier/latch) and
are injected straight into the process, never written to disk:

```bash
latch run -- ./target/release/almanac
```

Everything else is environment variables and mapping profiles. See
[.env.example](.env.example) for the complete contract — it is the
real list, checked against the code, rather than whatever a secrets
manager happened to hold.

A mapping profile is a small TOML file per source, naming the target
calendar and how to read that source's payload. There are working
examples in [fixtures/profiles](fixtures/profiles), which are also what
the regression tests pin.

## Development

```bash
cargo test --all          # the whole suite
./.claude/hooks/gates.sh   # what a commit has to pass: fmt, clippy -D warnings, tests, boundaries
```

The code splits into `src/core` (pure logic, no I/O) and `src/shell`
(HTTP, files, Google). That boundary is enforced by a gate rather than
by convention, because a single crate gives the compiler no way to
enforce it.

Releases are cut and signed locally, never in CI — see the runbook.

## Deployment

systemd on its own LXC ([deploy/almanac.service](deploy/almanac.service)),
because self-update replaces the running binary and a container would
silently discard that on the next recreation. A compose file
([deploy/docker-compose.yml](deploy/docker-compose.yml)) ships alongside
it for the future homelab-v2 migration; in that mode self-update turns
itself off and homelab v2 owns updates.

Installation, first run and recovery are in
[docs/OPERATIONS_RUNBOOK.md](docs/OPERATIONS_RUNBOOK.md).

## License

MIT — see [LICENSE](LICENSE).
