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
