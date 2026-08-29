# Legacy documents

Kept because they record how Almanac got here, not because they
describe it. Nothing in this directory is maintained, and nothing in it
should be used to answer "how does it work now" — for that, start at
[../ARCHITECTURE_REFERENCE.md](../ARCHITECTURE_REFERENCE.md).

| Document | What it was | Why it is here |
|---|---|---|
| [INVENTORY.md](INVENTORY.md) | The Phase 1 brownfield sweep of `cal-stacean`, 2026-08-28 | It inventories a 1,681-line `src/main.rs`, Infisical, a `config.toml` and a hardcoded Vikunja integration. None of that exists any more. It is still the only record of the 19 defects that shaped the rewrite, and several decisions only make sense next to it |
| [AFK_QUEUE.md](AFK_QUEUE.md) | The quarantine queue for the AFK build, closed 2026-08-28 | Empty by the time it closed — no frozen decision had to be reopened. Kept as evidence of that, since "nothing was quarantined" is a claim worth being able to check |

Live documents, all in the parent directory:

| Reading for | Document |
|---|---|
| What Almanac is for | [SCOPE.md](../SCOPE.md) |
| How to make it do something | [USER_GUIDE.md](../USER_GUIDE.md) |
| Why it is not doing it | [DEBUGGING_GUIDE.md](../DEBUGGING_GUIDE.md) |
| Running the machine | [OPERATIONS_RUNBOOK.md](../OPERATIONS_RUNBOOK.md) |
| The system as built | [ARCHITECTURE_REFERENCE.md](../ARCHITECTURE_REFERENCE.md) |
| Why it was built that way | [ARCHITECTURE_DECISIONS.md](../ARCHITECTURE_DECISIONS.md) |
| What is proven, and where | [TEST_PLAN.md](../TEST_PLAN.md) |
| The frozen feature list | [FEATURES.md](../FEATURES.md) |
