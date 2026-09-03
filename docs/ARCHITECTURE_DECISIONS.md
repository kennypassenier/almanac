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
| AR16 | Storage & message durability | Durable ingest journal + Google as the only authoritative store | Flow per message: receive + token check → append to journal + fsync → 202 to sender (fsync failure → 500, sender retries) → worker loop writes to Google → entry marked done. Replay after crash/power loss is safe because upsert (K2/AR15) + idempotency keys (M7) make redelivery converge. The worker serializes per `source_id`, which also answers the critic's search-then-write race. Synchronous callers (K8) wait for their entry's completion and get the event ID. Journal has an explicit size cap with a loud error (no silent caps). Measured cost: ~1–5 ms fsync vs 100–500 ms Google call. No database; the journal is transient transport state, not authoritative data. Kenny's AMQP/RabbitDispatcher idea was analyzed and rejected for Almanac: it relocates the single point of failure to a bridge/broker on the same host, triples the service count, conflicts with K8's synchronous replies, and would couple this project to an unfinished one (and overlap with his existing `kyu` hub). A future bridge can still compose in front of Almanac as just another HTTP source. Residual accepted gap: a POST arriving while the Almanac process itself is down is lost unless the source retries — window is seconds under systemd restart; recorded as a conscious limitation for Phase 7's TEST_PLAN. |
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

**Amendment to AR17 (mini-round, 2026-08-28, during the L3 report):**
token storage changes from per-profile hashes to an encrypted store,
adopting the pattern Kenny already designed for `kyu` (its W2 and
AR11 amendment).

*Why the original decision pinched.* A SHA-256 hash cannot be reversed,
so a token is visible exactly once — at creation. That is fine for a
CLI, but it makes a management dashboard impossible: it could never
show a working copy-paste command again. Kenny's actual constraint is
that tokens must be manageable without SSH-ing into the LXC for every
one.

*What replaces it.*
- A **bootstrap token** from the environment (via Latch, AR8) is what
  Kenny logs into the dashboard with. Something has to be trusted
  first.
- **App tokens live encrypted in a local store**, created and revoked
  from the dashboard, so a working command can always be reproduced.
- A **separate encryption key is mandatory whenever the bootstrap
  token is set.** Deriving the key from the bootstrap token would
  work right up until the day a leaked bootstrap token is rotated, at
  which point every stored app token silently becomes undecryptable.
  A bootstrap token without an encryption key is a configuration
  error and startup refuses it.
- Tokens never appear in logs, metrics or any rendered page except
  behind the dashboard's deliberate reveal control; a plaintext-scan
  test is mandatory.

*One path, not two (Kenny's correction to Claude's proposal).* Claude
proposed keeping per-profile `token_hash` for hand-managed sources
alongside the new store. Kenny rejected that: every token will come
from the dashboard, and two parallel authentication paths are exactly
the kind of thing that drifts apart and rots. So **`token_hash` is
removed from the profile schema entirely** and the encrypted store is
the only source of truth for who may post.

*Consequences for built work.* The L3 ingest layer keeps its shape —
the URL still selects the profile and the presented token is still
verified against what that source is allowed to use — but the lookup
moves from the profile file to the store. `examples/issue_token.rs`
becomes redundant once the dashboard exists and is removed with it.
The profile schema stays at `schema_version = 1`: no profile has ever
been deployed, so there is nothing in the field to migrate, and
bumping the version would imply a compatibility story that does not
exist. `core::token`'s hashing and constant-time comparison are
unaffected and stay as they are.

## Pre-L5 critic re-run (2026-08-28)

PROCEDURE mandates a fresh `architecture-critic` pass before any phase
touching real systems; L5 is the first production rollout. The re-run
found four blocking and seven serious objections. Kenny's decisions:

