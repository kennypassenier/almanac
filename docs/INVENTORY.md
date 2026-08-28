# INVENTORY.md — Almanac (formerly cal-stacean)

Phase 1 output — brownfield inventory. Source of truth: the code as of
2026-08-28, primarily `src/main.rs` (1,681 lines), plus `Cargo.toml`,
`config.toml`, `Dockerfile`, `Makefile`, `.github/workflows/github-actions.yml`,
`.env.example`, `.infisical.json`, `README.md`,
`PROJECT_BOOTSTRAP_CHECKLIST_VERBOSE.md`.

Method: traced from the single entry point (`main()` in `src/main.rs`)
through every function it calls, then swept the rest of the file for
orphans. No automated tests exist anywhere in the repo, so "working" below
means "reasoned from the code," not "verified by a test suite."

Every item has a stable ID (`<DOMAIN>-<n>`) for Phase 2 to reference.
"Vikunja-specific" vs. "calendar-core" is called out explicitly per
`docs/SCOPE.md`'s recycle-vs-drop question.

---

## 0. Entry point & execution order

`main()` — `src/main.rs:1589-1681` (`#[tokio::main] async fn main`). Runs,
in strict sequence with no error recovery (any `?` failure aborts the
process before the HTTP server binds):

1. Load `.env` via `dotenvy::dotenv()` (best-effort, errors ignored) — `main.rs:1597`
2. Parse `config.toml` — `main.rs:1600`
3. Init tracing subscriber — `main.rs:1603`
4. Build a shared `reqwest::Client` — `main.rs:1618-1622`
5. Load service-account credentials from env vars — `main.rs:1627`
6. Obtain a Google OAuth2 bearer token (one-shot) — `main.rs:1636`
7. Construct `GoogleCalendarClient` — `main.rs:1641`
8. Wrap in `AppState`, share via `Arc` — `main.rs:1647`
9. Build the Axum `Router` (4 routes) — `main.rs:1653-1658`
10. Bind TCP listener on `0.0.0.0:8080` (hardcoded, not configurable) — `main.rs:1663-1666`
11. `axum::serve` — runs until process is killed — `main.rs:1676-1678`

No signal handling, no graceful shutdown, no health-check endpoint, no
readiness probe. There is exactly one binary target (no CLI subcommands,
no flags, no `--help`) — the only "CLI surface" is "run the binary in a
directory containing `config.toml` and, optionally, `.env`."

---

## 1. Configuration & startup

| ID | Feature | What it does | Where | State | Notes |
|---|---|---|---|---|---|
| CFG-1 | TOML config loading | Reads `config.toml` from the **current working directory** (not an absolute or configurable path) and deserializes into `Config`. | `main.rs:669-677`, struct `main.rs:164-180` | Working | `Dockerfile` sets `ENV CONFIG_PATH=/etc/cal-stacean/config.toml` (`Dockerfile:93`) but the code never reads `CONFIG_PATH` — the env var is dead documentation, the binary always reads `./config.toml` relative to its launch dir. In the container this happens to work only because `WORKDIR`/`CMD` context makes `config.toml` resolve from `/etc/cal-stacean` — actually it does **not**: CMD runs `/usr/local/bin/cal-stacean` with default workdir `/`, so `config.toml` would fail to load unless the container's workdir is separately set. This is a latent container bug. |
| CFG-2 | Config schema | `default_calendar_id` (string), `default_color_id` (string "1"-"11"), `log_level` (string), `standard_colors` (`HashMap<String,String>` colour-name → colour-ID). All fields required; missing key = hard startup failure. | `main.rs:162-180` | Working | Single hardcoded `default_calendar_id` — no concept of multiple calendars or per-source calendar selection. Directly blocks SCOPE.md's multi-calendar requirement; needs redesign, not reuse as-is. |
| CFG-3 | Tracing/logging init | Validates `log_level` against `tracing::Level`, builds an `EnvFilter` (env var `RUST_LOG` overrides config value), configures `tracing_subscriber::fmt()` with file+line number annotations, writes to stdout. | `main.rs:692-712` | Working | Plain stdout logging only — no log rotation, no structured sink, no shipping to Loki/Grafana (relevant to Kenny's home network's existing Loki stack, but out of scope for this file). |
| CFG-4 | `.env` loading | `dotenvy::dotenv()` loads a local `.env` file into process env before any `std::env::var()` call. Silently no-ops if the file is absent. | `main.rs:1597` | Working | Comment claims a permissions error "will surface through the tracing output" — false: tracing isn't initialized yet at this point in `main`, so such an error is actually swallowed with `.ok()` and never logged anywhere. |
| CFG-5 | Colour name→ID table | `standard_colors` in `config.toml` maps 11 named Google Calendar colours (`lavender`...`tomato`) to their numeric IDs 1-11. | `config.toml:24-35` | Working | Generic/reusable — calendar-core, not Vikunja-specific. |

---

## 2. Authentication (Google OAuth2, service account)

