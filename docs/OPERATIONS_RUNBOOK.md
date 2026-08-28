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
# as root
useradd --system --home /opt/almanac --shell /usr/sbin/nologin almanac
mkdir -p /opt/almanac/{data,profiles} /etc/almanac
install -m 0755 almanac /opt/almanac/almanac
cp profiles/*.toml /opt/almanac/profiles/
chown -R almanac:almanac /opt/almanac

# the key that lets Latch open Almanac's secrets, and nothing else
printf 'LATCH_KEY_ALMANAC=%s\n' "$key" > /etc/almanac/latch.env
chmod 0600 /etc/almanac/latch.env

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
systemctl stop almanac
install -m 0755 <old binary> /opt/almanac/almanac
systemctl start almanac
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
| Profiles, config | git | `git clone` |
| Secrets | Latch escrow (offline) | `latch key restore` / `latch clone` |
| Journal (transient) | `/opt/almanac/data` | nothing to do — it is empty in steady state |
| Calendar data | Google | nothing to do |

So a destroyed LXC is a rebuild, not a recovery: R2, restore the Latch
key, done. The journal is worth backing up only if it is non-empty at
the moment of the backup, which means deliveries were failing then.
