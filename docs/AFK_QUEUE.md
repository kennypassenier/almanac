# AFK queue

Kenny is away; per PROCEDURE's AFK rule, work continues while the plan
is followed, and anything needing a deviation from a frozen decision is
quarantined here rather than silently built. **This file is the first
thing to present when he returns.**

Session started AFK: 2026-08-28, after the L5 Latch decisions.

## Pending mini-rounds

_none yet_

## Needs one action from Kenny before Almanac can run anywhere

- **`ALMANAC_SECRET_KEY` is not in Latch.** Found by running `--check`
  against the real secrets: Latch supplies the three Google values but
  not the key that encrypts the per-source tokens, so the service
  cannot start. It is a fresh random value, not a recovered one:

      latch set ALMANAC_SECRET_KEY "$(openssl rand -hex 32)"

  Deliberately not done here even though it is a single command — it
  mints a production secret that must then stay stable forever, and
  minting it is part of the rollout Kenny gated.

## Deliberately not done (needs Kenny)

- **Generating the minisign release key** and putting its public half
  in `RELEASE_PUBKEY` (`src/shell/update.rs`). Until then self-update
  refuses to run rather than installing something it cannot verify —
  the fail-closed direction. `docs/OPERATIONS_RUNBOOK.md` R6 has the
  three commands.
- **A live self-update between two real builds**, which needs the real
  signing key and a machine to restart on. The mechanism is proven
  end-to-end against a local mock release with a throwaway key (nine
  tests, including a tampered binary, a wrong signing key and a version
  that cannot start here), and `--check` is proven against the real
  binary and the real Latch secrets. What is unproven is the last step:
  SIGTERM → systemd → new binary. That belongs to the LXC drill.

- **The deployment itself.** Kenny chose "build everything first, roll
  out as a separate step with a go per action" (D9). Creating the
  throwaway LXC, provisioning the real one, and the Traefik route all
  wait.
- **Signing a real release.** The minisign secret key is offline and
  Kenny's. The release flow is built and testable against a locally
  generated throwaway key; the first genuine signed release is his.
- **The two Home Assistant manual steps** (the `rest_command` snippet
  and installing `script.almanac_send`), which he scheduled for
  deployment time.

## Completed while AFK

Two commits on `main`, CI green (run 33199983137):

- `0b92213` — version scheme (M8), release signing (AR19), systemd unit
  and compose file (AR20), a working Dockerfile, and the startup key
  check the Latch project's L3 decision asked for.
- `9fe80ce` — full self-update (M10): signature-before-download,
  checksum against the signed manifest, the `--check` probe (AR22), and
  automatic revert with notification (AR23). Plus the notification
  channel (AR27), the release layout (AR28), what `--check` means
  (AR29) and how the restart happens (AR30) — four decisions taken
  while building, all recorded and all in the report form.
- README rewritten (it still described cal-stacean, Vikunja, Infisical
  and five endpoints that do not exist) and `docs/OPERATIONS_RUNBOOK.md`
  written — the file AR24 and `sign-release.sh` were already pointing at.

Three bugs found and fixed on the way: the systemd unit made the
install directory read-only so every self-update would have failed at
the swap; journal compaction never fsynced its parent directory; and
executing a just-written binary can fail with `ETXTBSY` when another
thread forks at the wrong moment.

169 unit + 9 self-update E2E + 15 dashboard + 11 admin + 7 ingest + 3
fixture + 1 secrets-scan tests green. `--check` verified against the
real binary with the real Latch secrets.
