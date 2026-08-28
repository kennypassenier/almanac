# Realization plan — Almanac

Phase 5 output, approved by Kenny on 2026-08-28 (all six milestones
"Akkoord"; standing rules 1–20 confirmed item-by-item, SR3 after a
deep-dive round; hook configuration and placement approved — see the
decisions below the milestone table).

## Status

| Milestone | Features | Status |
|---|---|---|
| L0 · Walking skeleton & hygiene | [meta], K13 | **done**, report approved 2026-08-28 — CI run https://github.com/kennypassenier/almanac/actions/runs/33173304996 |
| L1 · Authenticated calendar core | K1, K4, K12, M3 (AR14, AR18) | **done incl. live proof**, report approved 2026-08-28 (CI https://github.com/kennypassenier/almanac/actions/runs/33174789576). The create→read→update→find→delete round-trip ran green against the real `almanac-test` calendar on 2026-08-28 via `latch run -- cargo test --test calendar_e2e -- --ignored` |
| L2 · Profiles & upsert engine | K2, K3, K5, M4, M6, M7 (AR15) | **done**, report approved 2026-08-28 — CI https://github.com/kennypassenier/almanac/actions/runs/33177366789. M7 is a deliberate, ratified partial (schema support only; the HTTP-layer idempotency-key mechanism needs L3's ingest endpoint) |
| L3 · Durable ingest & access | K6, K7, K8, M2, M7 (AR16, AR17) | **code done**, CI https://github.com/kennypassenier/almanac/actions/runs/33185972306 — both live power-loss drills green against the real scratch calendar; M7 now complete (schema in L2 + the Idempotency-Key header here); pending Kenny's report sign-off |
| L4 · Sources & visibility | K9, K11, M1, M9, M11 | **code done**, CI https://github.com/kennypassenier/almanac/actions/runs/33190845508 — awaiting Kenny's report sign-off |
| L4b · Dashboard & token management | M12 (AR17 amendment) | **code done**, CI https://github.com/kennypassenier/almanac/actions/runs/33192274769 — awaiting Kenny's report sign-off |
| L5 · Release, deployment & self-update | M8, M10 (AR19) | not started |

Deferred (rated Later, not planned into a milestone): K10 (Super
Productivity plugin), M5 (rate limiting).

## Milestones

### L0 · Walking skeleton & hygiene
Rename everything internal to almanac (Cargo package, binary, Docker
image, CI); delete the accidentally-committed 7.3 MB binary and
`.infisical.json`; rewrite the broken Makefile (the orphaned recipe
fragment that makes `make tag-minor` silently run `cargo clean` and
delete env files); set up the core/shell module structure (AR13) with
empty modules; CI green from day one on fmt + clippy + (empty) test
suite, including the AR13 boundary check.
**Exit criteria:** CI green on the empty-but-real structure; no
binaries in `git ls-files`; boundary check demonstrably fails on a
test violation.

### L1 · Authenticated calendar core
AR14 error model (thiserror + mandatory remedy field), K1 event CRUD,
K4 token refresh with AR18's single-flight lock spanning
refresh-plus-retry, M3 backoff retries with the transient/permanent
table fully test-covered, K12 Latch secrets delivery (`latch run`
locally and in CI — E2E tests hit a real test calendar from here on).
**Exit criteria:** E2E create→read→update→delete round-trip green
against the test calendar; token-expiry test green; classification
table fully covered; plaintext-scan assertion on logs green.

### L2 · Profiles & upsert engine
K5 TOML mapping profiles with AR15's immutable `source_id` +
`schema_version`; M4 startup validation (refuses to start naming
file/field/expectation); K2 upsert via extended property; K3 multiple
calendars with per-profile target; M7 idempotency keys; M6 timezone
field in the schema; AR15 regression fixtures (profile + payload +
expected event, byte-compared).
**Exit criteria:** same-event-twice → one Google event; two profiles →
two calendars without cross-contamination; broken profile → clear
startup refusal; fixtures green.

### L3 · Durable ingest & access
AR16 journal (append+fsync → 202, worker loop, replay-on-start,
explicit size cap with loud error); K6 per-source tokens (SHA-256
hash in profile, constant-time compare) + the token-generating CLI
helper (AR17); M2 graceful shutdown; K7 Home Assistant source; K8
Claude-session REST source (synchronous reply through the journal).
**Exit criteria:** kill -9 mid-processing → after restart nothing lost
and nothing duplicated (power-loss drill on the scratch resources);
401/403 tests; HA sample payload E2E green; shutdown-during-request
green.

### L4 · Sources & visibility
K9 alert sources (Uptime Kuma + Grafana webhooks → infra calendar);
K11 debug/introspection surface behind a separate admin token; M11 raw
request capture on the same admin surface (accepts any inbound request,
stores headers + body verbatim in memory, capped and expiring, so an
undocumented webhook's real shape can be observed before writing a
profile for it); M1 health endpoint (no auth, reachable both via
Traefik and directly); M9 dry-run tool.
**Exit criteria:** both alert payloads E2E green; debug query shows a
processed event's full route (in → profile → Google); a captured raw
request reads back byte-identical, with cap and expiry both proven;
health reachable
without token; dry-run yields the expected event without a Google
call.

### L4b · Dashboard & token management
*Added 2026-08-28 as the consequence of the M12 mini-round (see
`FEATURES.md`). Splitting it out rather than swelling L4 keeps the two
concerns separately reviewable — L4 is about what Almanac can see, L4b
about how it is administered. **Ratified by Kenny on 2026-08-28**
("Klopt") as a deviation from the six milestones he approved at the
Phase 5 gate.*

M12 dashboard on the AR17-amended model: bootstrap token from the
environment for login (remember-me cookie + logout, not browser basic
auth); an encrypted app-token store with a mandatory separate
encryption key; register/generate/revoke a source's token; copy-paste
commands carrying a real token, masked with a 10-second reveal and a
copy-without-display control; the K11/M11 views rendered as pages.
Bootstrap CSS vendored into the repo and image (no CDN — a LAN-only
service must not need the internet to render its own status page).
Removes `token_hash` from the profile schema and retires
`examples/issue_token.rs`.
**Exit criteria:** every page renders with seeded state; a revoked
token stops working immediately; a printed command's token actually
works; plaintext-scan proves no token reaches logs, metrics or any
page except behind the reveal control; `/healthz` still answers
without a token.

### L5 · Release, deployment & self-update
M8 single version scheme (Docker image tagged exactly as the git tag;
the old dual manual/CI tagging removed); signed-manifest release flow
(AR19 as amended: minisign signature under an offline key); deployment
to the Proxmox LXC incl. Traefik route and the manual-fallback runbook
procedure; M10 full self-update (detect, verify-before-replace, keep
previous binary, clean port handover via M2, journal buffers during
the swap).
**Exit criteria:** image-tag == git-tag check in CI; E2E self-update
test against a local mock release incl. rollback on a
failing-verification binary; service survives an LXC reboot on
Proxmox.

## Phase 5 decisions (from the gate form)

- **AR19 amendment (mini-round, 2026-08-28):** update authenticity is
  a **minisign signature under an offline key** (Kenny signs releases;
  the public key is embedded in the binary), not a checksum-only
  manifest — a checksum stored next to the binary proves nothing if
  the host serving both is compromised. Recorded as an amendment in
  `ARCHITECTURE_DECISIONS.md`.
- **Backup & restore:** state = (1) profiles/config in git, (2)
  secrets in Latch (state-in-git + key escrow), (3) transient journal
  on the LXC disk, (4) calendar data lives at Google. Mechanism:
  git+Latch cover (1)(2); the LXC rides the existing Proxmox vzdump
  backup regime for (3) — automatic, no new machinery. Restore-from-
  zero becomes a numbered OPERATIONS_RUNBOOK procedure (clone →
  `latch pull` → create LXC → deploy) and is **actually drilled** at
  least once in Phase 6/7. Accepted loss window on total machine
  loss: journal entries from the final moments (same conscious
  limitation as AR16's ingest window).
- **Scratch resources (standing rule 14):** a dedicated test calendar
  under the existing service account — Kenny's real calendars are
  never touched by any test — and a throwaway LXC on Proxmox for the
  deploy/reboot/power-loss drills of L3/L5. Every Phase 6 milestone
  runs at least one live drill against these.
  **Provisioned 2026-08-28:** calendar `almanac-test`, id
  `113eab48451fbcc1d7e11db451fb2f3503bdf4bd97ae7346912cf94ec42eacf0@group.calendar.google.com`,
  created by the service account itself via
  `examples/create_test_calendar.rs` (so it owns the calendar outright
  — no sharing step). Its id lives in Latch as
  `ALMANAC_TEST_CALENDAR_ID` next to the three service-account
  credentials. Run the live tests with
  `latch run -- cargo test -- --ignored`.
  **Latch caveat (2026-08-28):** `latch commit` silently finds 0 files
  here because its discovery walker honours `.gitignore`, and `.env`
  is (correctly) gitignored — see the spawned latch bug task. Until
  that is fixed, committing secrets for this project needs
  `.gitignore` moved aside for the duration of the `latch commit`.
- **Placement:** own light LXC now, **with the homelab-v2 compose file
  + stack preset as an L5 deliverable**. Hard requirement from Kenny:
  the migration to homelab v2 must be easy — therefore the compose
  file the standalone LXC runs is byte-for-byte the same file the
  homelab-v2 preset consumes. No separate deployment variants.
- **Hooks (installed with this plan):** git-native blocking layer
  (`.githooks/pre-commit` runs the gates, `.githooks/commit-msg`
  enforces bracketed IDs, wired via `core.hooksPath`) + the Claude
  Code PreToolUse layer (`.claude/hooks/check-commit.sh`) + branch
  protection on `main` requiring the CI `gates` check for merges
  (admin direct-push stays possible — solo workflow; red CI still
  blocks any PR merge per standing rule 6).
- **Known-red start:** clippy fails on the pre-rewrite cal-stacean
  code (doc lints). This is expected and is precisely L0's job; the
  enforcement commit itself predates hook activation, and every
  commit after it — all of L0 included — passes through the gates.
