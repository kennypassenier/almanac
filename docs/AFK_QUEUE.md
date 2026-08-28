# AFK queue

Kenny is away; per PROCEDURE's AFK rule, work continues while the plan
is followed, and anything needing a deviation from a frozen decision is
quarantined here rather than silently built. **This file is the first
thing to present when he returns.**

Session started AFK: 2026-08-28, after the L5 Latch decisions.

## Pending mini-rounds

_none yet_

## Deliberately not done (needs Kenny)

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

_see the combined report_