| ID | Feature | What it does | Where | State | Notes |
|---|---|---|---|---|---|
| AUTH-1 | Service-account credential loading | Reads three discrete env vars — `CLIENT_EMAIL`, `PRIVATE_KEY`, `TOKEN_URI` — expected to originate from a Google service-account JSON key, imported into Infisical or a local `.env`. | `main.rs:730-753` | Working | `.env.example` (`.env.example:1-11`) actually lists **10** keys (`AUTH_PROVIDER_X509_CERT_URL`, `AUTH_URI`, `CLIENT_EMAIL`, `CLIENT_X509_CERT_URL`, `PRIVATE_KEY`, `PRIVATE_KEY_ID`, `PROJECT_ID`, `TOKEN_URI`, `TYPE`, `UNIVERSE_DOMAIN`) mirroring the full Google service-account JSON, but the code only ever reads 3 of them. This is a docs/code contradiction — 7 vars are dead/unused by the running binary (harmless, just noise/confusion, likely a leftover from `infisical secrets generate-example-env` reflecting whatever's stored in Infisical, not what the code needs). |
| AUTH-2 | JWT construction & RS256 signing | Builds `GoogleClaims` (`iss`, `scope`, `aud`, `iat`, `exp`), normalizes literal `\n` sequences in the PEM private key to real newlines (Infisical-specific escaping quirk), signs with `jsonwebtoken` RS256. | `main.rs:774-807` | Working | JWT lifetime hardcoded to `JWT_LIFETIME_SECS = 3600` (`main.rs:57`) — the Google-enforced max, not configurable. |
| AUTH-3 | Token exchange | POSTs the signed JWT to `credentials.token_url` (`https://oauth2.googleapis.com/token`) using `grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer`, form-encoded per RFC 7523. Deserializes `access_token` + `expires_in`. | `main.rs:809-848` | Working | — |
| AUTH-4 | **Token obtained once, never refreshed** | The bearer token is fetched exactly once at startup (`main.rs:1636`) and stored as a plain `String` field on `GoogleCalendarClient` (`main.rs:260`) for the process lifetime. No refresh task, no expiry check, no retry-on-401. | `main.rs:254-265`, `1636-1641` | **Broken (known, confirmed defect)** | Google tokens expire after ~1 hour (`expires_in`, typically 3600s). After that, every Calendar API call fails with 401 for the remainder of the process's uptime — the server keeps running and keeps accepting HTTP requests, it just returns 500s forever. SCOPE.md success criterion 3 explicitly names this as a defect to fix, not carry forward. Calendar-core, must be redesigned regardless of source. |
| AUTH-5 | No credential rotation / no multi-account support | Exactly one service account, one set of credentials, loaded once. | `main.rs:718-753` | Working (as designed) but limiting | Fine for v1 single-account use; would need revisiting only if Almanac ever needs to act as multiple Google identities (not currently in scope). |

---

## 3. Calendar CRUD core (calendar-core — reusable regardless of source)

`GoogleCalendarClient` — `main.rs:254-559`. Wraps a shared `Arc<reqwest::Client>`, the current bearer token, and the parsed `Config`.

| ID | Feature | What it does | Where | State | Notes |
|---|---|---|---|---|---|
| CAL-1 | `create_event` | `POST {CALENDAR_EVENTS_BASE}/{calendarId}/events` with a `GoogleEvent` JSON body; returns the Google-assigned event incl. `id`. | `main.rs:290-332` | Working | Calendar always resolved from `self.config.default_calendar_id` — no per-call calendar override anywhere in the client API. This is the single biggest blocker to SCOPE.md's multi-calendar requirement. |
| CAL-2 | `get_event` | `GET .../events/{eventId}`, deserializes into `GoogleEvent`. | `main.rs:343-382` | **Dead code** | Annotated `#[allow(dead_code)]` (`main.rs:342`) — never called from any HTTP handler or webhook path. README advertises a `GET /api/v1/events/{id}` endpoint that does not exist in the router. Docs/code contradiction. |
| CAL-3 | `update_event` | `PUT .../events/{eventId}` — full resource replacement (not PATCH); caller must resend every field to retain it. | `main.rs:399-440` | Working | Reusable pattern, but full-PUT semantics need to survive into any rewrite (a partial-update caller would silently wipe fields). |
| CAL-4 | `delete_event` | `DELETE .../events/{eventId}`; treats any non-2xx (incl. 404/410) as an error. | `main.rs:454-486` | Working | Callers (Vikunja handler) pre-check existence via `find_event_by_property` before deleting, so the "410 Gone is an error" behavior is never actually hit in practice — but it's still a latent rough edge for a generalized client. |
| CAL-5 | `find_event_by_property` | `GET .../events?privateExtendedProperty={key}={value}` — Google-native filter restricted to events created by the same OAuth client; returns first match only. | `main.rs:505-558` | Working | **This is the core "upsert" pattern SCOPE.md wants generalized** — using a private extended property (currently hardcoded key `"vikunja_task_id"`) as an external-ID lookup so updates/deletes find the right event without Almanac needing its own state store. Calendar-core in intent, but the *key name* is Vikunja-specific (see VIK-3 below) — needs to become a per-mapping-profile parameter (e.g. `source_id`, `home_assistant_uid`, etc.). |
| CAL-6 | `GoogleEvent` model | Rust struct mirroring the Google Calendar v3 Event resource subset: `id`, `summary`, `description`, `location`, `colorId`, `start`/`end` (`EventDateTime`: RFC3339 `dateTime` + IANA `timeZone`), `extendedProperties.private` (`HashMap<String,String>`). CamelCase JSON via `serde(rename_all)`. `Option` fields skip serialization when `None`. | `main.rs:78-144` | Working | Only `extendedProperties.private` is used; `.shared` is deliberately omitted. No recurrence (`recurrence` RRULE), no attendees, no reminders — a fairly thin slice of the full Event schema. Reusable as the calendar-core data model, extendable later. |
| CAL-7 | Event-list response wrapper | `EventListResponse { items: Option<Vec<GoogleEvent>> }` — handles Google's quirk of omitting the `items` key entirely (not an empty array) when no results match. | `main.rs:152-156` | Working | — |
| CAL-8 | No pagination handling | `find_event_by_property` (and by extension any future "list events" feature) does not follow `nextPageToken`; only the first page Google returns is ever considered. | `main.rs:505-558` | Gap | Not currently a practical problem (queries are always scoped to one property match, expected to return 0-1 results), but would silently misbehave if ever reused for broader listing/search. |
| CAL-9 | No timezone handling beyond UTC | Every event built by the app hardcodes `time_zone: "UTC"` (`main.rs:1078,1082,1098,1102`); the `EventDateTime` struct supports arbitrary IANA zones but nothing in the code ever sets one. | `main.rs:80-86`, `1060-1107` | Gap | Fine for a homelab where Kenny is the only consumer and reads times in his calendar app's own local rendering, but worth flagging for the mapping-profile design (a source's local time semantics may not always be UTC-safe, e.g. DST edge cases). |

