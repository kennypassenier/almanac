# Operations runbook

What to do when something happens. Written for the case where it is
2am, the notification woke you up, and you do not want to re-derive the
design from the code.

Everything here assumes the standalone LXC running under systemd
(`deploy/almanac.service`). The homelab-v2 compose path is different in
exactly one way: self-update is off and homelab v2 owns updates.

---

## R1 · Cut a release

```bash
make tag-minor          # bumps Cargo.toml, commits, tags
git push && git push --tags
cargo build --release
./scripts/sign-release.sh
gh release create v<version> dist/v<version>/* --title v<version> --generate-notes
```

`make tag-*` bumps `Cargo.toml` and tags in one step deliberately: the
version in the binary and the version in the tag have to agree, and
`scripts/check-version.sh` fails the build if they ever do not. That is
not pedantry — an updater that compares its own version against the
latest release either never updates or updates on every poll when those
two disagree.

Signing happens on your machine, never in CI. A checksum served from
the same host as the binary proves nothing; the signature is the only
thing standing between an unattended updater and a compromised release
host.

**The release is invisible until `VERSION` is attached to it.** That
one asset is how running instances discover a new version.

## R2 · First install on a fresh machine

```bash
# as root, on the machine itself
apt-get install -y ca-certificates git   # git: `latch run` reads a git clone
useradd --system --home-dir /opt/almanac --shell /usr/sbin/nologin almanac
mkdir -p /opt/almanac/{data,profiles} /etc/almanac
install -m 0755 almanac /opt/almanac/almanac
install -m 0755 latch /usr/local/bin/latch
cp fixtures/profiles/*.toml /opt/almanac/profiles/   # examples — see below
```

**The shipped profiles are examples, not configuration.** Their
`target_calendar_id` values (`primary`, `infra`) are placeholders that
exist so the regression tests have something to pin. Deployed as they
are, a Home Assistant event lands on the service account's own invisible
calendar and an alert fails permanently against a calendar that does not
exist — no data lost, but no calendar either. Replace the id in each
profile with a real one before pointing any source at the service.

To create the real calendars under the service account, which then owns
them and needs no manual step in the Google Calendar UI:

```bash
latch run -- cargo run --example create_calendars     # creates, idempotent
latch run -- cargo run --example inspect_calendar_access   # shows who can see what
```

A calendar the service account creates is **invisible to everyone else**
until it is shared. `inspect_calendar_access` shows the ACL of each; add
your own account with `share_calendars` (or once, by hand) or nothing
that lands there will ever be visible.

Latch needs its cached clone and a link. Copy the clone from your
desktop rather than logging Latch in here — the clone is ciphertext
only, so copying it puts nothing readable on the machine, and it means
no GitHub token lives on an unattended box:

```bash
# from your desktop
tar czf - -C ~/.latch repo | ssh <machine> 'mkdir -p /opt/almanac/.latch && tar xzf - -C /opt/almanac/.latch'
# on the machine: tell latch which project this directory is
printf 'repo = "kennypassenier/secrets"\n\n[[projects]]\nname = "almanac"\ndir = "/opt/almanac"\n' \
    > /opt/almanac/.latch/config.toml
chown -R almanac:almanac /opt/almanac && chmod 700 /opt/almanac/.latch
```

The one key that opens those secrets. Pipe it — never paste it on a
command line, where it is visible in `ps`:

```bash
# from your desktop; the value never appears on screen or on disk
latch key show --reveal | awk '/^value/{print $2}' | \
    ssh <machine> 'umask 077; read k; printf "LATCH_KEY_ALMANAC=%s\n" "$k" > /etc/almanac/latch.env'
```

Take the value from the `value` line, not from a pattern of your own:
the key is 68 hex characters, and an extraction that assumes 64 will
silently truncate it and produce "stored key has 32 bytes, expected 34"
much later.

```bash
cp deploy/almanac.service /etc/systemd/system/
systemctl daemon-reload && systemctl enable --now almanac
```

Before starting it, prove the configuration is complete:

```bash
sudo -u almanac latch run -- /opt/almanac/almanac --check
```

`--check` loads the profiles, checks every secret Latch injects, and
proves the key opens the token store — then exits. It takes neither the
port nor the data-directory lock, so it is safe to run against a
machine that is already serving.

