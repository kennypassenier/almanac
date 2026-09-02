# Open points

Things decided but not yet finished, kept here rather than in a
conversation — a conversation gets summarised and then the point is
gone. Each entry says what is waiting, and what closes it.

## Measurement owed: does norm N4 hold without a reminder?

**Opened** 2026-09-02, by the correction form for the latch key loss
(standing rule 29, all nine fields ratified by Kenny).

**The fault.** Almanac's only encrypted secret store had exactly one
durable copy — the `LATCH_KEY_ALMANAC` line in the container's
`EnvironmentFile`, which is in the restic backup — and nothing said so
anywhere. When the workstation keyring emptied during a system upgrade,
a recovery survey across every project filed
`almanac/dev/.env.enc` as *"a `dev` environment … Nothing operational
depends on this file"*, while it held the credentials the live service
runs on. Nothing was lost, and that was luck rather than design.

**The gate that let it through.** Phase 8, the documentation gate. The
runbook answered "the journal is gone" (R11), "the service account must
be replaced" (R15) and "the state moves" (R16), and never "the key is
gone". An approval form that asks about the strongest claims in a
document cannot find a missing scenario.

**Also present in**, measured on 2026-09-02: kyu, kyu-runner and
newsflash — three running services, all consuming latch secrets, none
documenting key loss. They fix it in their own sessions, not from here.

**The measure.** ECOSYSTEM norm N4: a service's runbook names every copy
of its secret key, what each copy survives, and gives a runnable recipe
that restores the workstation's copy from the running deployment.
Almanac's own is R17, written and shipped. Enforcement is discipline,
reinforced by a fixed question in the doc-writer agent's brief — not
code, and marked as such per standing rule 24.

**What closes this entry.** At the next Phase 8 documentation gate of
any project: did the key-loss section appear without Kenny or Claude
asking for it? Write the answer here and close it. If it did not, the
fallback is already decided: a check that fails the documentation gate
for a project with a latch link and no such section.

**Review of the norm itself:** at the retrospective of the third project
to apply it.


## Closed 2026-09-02: does the release guard actually block a red CI?

**The fault.** CI was red on every commit from 2026-08-29 10:07 to
2026-09-02, through releases 1.1.0 through 1.5.0. The `container` job
could not start the image it had just built —
`version 'GLIBC_2.39' not found` — because the `rust:1.97-slim` builder
moved to a trixie base while the runtime stage was still bookworm. The
`gates` job stayed green the whole time, and nobody read the rest. No
published binary was affected: those are built natively, and v1.5.0
needs GLIBC_2.39 while CT 112 has 2.41, measured before it was said.

**The gates that let it through.** Branch protection on `main` requires
the `gates` check but allows a bypass, and every push used it. And R1,
the release procedure, never said "check CI first" — seven times.

**Also present in:** `binary-puzzle-toolkit`, the only other repository
with `enforce_admins: false`. Its CI is green, so nothing accumulated
there, but the gap is the same.

*Corrected while carrying this out:* the approved measure said the same
guard would go into that repository's Makefile. It has no Makefile —
it releases from a tag-triggered workflow — so there is no equivalent
place to put it, and standing rule 19 keeps work on a project inside a
session opened in that project. It is therefore **still open there**,
and the shape of the fix has to be decided in its own session: either
`enforce_admins` on, or a guard in whatever it does use to tag. Written
down here rather than quietly dropped, because a measure that covered
two repositories and silently covers one is how a correction stops
being one.

**The measure**, code-enforced: `scripts/check-ci.sh`, run as the first
step of `make tag-*`, before the version bump.

**Measured immediately, on the real thing** — which is why this entry
opens closed rather than pending:

    ./scripts/check-ci.sh d49cb69   CI is green on d49cb69.          exit 0
    ./scripts/check-ci.sh 345d847   CI is RED — refusing to release. exit 1
    ALMANAC_ALLOW_RED_CI=1 …        skipped, releasing anyway.       exit 0
    ./scripts/check-ci.sh 88e6f83   no CI run found — has it been…   exit 2

Tested by calling the guard with a commit rather than by running the
release target, deliberately: the homelab put three fake tags on GitHub
testing their equivalent, because their first attempt used `exit 0`
inside a make recipe and each recipe line is its own shell, so make
carried on to the tag and the push. `make -n tag-minor` confirms the
guard is the second line and nothing mutates before it.

**Fallback if it proves unusable:** `enforce_admins` on both
repositories and changes go through a pull request. **Review:** at the
first release where this guard blocks something it should not have.
