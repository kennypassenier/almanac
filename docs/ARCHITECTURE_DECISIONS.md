# Architecture decisions — Almanac

Frozen decisions, numbered `AR<n>`, permanent once recorded. Phase 3
(tech choice) supplies the first entries below; Phase 4 (architecture)
adds module boundaries, error model, data formats, storage, security
model, concurrency, and update mechanism after the `architecture-critic`
agent has attacked the draft.

Changes after a section is frozen go through a mini-round only
(`FORM_PROTOCOL.md` §5).

## Phase 3 — tech choice (frozen 2026-08-28)

| ID | Decision | Chosen | Notes |
|---|---|---|---|
| AR1 | Language & edition | Rust, edition 2024 | Hard constraint from `docs/SCOPE.md` (S6), formally confirmed here. |
| AR2 | MSRV / toolchain | 1.88, tracking stable above that | Matches the existing Dockerfile build stage (`rust:1.88-slim`) rather than inventing a separate baseline. |
| AR3 | Minimum platform | Linux x86_64 only | Server-side only, runs on Kenny's Proxmox (LXC or VM). No cross-compilation, no Windows target (unlike Latch). |
| AR4 | Web framework | Axum (kept) | Already proven in the existing codebase; native to the tokio/hyper ecosystem. Actix-web and Warp considered and rejected — no benefit at this route count. |
| AR5 | Google Calendar API access | Hand-rolled REST client (reqwest + serde), extended | `google-calendar3` (generated, pulls in `yup-oauth2`, built around interactive OAuth2 consent flows, huge API surface for 5 endpoints) and `gcal` (small but stalled since March 2024, scope explicitly frozen by its author, unclear private-extended-properties support — the exact mechanism K2's upsert pattern depends on) were both evaluated and rejected. The existing ~300-line hand-rolled client already works and gives full control over extended properties. |
| AR6 | Service-account auth (JWT signing) | `jsonwebtoken` (kept), RS256 | Already proven. K4 (token refresh) is a periodic task reusing the same signing function, not a new library. |
| AR7 | Config & mapping-profile format | TOML, for both `config.toml` and every per-source mapping profile | Consistent with the existing config file; one config language across the codebase. |
| AR8 | Secrets delivery from Latch | Process-wrapper: `latch run -- ./almanac` | Secrets land directly in process memory via `std::env::var()`, never touch disk as plaintext. Drops the `dotenvy` dependency and the "plaintext `.env` on disk" risk entirely — an improvement over the current Infisical flow, not just a swap. Rejected alternative: `latch pull` writing a decrypted `.env` file, kept behind `dotenvy` — closer to the current shape but reintroduces the plaintext-file risk Latch exists to remove. |
| AR9 | Retry/backoff mechanism (for M3) | The `backoff` crate | Retry-with-backoff has enough edge cases (jitter, max-tries, which status codes are transient vs. permanent) that a small, established crate beats hand-rolling; this is the one place AR11's "hand-roll by default" bar is clearly not met. |
| AR10 | Validation approach (for M4) | Hand-written validation functions per struct | Config and mapping-profile schemas are small and stable; a generic schema-validation crate (e.g. `validator`) adds ceremony for little benefit here. Matches the existing style (`init_tracing`'s `log_level` validation already works this way). |
| AR11 | Dependency policy | Reluctant — a new dependency must earn its place against "could 50–150 lines of hand-rolled code do this as well or better?" | Small, actively maintained, narrowly-scoped crates (e.g. `backoff`) are welcome; broad, generated, or framework-style crates (e.g. `google-calendar3`) only when hand-rolling clearly loses. |
| AR12 | License | MIT | Repo confirmed public on GitHub, no license file currently set. MIT matches the old README's stub intent and is the standard default for a small public personal tool. Latch itself is AGPL-3.0-or-later, but that's irrelevant here — Almanac invokes the `latch` binary as a separate process (AR8), it never links against it. |

No item drew "Meer uitleg"; no deep-dive round was needed. Kenny
accepted every recommended option as-is.

## Phase 4 — architecture (frozen 2026-08-28)

Draft attacked by the `architecture-critic` agent before the gate; its
surviving objections were shown to Kenny as ⚔ counter-arguments in the
form and are reflected in the final decisions below. AR16, AR17 and
AR19 went through two deep-dive rounds before settling.

| ID | Decision | Chosen | Notes |
|---|---|---|---|
| AR13 | Module boundaries | Single crate, core/shell split, CI-enforced | "core" module: pure logic (profile→event translation, upsert decision, validation), zero ambient I/O, everything injected via traits. "shell": real Axum handlers, real Google client, real file loading. ⚔ The critic's objection — convention-only boundaries erode silently for a solo maintainer — is answered mechanically: a CI/gates check fails the build if `core` imports any I/O crate (reqwest, axum, tokio::fs, …). Full workspace split rejected as premature ceremony at this scope (Latch needed it; Almanac doesn't). |
| AR14 | Error model | `thiserror` enum + explicit `remedy` field per variant + exhaustive transient/permanent test coverage | Variants per category (Config, ProfileValidation, GoogleApi{transient}, Auth). ⚔ Critic: thiserror templates are static, so the remedy is a required *field*, not a message template — every construction site must fill it; and the transient/permanent table (feeding M3/AR9 retries) gets unit tests for every documented Google error code, so a misclassified 403 can't silently hammer the API. |
| AR15 | Data format pinning | Profile TOML carries `schema_version` + a mandatory, immutable `source_id` field; upsert property format `almanac_source_id = "<source_id>:<external-id>"` pinned with regression fixtures | ⚔ Critic (blocking, accepted): deriving the source name from the profile's filename or display label means a rename silently orphans every existing event (upsert stops matching → duplicates). `source_id` is therefore an explicit field, decoupled from filename and label; M4 validation refuses startup on duplicate `source_id`s. Fixture files (profile + payload + expected event, byte-compared) pin both formats. |
| AR16 | Storage & message durability | Durable ingest journal + Google as the only authoritative store | Flow per message: receive + token check → append to journal + fsync → 202 to sender (fsync failure → 500, sender retries) → worker loop writes to Google → entry marked done. Replay after crash/power loss is safe because upsert (K2/AR15) + idempotency keys (M7) make redelivery converge. The worker serializes per `source_id`, which also answers the critic's search-then-write race. Synchronous callers (K8) wait for their entry's completion and get the event ID. Journal has an explicit size cap with a loud error (no silent caps). Measured cost: ~1–5 ms fsync vs 100–500 ms Google call. No database; the journal is transient transport state, not authoritative data. Kenny's AMQP/RabbitDispatcher idea was analyzed and rejected for Almanac: it relocates the single point of failure to a bridge/broker on the same host, triples the service count, conflicts with K8's synchronous replies, and would couple this project to an unfinished one (and overlap with his existing `mailbox` hub). A future bridge can still compose in front of Almanac as just another HTTP source. Residual accepted gap: a POST arriving while the Almanac process itself is down is lost unless the source retries — window is seconds under systemd restart; recorded as a conscious limitation for Phase 7's TEST_PLAN. |
| AR17 | Security & transport | Per-profile `token_hash` (SHA-256, constant-time compare); separate admin token for the K11 debug surface; Traefik-agnostic TLS with a documented manual fallback | Almanac's code has zero Traefik awareness — it always just listens on its HTTP port; TLS is terminated by the existing Traefik in the default deployment. Fallback is a *sender-side operator action*, never an Almanac decision: if Traefik dies (diagnosis: direct `curl http://<lxc>:8080/healthz` still answers), sources are temporarily repointed to the direct URL — K6 tokens keep protecting access, only wire encryption is temporarily lost. The switch is a numbered procedure in the Phase 8 operations runbook (Kenny chose manual over source-side auto-failover). M1's health endpoint is reachable both via Traefik and directly, so "Almanac dead" and "Traefik dead" are distinguishable in one command. The plaintext-token-generating CLI helper is an explicit Phase 5/6 deliverable, not an afterthought (critic's objection). |
| AR18 | Concurrency | Async single-flight lock around the *entire* token refresh-plus-retry operation | ⚔ Critic: locking only the first attempt recreates the thundering herd one round-trip later when the leader's refresh hits a transient 503 and all waiters retry independently. The lock therefore spans refresh + its backoff retries; waiters share the final outcome. |
| AR19 | Update mechanism | Full self-update (M10, rated Essential via mini-round) | Kenny explicitly wants the software to update itself — CI/Docker publishing alone is not self-update (something still has to pull and restart). Design requirements inherited from Latch's proven approach plus daemon-specific needs: checksum-manifest release flow (built in Phase 5/9), keep the previous binary, verify-before-replace, and clean handover of the bound port and in-flight requests (integrates with M2 graceful shutdown and the AR16 journal, which buffers during the swap). |

Phase 4 frozen by Kenny on 2026-08-28 ("Akkoord — bevriezen"). Changes
from here on go through mini-rounds only.

**Amendment to AR19 (mini-round, 2026-08-28, during the Phase 5
gate):** the dev procedure evolved the same day with the latch-v2 retro
lesson that update *authenticity* must be an architecture decision: a
checksum manifest served from the same host as the binary proves
nothing if that host is compromised — whoever can replace the binary
can replace the checksum. Kenny chose to upgrade AR19's verification
to a **minisign signature under an offline key**: releases are signed
with a key that never lives on any server; the updater verifies
against the public key embedded in the binary. Consequences for built
work: none (nothing built yet); L5's release flow signs the manifest
instead of merely hashing it.