| ID | Objection | Decision |
|---|---|---|
| AR20 | **Self-update and Docker are structurally incompatible.** A container's filesystem is its writable layer, so a self-replaced binary is silently discarded on the next container recreation — and the service then updates itself again, forever, with every diagnosis starting from the wrong version. | **systemd on the LXC running a bare binary**, so M10's self-update is real. The compose file and homelab-v2 preset ship alongside it for the future migration Kenny requires; in that mode self-update disables itself and homelab v2 owns updates. Both exist, and which one is active decides who updates. **Implementation note, 2026-08-29:** this held only as long as someone remembered to set `ALMANAC_SELF_UPDATE=off` in the compose file. Run the image without that line and AR20's guarantee silently did not apply. The binary now detects a Docker or Podman image itself and switches self-update off by default there, so the decision is enforced rather than configured. LXC deliberately does not count as an image — that is the deployment where self-update is wanted. `ALMANAC_SELF_UPDATE=on` still overrides. |
| AR21 | **Fail-fast at startup defeats SCOPE criterion 3.** After a power cut the LXC can start Almanac before the network settles; the first Google token fetch fails, the process exits, and systemd's default start limit parks the unit in `failed` permanently. Nobody notices for days. | **Distinguish a broken key from an unreachable Google.** A malformed key still exits immediately (it never fixes itself); a transient failure retries with backoff. The systemd unit carries no start limit, and Uptime Kuma watches `/healthz` so a wedged unit is visible rather than silent. |
| AR22 | **Two processes on one journal during the update handover.** The AR16 per-key lock is in-process; a new process draining the same journal the old one is mid-delivering produces exactly the duplicates the journal exists to prevent, and a concurrent compaction can discard done-markers so delivered entries replay. | **An OS-level lock on the data directory**, plus a `--check` mode so a new binary can prove it starts without claiming the port the old one still holds. |
| AR23 | **Nothing runs the previous binary when the new one starts and then dies.** Verify-before-replace covers a bad signature, not a config the new version needs and Latch does not yet have. | **Automatic revert**: after the swap the updater probes health and puts the previous binary back if it does not answer, with a notification — a reverted update must not be a silent event. |
| AR24 | **No story for signing-key loss or rotation.** The public key is compiled in, so a lost key permanently blocks updates on every installed binary. | **One key, as Latch does** (its `OPERATIONS_RUNBOOK.md` R11). Claude initially recommended baking in a spare; Kenny pushed back that both would live in the same vault, so a spare protects against rotation, not loss — and rotation only matters across many machines. There is one. The recovery path (regenerate, rebuild, place the binary by hand once) goes in the runbook, and repeated verification failure raises a notification, because today it would fail silently. |
| AR25 | **Sessions, captures and routes are in memory, so every update logs Kenny out** — sharpest for M11, which is used precisely while reverse-engineering an unknown webhook. | **Sessions move into the encrypted store** so they survive updates and restarts while logout stays a real server-side revocation. Self-update is suppressed while captures are still retained. |
| AR26 | **A long Google outage degrades badly unattended.** The worker never compacts during an outage while re-reading and re-attempting the whole journal every 5s, and hitting the size cap turns into lost events once HA's retry script gives up. | **Back off as failure persists**, and warn through the notification system well before the hard cap — intervene before loss, not after. |

### The deferred Latch objection, decided (2026-08-28)

The critic's remaining objection was about Latch on a headless LXC: an
unattended restart needs the credential that opens the store to live on
the machine, which vzdump then backs up beside the values it protects.
Kenny took it to the Latch project and came back with four decisions.

| ID | Question | Decision |
|---|---|---|
| L1 | Which credential goes on the LXC — the passphrase or the per-project key? | **The project key** (`LATCH_KEY_ALMANAC`). The passphrase opens every project's secrets and the GitHub token; the project key opens Almanac's five values and nothing else. A leaked LXC then costs one project rather than everything. |
| L2 | Exclude that key from vzdump, or accept that the backup contains it? | **Include it.** The backup is then as sensitive as the secrets and is treated that way. Excluding it would produce a restore that stops halfway for manual work — precisely what nobody wants during a real outage. |
| L3 | Almanac accepted a key that does not open the store and only failed later, as a 401 on every source. | **Repair it.** Latch fails loudly on a wrong key; Almanac did not. `TokenStore::verify_key_opens_store()` now proves at startup that the configured key opens the store, so a wrong key is a refusal to start instead of a service that looks healthy and rejects everyone. |
| L4 | Earlier documents in this repo called losing the key catastrophic — every token re-issued. | **Correct them.** Latch keeps all credentials in one passphrase-encrypted escrow file held offline, so recovery is `latch key restore` or a `latch clone` from Kenny's desktop. The backup section of REALIZATION_PLAN.md and the runbook now say so. |

### Decisions taken during Phase 7 hardening (2026-08-29)

