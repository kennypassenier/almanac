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
dashboard**, rated **Essential**, modelled on `mailbox`'s W2 so the
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
deliberately **not** folded into Almanac. Almanac repeats the mailbox
pattern; the central-issuer idea is recorded as an ecosystem candidate
for its own project so it gets its own scope phase rather than being
smuggled in here.
