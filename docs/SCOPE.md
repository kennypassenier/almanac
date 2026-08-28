# Scope — Almanac

Phase 0 output. Approved via the Phase 0 gate form on 2026-08-28 — every
item below reflects Kenny's actual answer, not the draft. Frozen except
through a mini-round (`FORM_PROTOCOL.md` §5) once later phases are under
way.

## Naming

**Almanac.** An almanac is historically a compiled reference of dates,
events, and recurring occurrences pulled from many unrelated domains —
astronomical, agricultural, civic — into one chronological volume. That
is exactly this project's job: pull events from unrelated systems (task
managers, home automation, monitoring, AI sessions) and compile them
into calendars anyone in the household can read. It doesn't collide with
an existing product occupying this niche, and it reads clearly without
translation.

Alternatives considered and dropped: Chronicle (collides with Google's
Chronicle security product), Postmark (collides with the email service
of the same name), Scribe (too generic, widely reused), Datum
(too generic to be findable).

**Action item for Phase 8:** the above rationale must open `README.md`,
before any other content, so the name is never a mystery to a future
reader.

## Mission

Almanac is a single service on the home network that receives events
from other systems and translates them into calendar entries on Google
Calendar. The calendar becomes the readable plan-and-log of the
household and the homelab. Source systems never need to know anything
about Google — no credentials, no API knowledge; only Almanac talks to
Google.

**Multi-calendar management is a first-class part of the mission, not
an afterthought.** Almanac must be able to create and manage several
distinct calendars — e.g. an "infra" calendar for homelab events, a
"hobbies" calendar, others as they come up — not just write into one
fixed default calendar. Each source's mapping profile (see below)
selects which calendar its events land on.

## Shape: hub-and-spoke, universal in, calendar-only out

Any source that can POST JSON over HTTP can plug in. Outbound stays
deliberately narrow for v1: Google Calendar is the only destination
type, but that destination now spans multiple calendars (see Mission).

Per source, a **mapping profile** defines:
- how payload fields translate to event fields (title, time window,
  color),
- which calendar the profile targets,
- how updates find and modify the same event again (an external ID
  stored as a Google Calendar extended property — the pattern already
  proven in cal-stacean's Vikunja integration, generalized instead of
  hardcoded).

Example inbound call, from Home Assistant via its built-in
`rest_command`:

```yaml
rest_command:
  almanac_log:
    url: "http://almanac.lan:8080/v1/ingest/home-assistant"
    method: post
    headers: { Authorization: "Bearer <token-from-latch>" }
    payload: '{"title": "Wasmachine klaar", "start": "{{ now().isoformat() }}"}'
```

## Candidate sources

| Source | Status | Notes |
|---|---|---|
| Home Assistant (`rest_command`) | In scope | No extra tooling needed on HA's side; native to HA. |
| Kenny's Claude sessions (REST) | In scope | Sessions call Almanac's own token-scoped REST API; Almanac is the only thing that ever holds the Google service account credentials. An MCP layer on top is a possible Phase 2 feature (Later-tier candidate), not a scope decision now. |
| Alert sources (Uptime Kuma, Grafana) | In scope | Both have native webhooks. Incidents/recoveries land as events on a dedicated infra calendar — a readable timeline, not a replacement for existing alerting. |
| Super Productivity (mini-plugin) | **Deferred — lowest priority, possible last addition** | SP has no native webhooks; the Local REST API is bound to `127.0.0.1:3876` on Kenny's PC and unreachable from an LXC. A small SP plugin reacting to `taskCreated`/`taskUpdate`/`taskComplete` event hooks is the only realistic route, and it is explicitly the *opposite* of urgent — build it only if everything else is solid and there's appetite left. |
| Vikunja webhook integration | **Out of scope** | Vikunja is no longer used. The mapping/upsert logic it proved (priority → color, ID-based lookup for updates) is kept as the template for the first real mapping profile; the Vikunja coupling itself is not. |

## Non-goals

- **No any-to-any automation bus.** This is n sources → 1 destination
  type (the calendar), not an n×m bus — that space belongs to tools
  like n8n, Node-RED, and Huginn. Phase 1 weighs those honestly
  (build-vs-buy) before anything gets built.
- **No internet exposure.** LAN-only: no port-forwarding, no public
  reverse proxy. Every source still gets its own bearer token (defense
  in depth — the current cal-stacean code has zero auth on its own
  endpoints, so anyone on the LAN can create or delete events today).
  A token is scoped per source and revocable without touching the
  others.
- **No web UI or dashboard for v1.** Configuration lives in files
  (mapping profiles, likely TOML). This is explicitly *not* a "ship it
  and hope" non-goal, though: easy debugging is a hard requirement.
  Structured logs are mandatory, and the design must leave room for
  some form of introspection (a status/debug endpoint, a query
  interface, or similar) so a problem can be diagnosed without reading
  raw traffic. The exact shape of that is decided in Phase 2/4, not
  here — but "no UI" must never become "no visibility."

## Success criteria

Almanac is done, for v1, when:

1. A Super Productivity task with a deadline is out of scope for v1
   (see deferred source above) — **not** a launch criterion.
2. A Home Assistant automation creates a calendar event without ever
   touching a Google credential.
3. Almanac runs unattended on Proxmox: survives a restart and a power
   loss without manual intervention, and refreshes its own Google OAuth
   token before expiry (the current code obtains a token once at
   startup and never refreshes it — a known defect that gets fixed as
   part of this project, not carried forward).
4. No secrets in the repo, logs, or process arguments; Latch supplies
   the runtime `.env`, and tests assert the absence of plaintext
   secrets in anything that touches secret material.
5. CI is green — full test suite — on every push.

## Hard constraints

- **Language:** Rust (edition 2024) for the hub. The Super Productivity
  plugin, if and when built, is necessarily JS/TS — a platform
  constraint of the host app, not a project preference.
- **Deployment:** Kenny's standalone Proxmox server. Whether Almanac
  gets its own LXC or shares one with another service is a Phase 5
  decision, not a Phase 0 one.
- **Secrets:** Latch (Kenny's own secrets-distribution project, at
  release 2.0.0) replaces Infisical entirely, including the CI
  integration. This is Latch's first real consumer.
- **Network:** LAN-only, per the non-goals above.
- **Tooling:** subscription-included only, for the whole project
  (local subagents, `/code-review`, `/security-review` — never
  credit-billed extras).

## Disposition of the existing code

The current codebase (`src/main.rs`, ~1,700 lines, no tests, formerly
named cal-stacean) is raw material, not a mold: restructuring up to and
including a full rewrite is allowed. Phase 1 inventories exactly what
it does today before anything is discarded, so nothing silently
disappears. The repository and its working directory have already been
renamed from `cal-stacean` to `almanac` (2026-08-28); internal
references (package name, binary name, Docker image name, CI workflow)
are Phase 1 inventory items, to be changed deliberately rather than
piecemeal.

## Open items

None raised in the Phase 0 form (S9 and the general remarks field were
both left blank).