| ID | Question | Decision |
|---|---|---|
| AR31 | An entry that can never be delivered — an unmappable payload, a calendar the service account cannot write to, a timezone Google rejects — stayed pending forever. It held the worker at its slowest backoff, delayed every other source by up to half an hour, and eventually raised a backlog alert about a queue of exactly one dead event. | **Set it aside after three consecutive permanent failures, do not delete it.** Three, not one, because a permanent classification can be wrong at the edges — a calendar mid-permission-change, a 403 whose reason Google has not documented — and three passes across the backoff ladder is long enough for a misclassification to recover. Not deleted, because the source was told 202: throwing the payload away would make that a lie, and the failure reason is the only thing that makes the profile fixable. It stays in the journal, survives compaction, is readable on the debug surface, and raises its own notification. |
| AR32 | The capture endpoint was guarded by the bootstrap token, which also logs into the dashboard and reveals every source's plaintext token — while the product's own UI told the operator to point undocumented third-party webhooks at it. | **A separate capture-only credential** (`ALMANAC_CAPTURE_TOKEN`) that authorizes posting a capture and nothing else. The bootstrap token still works for the operator. What must never happen is the reverse, and cannot: nothing else consults the capture token. |
| AR33 | The worker's backoff, recovery and warn-once logic lived inline in an async loop with hardcoded intervals, so none of it could be tested — and its failure modes are silent ones: staying on half-hourly polling forever after one blip, or reporting the same backlog every fifteen seconds until the alert is worthless. | **The pacing is a pure state machine** in `core::pacing`; the worker keeps only the I/O. Measuring whether the journal is filling is I/O and stays in the shell; deciding what to do about it does not. |
| AR34 | The listener's address was a compile-time constant, so two instances could not coexist even briefly, changing the port needed a rebuild, and the graceful-shutdown path could not be tested at all. | **`ALMANAC_BIND`, defaulting to the previous value.** A configuration knob that changes nothing in production and makes M2 provable. |

### Decisions taken while building L5 (2026-08-28)

| ID | Question | Decision |
|---|---|---|
| AR27 | AR23, AR24 and AR26 all require a notification, and Almanac has no notification channel. | **Reuse Home Assistant's homelab-ops webhook** rather than build a second channel. It already archives every event to `/media/homelab_events.log`, mirrors it to the Homelab dashboard tab, and pushes *only failures*, and only when `input_boolean.homelab_event_notifications` is on — so Almanac's events inherit Do Not Disturb, the active-hours schedule and the acknowledgement bus for free. The payload shape (`op`/`ok`/`version`/`error`) is the one that automation already parses, and `op` doubles as the deduplication key, so a repeated failure collapses to one line instead of stacking. Consequence accepted: the webhook id is the only authentication and it is `local_only`, so nothing sensitive is ever sent through it. |
| AR28 | Where do releases live for the updater to find? | **GitHub Releases**, because the repository is already there and it means no extra host to keep alive. Discovery is one plain asset — `latest/download/VERSION` — rather than the GitHub API: no token, no rate limit, and nothing to parse that an attacker could confuse. The updater is not otherwise GitHub-specific; any host serving `latest/download/VERSION` and `download/v<version>/<file>` works, which is exactly what the E2E test does. |
| AR29 | The `--check` probe has to decide what "can this version start here?" means. | **Everything that can differ between two versions on one machine, and no network.** It loads profiles, checks the secrets Latch injects, and proves the key opens the token store — then exits. It deliberately does not call Google, so the answer is about this build and this configuration rather than about whether Google happens to be reachable in that second. It also takes neither the port nor the data-directory lock, both of which the running process still holds. |
| AR30 | How does the process get from "new binary installed" to "running the new binary"? | **Raise SIGTERM on itself and let the supervisor restart it.** That reuses M2's graceful drain exactly as `systemctl restart` would, instead of adding a second shutdown path that would have to be kept correct in parallel. It also means a failure to restart looks like any other stopped unit, which is already monitored. |

Also accepted without needing a decision: the version number has no
single source (the binary says 0.1.0, the only tag is v0.0.1, and
`make tag-minor` never touches Cargo.toml) so an updater has nothing
to compare against; the Dockerfile as committed cannot run Almanac
(no WORKDIR, no volume, and it still copies a `config.toml` nothing
reads which names Kenny's personal calendar); and neither `compact()`
nor the token store fsyncs the parent directory after its atomic
rename, so a real power cut can lose the rename. All are fixed in L5,
and a genuine reboot drill on the throwaway LXC is added — the
existing power-loss drill simulates the crash in userspace and no real
power cut has ever been exercised.


## AR15 amendment (2026-09-03) — the profile stops describing the event

AR15 froze `source_id` as a source's immutable identity and the upsert
key's shape (`<source_id>:<external-id>`). Both stand unchanged.

What changes around them: a profile no longer says *what a payload
means*. Per-event choices — all-day, colour, free/busy, status,
reminders, length, timezone — travel in the call, because they are
things the source knows per event and the profile could only fix once
for all of them (K23). The profile keeps exactly what belongs to the
source rather than to the event: its identity, its calendar, and two
defaults.

The upsert key is unaffected in shape and stronger in practice. It used
to be absent whenever a profile named no external id field, and an event
without it can never be found again — no update, no delete. Ingest now
refuses a call carrying neither an `external_id` nor an
`Idempotency-Key` header, so the key exists for every event Almanac
creates rather than merely usually.

`schema_version` goes to 2. A v1 profile is refused with a message
saying what changed and what to do, rather than being read as if its
`[mapping]` block were noise — which is what a silent upgrade would have
done to every deployed profile at once.
