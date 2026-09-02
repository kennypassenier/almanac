# Almanac — user guide

How to make Almanac do things: connect a source, shape what it writes
onto a calendar, correct it when a source changes its mind, and take it
away again.

This is the "how do I" document. What went wrong and how to find out is
[DEBUGGING_GUIDE.md](DEBUGGING_GUIDE.md); how the machine is run is
[OPERATIONS_RUNBOOK.md](OPERATIONS_RUNBOOK.md).

Feature IDs (K5, M9, …) are in the margins so a claim here can be traced
to [FEATURES.md](FEATURES.md) and to the test that keeps it true.

---

## 1 · The shape of the thing

Something posts a JSON payload to Almanac. Almanac looks up that
source's **mapping profile**, which says which calendar to write to and
which fields of the payload mean what, and turns the payload into a
Google Calendar event.

```
Home Assistant ─┐
Grafana ────────┼─→ Almanac ─→ Google Calendar (several calendars)
Uptime Kuma ────┤
a Claude session┘
```

Three things follow from that, and they are the whole design:

**A source never holds Google's credentials.** Almanac does, and
nothing else in the homelab does (K12). A source holds only its own
bearer token, which grants exactly one thing: posting as that source.

**Almanac answers before Google does.** A payload is written to a
durable journal and flushed to disk *before* the 202 comes back (AR16),
and a background worker delivers it. A source that got a 202 can forget
about it — a power cut between the 202 and the calendar costs nothing,
the entry goes out on the next start.

**Sending the same thing twice is safe.** Events are matched by an id
the source chose, stored as a private property on the Google event, so
a redelivery updates the existing event instead of making a second one
(K2).

---

## 2 · Connecting a new source

### 2.1 · Find out what it actually sends (M11)

Do not guess the payload shape from documentation. Point the source at
the capture endpoint and read what really arrives:

```bash
# On the source's side, temporarily send to this instead:
POST http://10.10.10.12:8080/v1/debug/capture/my-new-thing
Authorization: Bearer $ALMANAC_CAPTURE_TOKEN
```

Then read it back, verbatim, headers included:

```bash
curl -s -H "Authorization: Bearer $ALMANAC_BOOTSTRAP_TOKEN" \
  http://10.10.10.12:8080/v1/debug/capture | jq .
```

The capture token can only *post* captures — it cannot read them back
and it opens nothing else (S2). That is deliberate: the token you paste
into a third-party tool while experimenting is the one most likely to
end up somewhere it should not, so it is the one that can do least.

Captures live in memory, are capped, and expire. They are for an
afternoon of reverse-engineering, not a log.

### 2.2 · Write the mapping profile (K5)

One TOML file per source. The quickest way is the dashboard: on
`/dashboard/sources` the **Add a source** box comes pre-filled with a
starter profile — edit it, save it, and it is live immediately, with no
restart (K21). The same rules that run at startup check it first, so a
mistake names its field and saves nothing.

The file itself lands in the profiles directory
(`/appdata/almanac/almanac-config/profiles/` on the deployment) and can
equally be written there by hand; **Reload profiles from disk** on the
same page picks that up without a restart. Either way it looks like
this:

```toml
schema_version = 1
source_id = "home-assistant"
target_calendar_id = "2774a1…@group.calendar.google.com"

[mapping]
title_field = "title"
description_field = "description"
external_id_field = "entity_id"
start_field = "start"
duration_minutes = 60
timezone = "Europe/Brussels"
```

| Key | Required | What it does |
|---|---|---|
| `schema_version` | yes | Always `1`. A future format bump refuses old files loudly rather than misreading them. |
| `source_id` | yes | The URL segment this source posts to, and the name of its token. Must be unique across all profiles. |
| `target_calendar_id` | yes | Which calendar this source's events land on (K3). |
| `title_field` | yes | Which payload field becomes the event title. |
| `description_field` | no | Which field becomes the body. Omit for a title-only event. |
| `external_id_field` | no | The field holding the source's own id for this thing. This is what makes updates converge instead of duplicating — see 3.1. |
| `start_field` | yes | The field holding the start time, as RFC 3339. |
| `duration_minutes` | for a timed event | How long the event is. Must be greater than zero. Leave it out on an all-day profile — setting both is refused at startup rather than silently resolved. |
| `end_field` | for an event whose length varies | Which payload field holds the end time, as RFC 3339 (K18). Use instead of `duration_minutes` when the source reports a period rather than a moment — a cheap-power window, a wash cycle, a week away. |
| `location_field` | no | Which payload field becomes the event's location (K15). |
| `all_day` | no | `true` makes a day marker instead of a timed block (K14). The start field may then be either `2026-09-01` or a full timestamp on that day. |
| `duration_days` | no | How many days an all-day event covers; defaults to 1. Only on an all-day profile. |
| `busy` | no | `false` shows the event without consuming your availability (K17) — what an infra incident should do. Absent leaves Google's default, which is busy. |
| `timezone` | yes | An IANA name, e.g. `Europe/Brussels`. Checked at startup (M4) — a typo is caught when the service starts, not by Google days later. |