## R3 · What self-update does

Every six hours, and never within five minutes of a start:

1. fetch `latest/download/VERSION`; stop if it is not newer;
2. fetch `SHA256SUMS` and its `.minisig`, and verify the signature
   **before downloading the binary**;
3. download the binary and check it against the signed manifest;
4. run the new binary with `--check`;
5. move the running binary to `almanac.prev`, put the new one in place,
   record that an update is unproven, and SIGTERM itself so systemd
   restarts into it.

It skips a cycle entirely while captured requests are still retained —
restarting mid-investigation would discard exactly the requests you
were looking at.

Watch it with:

```bash
journalctl -u almanac -f | grep -i updat
```

## R4 · "Update reverted" notification

The new version installed, restarted, and did not stay up for a minute.
The previous binary is already back in place and running; nothing needs
doing tonight.

```bash
systemctl status almanac                 # should be active, on the old version
journalctl -u almanac -n 200 --no-pager  # why the new one died
ls -l /opt/almanac/almanac*              # .prev is gone; it was moved back
```

The revert only happens after a second start with the update still
unproven, so a slow start is not mistaken for a broken one. Fix the
cause, cut a new release; do not re-publish the same version number.

If the notification says the previous binary **could not** be restored,
that is the one case needing hands: install a known-good binary from a
GitHub release by hand (R2's `install` line) and restart.

## R5 · "Release failed verification" notification

Raised after three consecutive failures, so a truncated download does
not wake you.

It means one of two things, and they need opposite responses:

- **the release host is serving something it should not** — do not
  install anything by hand until you know what happened;
- **the signing key changed** and running instances still carry the old
  public key — see R6.

Nothing was installed either way. The service is unaffected and still
running the version it had.

## R6 · The signing key is lost or regenerated

There is exactly one key, deliberately (AR24). A spare in the same
vault protects against rotation, not loss, and rotation only matters
across many machines — there is one.

So losing it is not an emergency, it is an afternoon:

```bash
minisign -G                                   # new key pair
# put RELEASE_PUBKEY in src/shell/update.rs = the base64 line of minisign.pub
```

Then cut a release with the new key (R1) and install that one build by
hand once (R2's `install` line + `systemctl restart almanac`). From
then on self-update works again, because the running binary now carries
the new public key.

Back up `~/.minisign/minisign.key` to Bitwarden. Losing it costs one
manual install; losing it *and* not noticing costs months of silently
skipped updates, which is why R5's notification exists.

## R7 · Latch and the key on the LXC

`/etc/almanac/latch.env` holds `LATCH_KEY_ALMANAC` — the per-project
key, not the passphrase. The passphrase would open every project's
secrets and the GitHub token; the project key opens Almanac's five
values and nothing else.

**Losing the key on the LXC is not catastrophic.** Latch keeps every
credential in one passphrase-encrypted escrow file held offline, so
recovery is `latch key restore`, or a `latch clone` from the desktop.
No token needs re-issuing.

Note, consciously accepted: vzdump backs up that key alongside the
encrypted store it opens, so **the backup is as sensitive as the
secrets themselves**. Treat it accordingly. The alternative — excluding
the key — produces a restore that stops halfway for manual work, which
is the last thing anyone wants during a real outage.

## R8 · "Journal backlog" notification

Deliveries have been failing long enough that the journal is over half
its cap. Once it fills, ingest starts refusing events and the sources'
own retries eventually give up — so this arrives while there is still
room to act, not after.

```bash
journalctl -u almanac -n 100 --no-pager | grep -i "delivery failed"
curl -H "Authorization: Bearer $ALMANAC_BOOTSTRAP_TOKEN" \
     http://localhost:8080/v1/debug/status
```

Usually Google is unreachable or the service account lost access to a
calendar. Nothing is lost while the journal has room; entries deliver
themselves once the cause is fixed. The worker backs off as the outage
lasts, up to half-hourly, and speeds back up the moment anything gets
through.

## R9 · Roll back on purpose

```bash
# Rename, never overwrite: writing over a binary that is executing
# fails with "Text file busy". A rename replaces the directory entry
# and leaves the running process on its old inode, which is exactly
# what the self-updater does.
install -m 0755 <old binary> /opt/almanac/almanac.new
chown almanac:almanac /opt/almanac/almanac.new
mv /opt/almanac/almanac.new /opt/almanac/almanac
systemctl restart almanac
```

Then either publish a newer release or the updater will put the newer
version straight back on its next check. To stop that:

```bash
systemctl edit almanac      # Environment=ALMANAC_SELF_UPDATE=off
```

## R10 · Two processes, one data directory

Almanac takes an `flock` on `data/.lock` at startup and refuses to
start if another process holds it. If you see that refusal, a previous
instance is still running — find it before doing anything else. Two
processes over one journal deliver the same event twice and can lose
delivery records.

`--check` does not take the lock, so it is always safe to run.

## R11 · Backup and restore

State lives in four places, and only one of them is on the LXC:

| What | Where | Restore |
|---|---|---|
| Profiles (examples) | git (`fixtures/profiles/`) | `git clone`, then set the real calendar ids |
| Real calendar ids | this deployment only | `cargo run --example inspect_calendar_access` lists them |
| Secrets | Latch escrow (offline) | `latch key restore` / `latch clone` |
| Journal (transient) | `/opt/almanac/data` | nothing to do — it is empty in steady state |
| Calendar data | Google | nothing to do |

Since 2026-08-29 the homelab also takes a nightly restic snapshot of
`/opt/almanac` and `/etc/almanac` to Drive. Worth knowing exactly what
that contains, because it is both halves of the same lock: `/etc/almanac`
holds `latch.env` with the project key, and `/opt/almanac` holds the
encrypted token store and the Latch clone the key opens.

That is not a new category of exposure — restic encrypts client-side
with AES-256 before anything leaves the machine, under Kenny's own
64-character password which lives in Bitwarden and never at Google, and
the same Drive already holds the homelab's full secrets vault under the
same encryption. What it does change is the inventory: whoever holds
that restic password now also holds lock and key for Almanac's secrets
in one place.

So a destroyed LXC is a rebuild, not a recovery: R2, restore the Latch
key, done. The journal is worth backing up only if it is non-empty at
the moment of the backup, which means deliveries were failing then.

## R12 · Metrics and what to alert on (M13)

`GET /metrics` on the same port, no token needed. Prometheus on CT 113
scrapes it:

```yaml
  - job_name: almanac
    static_configs:
      - targets: ["10.10.10.12:8080"]
```

Six series, all prefixed `almanac_`: events accepted, delivered,
failed, set aside, token refreshes, and the journal depth. Plus
`almanac_build_info{version="..."}`, which is how you see at a glance
which version the hub is actually on.

Two of these are worth an alert:

- `almanac_journal_pending` climbing and not coming back down means
  deliveries are failing. The hub is doing the right thing — that is
  what the journal is for — but nobody is looking at the reason yet.
- `almanac_journal_readable` at 0 means the scrape could not read the
  journal at all. The depth gauge is deliberately absent in that case
  rather than reported as zero, so an alert on `pending == 0` will not
  fire and hide it. Alert on this one separately.

`almanac_deliveries_failed_total` climbing while
`almanac_events_delivered_total` also climbs is a retry story, not an
outage. Only sustained failures with no deliveries are a problem.

Do not point Uptime Kuma at `/metrics` or Prometheus at `/healthz`.
Liveness is `/healthz` (JSON, watched by Uptime Kuma per AR21); metrics
are here (exposition format). Prometheus parses only its own format, so
pointed at `/healthz` it would report a perfectly healthy service as
permanently down.

## R13 · Is the self-updater still looking?

Every six hours, and five minutes after each start, the log gets one
line either way:

```
checked for a new release; already on the latest version=0.1.3
```

If that line has not appeared since the last restart plus five minutes,
the updater is not running — which is a real failure mode: it once sat
silent for six hours because the check interval was scheduled from
process start rather than from the end of the startup delay, and
nothing in the log distinguished that from working correctly.

To see when it last looked:

```bash
journalctl -u almanac | grep "checked for a new release" | tail -3
```

## R14 · Who installs new versions

Almanac updates itself, unless it is running from an image somebody
else builds.

**On the LXC (the live deployment):** it checks every six hours, and
five minutes after each start. It verifies the minisign signature and
the checksum, runs the new binary once with `--check` before trusting
it, keeps the old one as `almanac.prev`, and reverts if the new version
does not reach "serving" within a minute of starting. Nothing to
configure.

**In a docker image:** self-update switches itself off, and says so in
the log:

```
running inside a docker or podman image — self-update is off by
default, because a binary replaced inside a container is lost the
moment the container is recreated while looking identical to the image
it came from. Update by pulling a new image. Set ALMANAC_SELF_UPDATE=on
to override.
```

This is AR20 enforced by the binary rather than trusted to whoever
writes the compose file. A container that replaces its own binary keeps
running the new version until it is recreated, then silently goes back
to the image's version — and every diagnosis after that starts from the
wrong version number.

**LXC is not treated as an image.** An LXC container is a long-lived
machine with a filesystem that survives; a Docker container is a
rebuilt artifact. The check is specifically for Docker and Podman
(`/.dockerenv`, `/run/.containerenv`, and the OCI markers in
`/proc/1/cgroup`) and never for "am I in a container", which would
switch self-update off on exactly the machine it was built for.

**Turning it off or on by hand.** `ALMANAC_SELF_UPDATE` accepts
`off`/`false`/`0`/`no` and `on`/`true`/`1`/`yes`, case-insensitively.
Anything it does not recognise counts as **off** — a slipped finger
should not be what lets a process rewrite its own binary. An empty
value is the same as not setting it at all.

```yaml
# docker-compose.yml — the default already, stated for the reader
environment:
  ALMANAC_SELF_UPDATE: "off"
```

```yaml
# a container run as a long-lived pet, with the data directory on a
# volume, may opt back in
environment:
  ALMANAC_SELF_UPDATE: "on"
```

```bash
# on the LXC, to hand updates to something else
systemctl edit almanac      # Environment=ALMANAC_SELF_UPDATE=off
```

## R15 · Replacing the service account

Done once, on 2026-08-29, when the account was still called
`cal-stacean` and every mail Google sent said so. A service account's
name cannot be changed after creation, so a rename means a new account
— and a new account owns nothing, which is most of the work.

**In the Google Cloud console (only a person can do this):** create the
service account, give it no roles at all (Almanac only touches calendars
it created itself), add a JSON key, and make sure the Google Calendar
API is enabled on the project.

**Then, on Kenny's machine:**

```bash
latch edit .env      # replace CLIENT_EMAIL and PRIVATE_KEY
latch commit .env && latch push
latch run -- cargo run --example create_calendars      # new account, new calendars
latch run -- cargo run --example create_test_calendar  # for the live tests
latch edit .env      # point ALMANAC_TEST_CALENDAR_ID at the new one
latch commit .env && latch push
latch run -- cargo test --test calendar_e2e -- --ignored   # prove it before touching the deployment
gh secret set CLIENT_EMAIL PRIVATE_KEY ALMANAC_TEST_CALENDAR_ID   # the nightly live tests
```

**Then, on the deployment:** the LXC has a ciphertext-only clone with no
credentials to pull with, by design — copy `~/.latch/repo` across again
(`tar`, `pct push`, `chown -R almanac:almanac`), update
`target_calendar_id` in every profile under `/opt/almanac/profiles/` to
the new calendars, and restart. `almanac_token_refreshes_total` going to
1 on `/metrics` is the proof it authenticated with the new key.

**Three things that cost time and will again:**

`latch edit` honours `VISUAL` before `EDITOR`. On this machine `VISUAL`
was set to `“kate -b”` with typographic quotes, so every attempt tried
to launch a program literally named `“kate`. Set both variables, and
point them at an executable file — latch spawns the value as one
program name, not through a shell, so `EDITOR="python3 script.py"` is
read as a binary called `python3 script.py`.

The profiles on the deployment hold the calendar ids, and they are not
in this repository (deliberately — they are the household's, not the
code's). Changing accounts without changing the profiles leaves Almanac
authenticating perfectly and writing to calendars it no longer owns.

The old calendars stay in Kenny's calendar list until the old service
account is deleted; they are owned by that account, not by him. Deleting
the old project in the console removes both.