---

## 4. HTTP API surface & security

Router — `main.rs:1653-1658`. Exactly 4 routes, all `POST`, all sharing one `AppState` via `Arc`.

| ID | Route | Handler | What it does | Where | State |
|---|---|---|---|---|---|
| HTTP-1 | `POST /api/v1/events/create` | `create_event_handler` | Deserializes body as `GoogleEvent`, calls `create_event`, returns `201` + `{status, event_id}` or `500` + error envelope. | `main.rs:870-916` | Working |
| HTTP-2 | `POST /api/v1/events/update/{id}` | `update_event_handler` | Path param `id` + full `GoogleEvent` body, calls `update_event` (PUT semantics), returns `200` + `{status, event_id, updated_summary}`. | `main.rs:918-965` | Working |
| HTTP-3 | `POST /api/v1/events/delete/{id}` | `delete_event_handler` | Path param `id`, no body, calls `delete_event`, returns `200` + `{status, deleted_event_id}`. | `main.rs:967-1009` | Working |
| HTTP-4 | `POST /webhooks/vikunja` | `vikunja_webhook_handler` | See §5 (Vikunja integration). | `main.rs:1166-1578` | Working, Vikunja-specific |

| ID | Cross-cutting HTTP feature | Notes |
|---|---|---|
| HTTP-5 | **No authentication/authorization on any endpoint.** Every route is reachable by any host that can reach port 8080 — no bearer token, no API key, no mTLS, no IP allowlist. | Confirmed by direct code reading: no auth middleware/layer is registered anywhere in the `Router` construction (`main.rs:1653-1658`), and no handler checks headers before acting. SCOPE.md explicitly calls this out as a known gap to close (per-source bearer tokens via Latch) — **the single highest-severity item in this inventory** given the current network is LAN-only but still multi-tenant (anyone on the LAN can create/delete arbitrary calendar events today). |
| HTTP-6 | No rate limiting, no request size limits (beyond Axum/Hyper defaults), no CORS policy configured. | Axum defaults apply silently; nothing explicit in code. |
| HTTP-7 | Uniform JSON error envelope | All handlers return `(StatusCode, Json<Value>)` via the `HandlerResult` type alias (`main.rs:859`), body shape `{"status": "error", "message": "..."}` or `{"status": "success", ...}`. Consistent, reusable pattern regardless of source — calendar-core / gateway-core. | `main.rs:854-859` |
| HTTP-8 | README documents endpoints that don't exist / don't match | README's "API Endpoints" section (`README.md:140-145`) lists `POST /api/v1/events` (no `/create` suffix), `GET /api/v1/events/{id}`, `PUT /api/v1/events/{id}`, `DELETE /api/v1/events/{id}`, `GET /api/v1/events?query=...` — **none of these five match the actual router**, which uses `/api/v1/events/create`, `/api/v1/events/update/{id}` (POST not PUT), `/api/v1/events/delete/{id}` (POST not DELETE), and has no GET/list endpoint at all. Docs/code contradiction — README is aspirational/stale, not evidence. |
| HTTP-9 | No health/readiness endpoint | Nothing like `GET /healthz` exists. Relevant given SCOPE.md's requirement that Almanac "survives a restart... without manual intervention" on Proxmox — no way to externally verify liveness today besides trying a real route. |
| HTTP-10 | No introspection/debug surface | SCOPE.md non-goals explicitly require *some* debug/status/query surface even without a UI; none exists in the current code at all (not even structured request logging beyond `tracing::info!` per-handler, which is present — see CFG-3). |

---

## 5. Vikunja integration (Vikunja-specific — "drop the coupling, keep the pattern" per SCOPE.md)