**Exactly one of `duration_minutes`, `duration_days` and `end_field`**
applies, and setting two is refused when the profile loads rather than
resolved in whatever order the code happens to read them. An end at or
before the start is refused too: Google accepts it, and the result is an
event that appears on no calendar at all.

**Nested fields work** with dots: `title_field = "data.alert.name"`
reads `{"data": {"alert": {"name": "…"}}}`.

**Numbers and booleans are coerced to strings**, so a payload with
`"severity": 3` can feed a title without special handling.

**Reminders** (optional, K16). Three distinct outcomes, and the
difference between the last two matters:

```toml
# ask for reminders
[mapping.reminders]
popup_minutes_before = [30]
email_minutes_before = [1440]
```

```toml
# deliberate silence, overriding whatever the calendar defaults to
[mapping.reminders]
silent = true
```

Omit the block entirely and the calendar's own default applies. That is
*not* the same as `silent = true`, which says "no reminders" out loud.
Google allows at most five reminders, none further out than four weeks;
both limits are checked when the profile loads rather than on every
event forever.

**Status by value** (optional, K17) — the same shape as colours, mapping
a payload field onto Google's three statuses:

```toml
[mapping.status_by]
field = "status"
default = "confirmed"
values = { resolved = "cancelled" }
```

Only `confirmed`, `tentative` and `cancelled` are accepted; anything
else is refused at startup.

**Colours by value** (optional):

```toml
[mapping.color_by]
field = "severity"
default = "8"
values = { critical = "11", warning = "5", ok = "10" }
```

Those are Google's colour ids. An unrecognised value falls back to
`default` rather than failing the mapping.

### 2.3 · Check the profile before connecting anything (M9)

The dry-run endpoint shows exactly what a payload would become,
**without writing to Google**:

```bash
curl -s -X POST \
  -H "Authorization: Bearer $ALMANAC_BOOTSTRAP_TOKEN" \
  -H "content-type: application/json" \
  -d '{"title":"Bin day","entity_id":"sensor.waste","start":"2026-09-01T07:00:00Z"}' \
  http://10.10.10.12:8080/v1/debug/dry-run/home-assistant | jq .
```

If a required field is missing, the answer says which field, in which
profile — not "mapping failed".

### 2.4 · Issue the source its token (K6, M12)

From the dashboard at `/dashboard/sources`: the source appears in the
list as soon as its profile is loaded (2.2), and *Issue token* gives it
one. Paste that into the source's configuration. The token is shown
once for copying and stored encrypted; the file on disk never contains
the plaintext.

Each source's token opens only its own endpoint. One source's token
posting as another is rejected exactly like a wrong token — and so is
an unknown source id, so probing cannot tell "no such source" from
"wrong token".

Revoking is immediate: the very next request with that token fails.

### 2.5 · Point the source at Almanac

```
POST http://10.10.10.12:8080/v1/ingest/home-assistant
Authorization: Bearer <that source's token>
Content-Type: application/json

{"title": "Bin day", "entity_id": "sensor.waste",
 "start": "2026-09-01T07:00:00Z"}
```

```json
202 Accepted
{"status": "accepted", "entry_id": "01J…"}
```

202, not 200, and deliberately: it means *"this is safely written down
and will happen"*, not *"this is on your calendar"*. The event appears a
moment later.

For Home Assistant specifically, including a `rest_command` that retries
properly, see [integrations/home-assistant.md](integrations/home-assistant.md).

---

## 3 · Changing and removing events

### 3.1 · Updating an event (K2)

Post again with the same `external_id_field` value. Almanac finds the
existing event by the private property it stored and replaces it.

```bash
# same entity_id, new time → the same event moves
{"title": "Bin day", "entity_id": "sensor.waste",
 "start": "2026-09-02T07:00:00Z"}
```

There is no separate update call, and there is no risk in sending the
same thing twice — which is the point, because a source retrying after
a timeout has no idea whether the first attempt landed.

**Without `external_id_field`**, every post creates a new event. If the
source has no natural id, send an `Idempotency-Key` header instead (M7)
and Almanac uses that as the key:

