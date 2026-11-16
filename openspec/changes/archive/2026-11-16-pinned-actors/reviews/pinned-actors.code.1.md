---
id: pinned-actors.code.1
kind: code
change: pinned-actors
round: 1
base: 12c57e3d70f5af4df945b1a742eab14414f96beb
head: 51fd6a5eefc54cb71725e524f06f22dd229beeea
requested: 2026-11-16T18:17:13+01:00
---

Follow `.claude/skills/review-loop/code.md`, and hand in the report with `python3 scripts/review.py done pinned-actors.code.1 <file>`.

## Note from the requester

Version: 0.20.0 (breaking: the proposal does not mark it BREAKING, but the design adds public struct fields without literal compatibility and appends schema 8 with a drain requirement; docs/how-to/install.md#upgrade counts both as compatibility boundaries, as for 0.19.0)

Builder report (feat/pinned-actors, head 51fd6a5; base 12c57e3 = current main):

Tasks: all six done. Code, generated references and the handoff in 902c51e and a92f1c1; 51fd6a5 only ticks task 6.

Checks:
- Workspace tests with PostgreSQL: 2524 passed, 0 failed.
- New placement conformance cases pass on memory, SQLite and PostgreSQL; upgrade tests from schema 7 pass on both SQL stores.
- clippy -D warnings, fmt, feature builds, client tests on Node 26, fixtures, docs, doctests, skill, openspec validate --strict.
- gate.py status: "checks: yes, docs pass over the full pass of a92f1c168fc8"; "e2e: yes, e2e pass of the full-checked base a92f1c168fc8".

Departures (please judge):
- The CLI has no wide listing of actors with their state, so PINNED-TO shows only in `get actor -o wide`; the HTTP and client state listing carry pinnedTo.
- The fuzz coverage test failed once pins were added (one count fell from the required 10 to 9). The builder lowered how often generated creates are pinned rather than lowering that floor, and added a floor for pinned actors seen.

New concepts: pin (optional owner id set at create, never changed; only that node or worker runs the actor's turns; messages wait while it is away); claim owner (the node or worker id a claim is made for; Turn::node_id() returns it, never the HTTP server's id).

Breaking: new public fields on ActorCreate, ActorRecord, ActorSnapshot, CreateActorRequest, ActorCreateView and ActorView (struct literals need pinned_to: None); schema 8, stop every 0.19 direct-store node before migrating; upgrade HTTP servers before anyone sends pinnedTo; S8 liveness now needs an eligible executor that keeps polling the type (ADR 0052).

Docs: handed off, no blockers. docs-handoff.md proposes text for architecture, the S8 row with three test names, the core, HTTP, app and design guides, operations and the upgrade procedure.