| ID | Feature | What it does | Where | State | Notes |
|---|---|---|---|---|---|
| VIK-1 | `VikunjaAction` enum | Deserializes Vikunja's dot-separated webhook action strings: `task.created`, `task.updated`, `task.deleted`, `task.overdue`, `tasks.overdue`, plus a catch-all `Unknown` (serde `#[serde(other)]`) for forward-compat with unrecognised actions. | `main.rs:586-615` | Working, Vikunja-specific | The "catch-all + 200 OK no-op" pattern (never 422 on unknown payload shapes) is a good generic webhook-ingestion pattern worth recycling. |
| VIK-2 | `VikunjaTaskData` / `VikunjaPayload` | Deserializes the webhook body: `id` (i64), `title`, `description` (defaults to empty string), optional `due_date` (RFC3339), optional `priority` (1-5). Extra fields Vikunja sends are silently ignored by serde. | `main.rs:617-657` | Working, Vikunja-specific | Field names/shape are entirely Vikunja's schema — not reusable as-is, but the "typed struct with `#[serde(default)]` tolerance" pattern is. |
| VIK-3 | `build_google_event_from_task` — field mapping | Maps `title→summary`, `description→description` (omitted if empty), `due_date→start` (+1h for `end`) or falls back to today-noon-UTC to 1pm placeholder window if no due date, `priority→colorId` via VIK-4, and injects `task.id` as `extendedProperties.private["vikunja_task_id"]`. | `main.rs:1060-1139` | Working, Vikunja-specific | This function is exactly the shape a "mapping profile" needs to become generic/configurable: field-mapping + time-window derivation + colour derivation + external-ID tagging. The hardcoded property key `"vikunja_task_id"` (line 1120) is the one piece that must become a per-profile parameter (e.g. `"{source}_id"`) for CAL-5 to generalize. |
| VIK-4 | `priority_to_color_id` | Maps Vikunja priority 1-5 to a Google colour ID by looking up a fixed name array (`sage, peacock, banana, tangerine, tomato`) against `config.standard_colors`, falling back to `config.default_color_id` if a name is missing from config. | `main.rs:1031-1043` | Working, Vikunja-specific | The priority→colour *mapping table* is Vikunja's semantics (Vikunja's own 1-5 priority scale); the *mechanism* (name lookup against a configurable colour table with a safe fallback) is calendar-core and reusable by any future profile that wants severity/priority colour-coding (e.g. Uptime Kuma incident severity). |
| VIK-5 | `vikunja_webhook_handler` — dispatch table | Routes each `VikunjaAction` to CRUD operations: `task.created`→create; `task.updated`→find-by-property then update-if-found-else-create (upsert); `task.deleted`→find-by-property then delete-if-found-else-no-op (idempotent); `task.overdue`/`tasks.overdue`→find-by-property then re-map+force-tomato-colour+update, or no-op if not found; `Unknown`→200 no-op. | `main.rs:1166-1578` | Working, Vikunja-specific | The **upsert-by-external-ID dispatch pattern** (lookup → branch create-vs-update, lookup → branch delete-vs-noop) is precisely what SCOPE.md calls out as "the pattern already proven... generalized instead of hardcoded." This is the highest-value piece of VIK code to recycle into the mapping-profile engine; the Vikunja-specific action names and field mapping are not. |
| VIK-6 | Overdue colour override | On `task.overdue`/`tasks.overdue`, forces `color_id` to the `"tomato"` entry in `standard_colors` (falling back to literal `"11"` if absent), overwriting whatever `priority_to_color_id` would have produced. | `main.rs:1483-1505` | Working, Vikunja-specific | One-off business rule specific to Vikunja's overdue semantics; not directly reusable, but demonstrates a need the mapping-profile design should support generically: "conditional colour override rules," not just static priority mapping. |
| VIK-7 | No webhook signature verification | Vikunja supports signing webhook payloads (HMAC), but the handler performs no signature check at all — the endpoint trusts any POST body that parses as `VikunjaPayload`. | `main.rs:1166-1178` | Gap / defect, Vikunja-specific | Moot once Vikunja is dropped, but the *absence of source-authentication on inbound webhooks* is the same root issue as HTTP-5 and should inform the new bearer-token design for all future source integrations. |

---

## 6. External dependencies (`Cargo.toml`)

| Crate | Version | Used for |
|---|---|---|
| `tokio` | 1, `full` | Async runtime, TCP listener, `#[tokio::main]`. |
| `serde` / `serde_json` | 1 | (De)serialization for config, HTTP bodies, Google API payloads. |
| `toml` | 0.8 | Parses `config.toml`. |
| `dotenvy` | 0.15 | Loads `.env` into process environment. |
| `tracing` / `tracing-subscriber` (env-filter) | 0.1 / 0.3 | Structured logging to stdout. |
| `reqwest` | 0.12, `json` + `rustls-tls` (no default features — avoids system OpenSSL at the crate level, though the Dockerfile builder still installs `libssl-dev`/`libssl3`, suggesting either leftover boilerplate or a transitive dependency still needing OpenSSL; worth double-checking during a rewrite). | HTTP client for Google OAuth2 token endpoint and Calendar v3 API. |
| `jsonwebtoken` | 9 | RS256 JWT construction/signing for the service-account flow. |
| `axum` | 0.8 | HTTP server/router framework. |
| `chrono` | 0.4, `serde` | RFC3339 date parsing/arithmetic for `due_date` → event window mapping. |