```
Idempotency-Key: shopping-run-2026-09-01
```

The profile's own `external_id_field` wins when both are present.

### 3.2 · Retiring a source (K21)

*Retire* on `/dashboard/sources` ends a source: its token is revoked and
its profile is renamed to `<source_id>.toml.retired`, which the loader
does not read. It stops posting immediately, no restart.

The file stays, and so does its row on the page, marked `retired` — a
source that vanished without trace is indistinguishable from one that
was never there, and the question months later is always "did we have
one of these?". To undo it: rename the file back to `.toml` and press
*Reload profiles from disk*.

Retiring is refused while that source still has events waiting in the
journal, and says how many. Deliveries resolve their calendar through
the profile, so retiring first would strand them.

**Events already on the calendar are not touched.** They belong to the
calendar now. To remove them, delete them by external id (3.3) *before*
retiring the source, while its token still works.

### 3.3 · Deleting an event (K8)

Address it by the id the source itself used:

```bash
curl -X DELETE \
  -H "Authorization: Bearer <that source's token>" \
  http://10.10.10.12:8080/v1/ingest/home-assistant/events/sensor.waste
```

```json
{"status": "deleted"}
```

Deleting something that is not there answers `not_found` rather than
pretending to have done something. A source can only delete events it
created — one source's token cannot delete another's event, even
knowing the id.

### 3.4 · When you need the event id back (K8)

The ordinary endpoint answers 202 and does not wait. When the caller
genuinely needs to know it landed — a Claude session that wants to
report back — there is a synchronous variant:

```bash
POST /v1/ingest/{source_id}/sync
```

```json
200 OK
{"status": "delivered", "event_id": "abc123…", "created": true}
```

`created: false` means it updated an existing event. If Google is
unreachable, this answers 502 **and keeps the payload** — it is still
journalled and still goes out later. A 502 here means "not yet", never
"lost".

---

## 4 · Several calendars (K3)

Each profile names its own `target_calendar_id`, so the split is
whatever you want it to be. The live deployment uses two:

| Calendar | Sources |
|---|---|
| Almanac · Huishouden | home-assistant |
| Almanac · Infra | grafana, uptime-kuma |

Almanac creates its own calendars rather than needing you to make them
in Google's UI. `examples/create_calendars.rs` creates whatever is
missing and shares each one with a real person — sharing is not
optional and not a separate step, because a calendar the service
account creates is owned by the service account and invisible to
everyone else until it is shared. That mistake has been made here twice
and the tool now re-checks every calendar on every run.

Real calendar ids live in the deployment's profiles and deliberately
not in this repository — they are the household's, not the code's.

---

## 5 · Watching it work

| Where | What it tells you |
|---|---|
| `/dashboard` | Status, sources and tokens, recent captures. The one place to look first. |
| `/v1/debug/status` (K11) | Which profiles are loaded, how deep the journal is, and how recent events were routed. |
| `/metrics` (M13) | Counters for Prometheus: accepted, delivered, failed, set aside, token refreshes, journal depth. |
| `/healthz` (M1) | Liveness only. Answers 200 while Google is down, on purpose — Almanac riding out an outage is working correctly, and a health check that goes red would be lying. |

`/healthz` and `/metrics` need no token, because monitoring that cannot
authenticate reports a healthy service as down. Everything else does.

---

## 6 · What Almanac will not do

- **It does not read calendars back to you.** It writes; Google Calendar
  is the reader.
- **It does not schedule anything itself.** No cron, no timers, no
  "remind me in an hour". A source decides when something happens;
  Almanac writes it down.
- **It does not merge or deduplicate across sources.** Two sources
  reporting the same event produce two events, on whichever calendars
  their profiles name.
- **It does not rate-limit inbound requests** (M5, deliberately Later).
  Every source is on the LAN and trusted; this would matter the day one
  is not.
- **It does not do repeating events** (declined 2026-08-29). Not for
  lack of value — a recurring event is one Google event with instances
  beneath it, and Almanac's whole model is one payload, one event. An
  update from a source would have to choose between rewriting the
  series and rewriting one occurrence, and half-answering that is how a
  source silently overwrites a whole series. Post each occurrence
  instead: a week ahead every Monday works today.
- **It does not invite anyone** (declined 2026-08-29). Adding guests
  means Almanac sends mail to people, and a profile mistake stops being
  a wrong calendar entry and becomes an invitation to the wrong person.
  Share the calendar instead.
- **It does not retry forever.** An entry that fails permanently three
  times is set aside as dead, kept in the journal with its reason, and
  reported — rather than blocking everything behind it (T1).