No test-only dependencies (no `mockito`, `wiremock`, `assert`-style crates) — consistent with "no tests exist anywhere."

---

## 7. Storage formats

| ID | Format | Purpose | Notes |
|---|---|---|---|
| STORE-1 | `config.toml` | Static app config: default calendar, default colour, log level, colour name table. | Plain TOML, no schema versioning, no validation beyond serde's own type-checking (an invalid `log_level` string does fail with a clear error — `main.rs:695-700`). |
| STORE-2 | `.env` | Runtime secrets: 3 vars actually consumed (`CLIENT_EMAIL`, `PRIVATE_KEY`, `TOKEN_URI`); `.env.example` lists 10 (see AUTH-1). | Not committed (`.gitignore:2`); generated by Infisical in CI and locally via `make secrets`. |
| STORE-3 | No database, no on-disk state, no cache | Google Calendar itself is the only persistence layer — the app is fully stateless between requests, relying entirely on `extendedProperties.private` tags stored inside Google's own event objects for "which task maps to which event" lookups (see CAL-5). | This statelessness is a deliberate, reusable architectural property worth explicitly preserving in the rewrite. |

---

## 8. Network endpoints (outbound)

| ID | Endpoint | Purpose | Where |
|---|---|---|---|
| NET-1 | `https://oauth2.googleapis.com/token` (from `TOKEN_URI` env var, not hardcoded — though `.env.example` and docs imply it's always this exact value) | OAuth2 JWT-bearer token exchange. | `main.rs:743-746`, `809-824` |
| NET-2 | `https://www.googleapis.com/calendar/v3/calendars/{calendarId}/events[/​{eventId}]` (`CALENDAR_EVENTS_BASE`, `main.rs:67`) | All Calendar CRUD + search calls. | `main.rs:63-67`, throughout §3 |

Inbound: `0.0.0.0:8080` (see §4). No outbound calls to Vikunja itself — the integration is receive-only (webhook consumer), Almanac never calls back into Vikunja.

---

## 9. Build, packaging & CLI/UI surface

| ID | Feature | What it does | Where | State | Notes |
|---|---|---|---|---|---|
| BUILD-1 | Cargo package name | `name = "cal-stacean"` | `Cargo.toml:2` | As-is per task instructions — do not rename in Phase 1. |
| BUILD-2 | Makefile `BINARY_NAME` | `cal-stacean` | `Makefile:9` | Same — untouched. |
| BUILD-3 | `make secrets` | Runs `infisical export --env=dev --format=dotenv > .env`. | `Makefile:67-70` | Working, Infisical-coupled | Direct dependency on Infisical CLI being installed and authenticated locally. |
| BUILD-4 | `make example-env` | Runs `infisical secrets generate-example-env > .env.example`. | `Makefile:72-75` | Working, Infisical-coupled | Explains why `.env.example` has 10 keys regardless of what code actually reads (AUTH-1) — it reflects whatever's stored in the Infisical project, not the code's actual env-var contract. |
| BUILD-5 | `make build` | Depends on `secrets example-env`, then `cargo build --release` and copies the binary to the repo root as `./cal-stacean`. | `Makefile:77-81` | Working | **This is why a 7.3 MB ELF binary is currently committed to git at the repo root** (`/home/kenny/Projects/almanac/cal-stacean`, confirmed via `git ls-files`) — `make build`'s own `cp` step places it exactly where `.gitignore` does *not* exclude it (`.gitignore` only excludes `/target` and `Cargo.lock`, not the bare `cal-stacean` filename). `.dockerignore` *does* exclude it from the Docker build context (`​.dockerignore:9-10`), but that has no bearing on git. This is a real defect: a stale, non-reproducible binary artifact is checked into version control. |
| BUILD-6 | `make run` | Depends on `build`, then executes `./cal-stacean` directly (relies on `.env`/`config.toml` being present in cwd — see CFG-1/CFG-4). | `Makefile:84-86` | Working | — |
| BUILD-7 | `make tag-major` / `make tag-minor` | Parses the latest `git describe --tags` semver tag, bumps major or minor (resets lower components to 0), creates a local git tag; does **not** push it (`README.md` says to run `git push --tags` manually). | `Makefile:106-118` | Working | No `make tag-patch` target actually exists in the Makefile despite being referenced by `help` (`Makefile:55` only documents `tag-minor`/`tag-major`) and by `README.md:63,163` and `PROJECT_BOOTSTRAP_CHECKLIST_VERBOSE.md`. **Docs/code contradiction**: `tag-patch` is documented in two places but does not exist as a Makefile target. |
| BUILD-8 | `help` target | Prints available targets and computed `FULL_IMAGE`/`LATEST_IMAGE` strings. | `Makefile:46-60` | Working | Text mentions `docker-build`, `docker-login`, `docker-push` targets (`Makefile:58-60` list `help` output referencing these, and `README.md:58-60` documents them) — **none of these three targets actually exist in the Makefile.** Another docs/code contradiction; Docker build/push only happens in CI (see CI-*), not via `make`. |
| BUILD-9 | Malformed Makefile — `clean` target lost its header | Lines 120-125 (`@echo "Cleaning..."`, `cargo clean`, `rm -f` x3) are **not** attached to any target declaration — they sit as orphaned recipe lines directly under the `tag-minor` target's body (no blank line + new `target:` line separates them), meaning `cargo clean`/`rm -f $(BINARY_NAME)` etc. silently execute as part of `make tag-minor` every time it's invoked, and there is no separately invokable `make clean` at all despite `.PHONY` (`Makefile:33-34`) listing no `clean` and README (`README.md:54`) documenting `make clean` as a real command. | `Makefile:113-125` | **Broken** | Concretely: running `make tag-minor` creates a git tag *and then* wipes `target/`, the built binary, `.env`, and `.env.example` as an undocumented side effect. This is a genuine bug, not just a docs mismatch — worth flagging strongly for Phase 2/5. |
| BUILD-10 | `Dockerfile` — multi-stage build | Stage 1 (`rust:1.88-slim`) compiles `cargo build --release --locked`; stage 2 (`debian:bookworm-slim`) copies only the binary + `ca-certificates`/`libssl3` + a baked-in `config.toml`. | `Dockerfile:24-104` | Working (unverified — not built during this inventory sweep, read-only) | Image/binary name still `cal-stacean` per task instructions — untouched. `CONFIG_PATH` env var is set but unused by the code (see CFG-1) — the container likely doesn't actually pick up `/etc/cal-stacean/config.toml` unless the container's working directory happens to default there, which it does not (no `WORKDIR` set in the runtime stage) — likely broken at actual container-run time, not just cosmetically stale. |
| BUILD-11 | No CLI args / no subcommands / no `--help` | The binary takes zero command-line arguments; behavior is entirely file- and env-driven. | `main.rs` (absence throughout) | As designed | The only "surface" to interact with the running process is the 4 HTTP routes (§4). |
| BUILD-12 | No UI of any kind | Confirmed — no templates, no static assets, no frontend code anywhere in the repo. | n/a | As designed, matches SCOPE.md's "no UI for v1" | — |

---

## 10. CI/CD pipeline (`.github/workflows/github-actions.yml`)

Single workflow, "CI/CD Pipeline" name convention (per bootstrap checklist), triggers on push to `main` (ignoring markdown-only changes) and manual `workflow_dispatch`. **13 sequential steps, no test step at all** — despite README claiming `cargo test` is part of the dev workflow (`README.md:55`), CI never runs it and no test files exist to run.

| ID | Step | What it does | Where | Notes |
|---|---|---|---|---|
| CI-1 | Checkout | `actions/checkout@v4` using `CR_PAT` (a GitHub PAT), not the default `GITHUB_TOKEN`. | `github-actions.yml:20-23` | Needed so later steps can push commits/tags with elevated permissions. |
| CI-2 | Install Rust | `actions-rs/toolchain@v1`, stable. | `github-actions.yml:28-32` | `actions-rs/toolchain` is an unmaintained/archived third-party action — worth flagging as a supply-chain/maintenance risk for Phase 5, independent of the Infisical→Latch swap. |
| CI-3 | Install Infisical CLI | `curl \| sudo bash` installer + `apt-get install infisical`. | `github-actions.yml:36-39` | Infisical-coupled — dropped entirely once Latch replaces it (per SCOPE.md hard constraint). |
| CI-4 | Fetch secrets → `.env` + verbose logging | `infisical export --env=prod --projectId=... --format=dotenv > .env`, then prints exit code, key names (not values), line count, and a redacted-value dump of `.env` to the workflow log. | `github-actions.yml:44-58` | Infisical-coupled | The redaction logic (`awk` printing `KEY=[REDACTED]`) is a reasonable pattern to preserve in whatever Latch-based CI step replaces this — worth keeping the "never print secret values" habit even though the mechanism changes. |
| CI-5 | Generate `.env.example` | `infisical secrets generate-example-env --projectId=...`. | `github-actions.yml:64-65` | Infisical-coupled |
| CI-6 | Verify `.env` matches `.env.example` keys | Loops over `.env.example` keys, fails the build if any is missing from `.env`. | `github-actions.yml:68-81` | Reusable pattern in spirit ("fail CI if required secrets are missing") regardless of which secrets tool provides them. |
| CI-7 | Commit & push `.env.example` if changed | Configures a bot git identity, commits/pushes `.env.example` back to `main` directly from CI if it drifted. | `github-actions.yml:85-93` | CI pushing directly to `main` outside of PR review — a process choice worth re-examining in Phase 5, independent of secrets tooling. |
| CI-8 | Build release binary | `cargo build --release`. | `github-actions.yml:97-98` | — |
| CI-9 | Copy binary to repo root | `cp target/release/cal-stacean ./cal-stacean`. | `github-actions.yml:102-103` | This is a **second** source of the committed root-level binary artifact (see BUILD-5) — but note CI does not `git add`/commit it (only `.env.example` gets pushed back, step CI-7), so the committed binary currently in the repo was placed there by a **local** `make build` run, not by CI. Confirmed via `git log`: the tracked `cal-stacean` binary exists in the repo tree today. |
| CI-10 | Auto-bump patch version & tag | Reads latest `v*` tag via `git describe`, increments patch, creates and **pushes** the new tag directly — on every single push to `main`, unconditionally. | `github-actions.yml:105-124` | **Working but aggressive**: every merge to main, no matter how small, mints and pushes a new patch version tag automatically. No semantic-versioning judgment call — purely mechanical patch bump. Worth a deliberate decision in Phase 5 (keep automatic tagging vs. move to manual/conventional-commit-driven bumps). |
| CI-11 | Upload build artifact | `actions/upload-artifact@v4`, named `cal-stacean-binary`. | `github-actions.yml:128-132` | — |
| CI-12 | Clean build artifacts | `cargo clean`. | `github-actions.yml:135-136` | — |
| CI-13 | GHCR login | `docker login ghcr.io` using `CR_PAT`. | `github-actions.yml:140-141` | — |
| CI-14 | Build & push Docker image | `docker build -t ghcr.io/<owner>/cal-stacean:latest .` then `docker push`. | `github-actions.yml:144-151` | **Only ever pushes the `:latest` tag** — despite CI-10 minting a semver git tag in the same run, the Docker image is never tagged with that same version string, only `latest`. So there is no way to pull "the Docker image matching git tag v0.4.2" — a real gap between the versioning scheme and the artifact that's actually shipped. |

---

## 11. Secrets management (Infisical) — full surface, to be replaced by Latch

| ID | Item | Where | Notes |
|---|---|---|---|
| SEC-1 | `.infisical.json` — workspace binding | `.infisical.json:1-5` | Contains `workspaceId` (a UUID) and empty `defaultEnvironment`/`gitBranchToEnvironmentMapping`. Not itself secret, but ties the repo to a specific Infisical project. |
| SEC-2 | Local secret fetch | `make secrets` (BUILD-3) | Requires local Infisical CLI + authenticated session. |
| SEC-3 | CI secret fetch | CI-3/CI-4 | Requires `INFISICAL_TOKEN`, `INFISICAL_API_URL`, `INFISICAL_PROJECT_ID` as GitHub Actions secrets (per `PROJECT_BOOTSTRAP_CHECKLIST_VERBOSE.md:62-73`). |
| SEC-4 | `.env.example` auto-generation | CI-5, BUILD-4 | Both driven directly by Infisical CLI, not by static code introspection — meaning `.env.example` reflects whatever is *stored in Infisical today*, not what `main.rs` actually reads (see AUTH-1 mismatch). |
| SEC-5 | Docs prescribing the Infisical flow | `README.md:123-135`, all of `PROJECT_BOOTSTRAP_CHECKLIST_VERBOSE.md` | The bootstrap checklist is a generic template doc ("copy this file to each new project"), not specific to this app — it's process documentation, not application behavior; still an accurate description of the *current* CI/local secret flow that Latch must replace end-to-end (local fetch, CI fetch, GH Actions secrets, `.env.example` generation, and the PAT-based GHCR/push flow that piggybacks on the same CI credentials chain). |

Per SCOPE.md hard constraints, Infisical is being dropped entirely in favor of Latch; every SEC-* item above is a concrete surface Latch's integration needs to cover an equivalent (or explicitly decide not to cover) for Phase 5.

---

## 12. Versioning & tagging scheme

| ID | Item | Where | Notes |
|---|---|---|---|
| VER-1 | Tag format | `vMAJOR.MINOR.PATCH` (e.g. `v0.4.2`) | `Makefile:98-118`, `github-actions.yml:105-124` |
| VER-2 | Two independent tagging mechanisms that can drift | (a) local `make tag-major`/`make tag-minor` (manual, not pushed automatically); (b) CI's fully automatic patch bump on every push to `main` (CI-10). | If a developer runs `make tag-minor` locally and pushes tags manually while CI is *also* auto-bumping patch on every push, the two mechanisms can race/diverge (e.g. CI bumps `v0.4.2`→`v0.4.3` on the same push that a human intended to be `v0.5.0`, unless the human's tag creation happens to land and get fetched before CI's `git describe` runs). Not a currently-observed failure, but a structurally fragile setup worth simplifying. |
| VER-3 | `Cargo.toml` version never bumped | `version = "0.1.0"` (`Cargo.toml:3`) | Static since project inception apparently — the git-tag-based versioning scheme (VER-1) is entirely decoupled from the Cargo package version, which nothing in the Makefile or CI ever touches. |

---

## 13. Known defects & gaps — consolidated

(Cross-referenced to the IDs above; this section exists purely as a flat, skimmable list for Phase 2 triage — details live with each item above.)

1. **AUTH-4 / no token refresh** — server silently starts failing all Calendar calls ~1h after boot. (SCOPE.md-confirmed defect.)
2. **HTTP-5 / no auth on any HTTP endpoint** — anyone on the LAN can create/update/delete calendar events. (SCOPE.md-confirmed defect.)
3. **VIK-7 / no Vikunja webhook signature verification** — moot after Vikunja is dropped, but same root cause as #2.
4. **CFG-2 / single hardcoded calendar** — blocks the multi-calendar requirement entirely; needs a real redesign, not a patch.
5. **CFG-1 & BUILD-10 / `CONFIG_PATH` env var is defined but never read** — the Docker image's config-path mechanism is dead/likely broken at actual runtime (no `WORKDIR` set in the runtime stage).
6. **BUILD-5 / a 7.3 MB compiled binary is committed to git** at the repo root, confirmed tracked (`git ls-files`), stale and non-reproducible.
7. **BUILD-9 / `make tag-minor` silently runs `cargo clean` + deletes the binary and both env files** as an orphaned, unlabeled recipe fragment — a real Makefile bug, not just doc drift.
8. **BUILD-7 / BUILD-8 — `tag-patch`, `docker-build`, `docker-login`, `docker-push` are all documented (README, checklist, even the Makefile's own `help` text) but do not exist as Makefile targets.**
9. **CI-14 / Docker image is only ever tagged `:latest`**, never with the semver tag CI mints in the same run — no way to pull a historically pinned image version.
10. **HTTP-8 / README's documented API endpoints don't match the actual router** (wrong paths, wrong HTTP verbs, a nonexistent GET/list endpoint).
11. **CAL-2 / `get_event` is dead code** (`#[allow(dead_code)]`), unreachable from any handler, contradicting the README's advertised `GET /api/v1/events/{id}`.
12. **AUTH-1 / `.env.example` lists 10 keys, code reads 3** — generated from whatever's in Infisical, not from the code's actual contract.
13. **No tests anywhere** — README documents `cargo test` as part of the workflow (`README.md:55`); zero test files exist in the repo, CI never runs `cargo test`. Directly contradicts SCOPE.md success criterion 5 ("CI is green — full test suite — on every push") as a target state, and is explicitly flagged there as current-state-to-be-fixed.
14. **CAL-8 / no pagination handling** on the one list-style API call in use.
15. **CAL-9 / hardcoded UTC** — no real per-source timezone handling despite the data model supporting it.
16. **VER-2 / two independently-triggered tagging mechanisms** (manual `make tag-*` vs. CI's automatic per-push patch bump) that can drift against each other.
17. **HTTP-9 / HTTP-10 / no health endpoint, no debug/introspection surface** — directly relevant to SCOPE.md's requirement that "no UI" must never become "no visibility."
18. **CI-2 / `actions-rs/toolchain@v1`** is an archived, unmaintained third-party GitHub Action — supply-chain risk independent of the Infisical swap.
19. **BUILD-10 (Dockerfile stage 1) installs `libssl-dev`/`libssl3`** despite `reqwest` being configured with `rustls-tls` and `default-features = false` (`Cargo.toml:26-29`), which should avoid an OpenSSL dependency — worth verifying whether this is dead weight in the image or a real transitive need before carrying it into a rewrite.

---

## 14. Recycle vs. drop — Vikunja-specific behavior mapped to the new mapping-profile design

Per `docs/SCOPE.md`: Vikunja itself is out of scope; the upsert/mapping *pattern* it proved is the template for Almanac's general mapping-profile engine.

| Vikunja-specific item | Recycle the underlying pattern? | Into what |
|---|---|---|
| VIK-1 `VikunjaAction` enum + catch-all `Unknown`→200-no-op | **Recycle** | Generic pattern: each source profile should tolerate unrecognised event/action types gracefully (log + 200, never 422/500) rather than rejecting unknown payload shapes outright. |
| VIK-2 typed, tolerant payload struct (`#[serde(default)]`, ignore unknown fields) | **Recycle** | Generic pattern for every future source's payload struct — defensive deserialization so a source adding a new field never breaks Almanac. |
| VIK-3 `build_google_event_from_task` (field mapping + time-window derivation + external-ID tagging) | **Recycle the shape, drop the specifics** | Becomes the template for a per-source "mapping profile": title/description/time-window/colour field mapping + external-ID tag, but declaratively configured (likely TOML per SCOPE.md) instead of hardcoded Rust per source. The hardcoded `"vikunja_task_id"` property key must become a profile-supplied parameter. |
| VIK-4 `priority_to_color_id` (name-lookup against configurable colour table with safe fallback) | **Recycle the mechanism** | Any severity/priority-driven colour rule (e.g. Uptime Kuma incident severity, Grafana alert level) can reuse "look up a named colour against `standard_colors`, fall back to `default_color_id`." The specific 1–5 Vikunja priority scale is dropped. |
| VIK-5 upsert dispatch (`find_event_by_property` → branch create/update, branch delete/no-op) | **Recycle — this is the core value** | This is explicitly named in SCOPE.md as "the pattern already proven in cal-stacean's Vikunja integration, generalized instead of hardcoded." Becomes the generic "does this source event already have a matching calendar event (by external-ID extended property)? update it; else create it" engine, parameterized by which property key each mapping profile uses. |
| VIK-6 conditional colour override (overdue→forced tomato) | **Recycle the concept, drop the specifics** | Demonstrates mapping profiles need to support conditional/override rules beyond static field mapping, not just Vikunja's overdue case specifically. |
| VIK-7 no signature verification on the Vikunja webhook | **Drop — but its absence informs HTTP-5's fix** | Not a pattern to recycle; rather, evidence that *all* inbound source integrations (webhook or REST) need Almanac-side, per-source bearer-token auth (Latch-issued) from day one of the rewrite, not bolted on later. |
| Vikunja's actual field names/action strings/priority scale | **Drop entirely** | Fully Vikunja-specific vocabulary; no reuse value once Vikunja itself is out of scope. |

