Verdict: not yet

## Blocking

1. **Shutdown budget and the existing Stopping scenario.** design.md Decision 4, spec "Tasks end first at a stop". With tasks first, the node keeps claiming new turns and `/ready` stays 200 for up to a whole grace period. Only then does the node stop, and its turns get a second grace period. The worst case becomes 2 x grace + cancel timeout + 5 s drain. That breaks the ordering in `openspec/specs/app/spec.md` "Stopping" (the node claims no more; `/ready` 503) and the platform budget in `docs/operations.md` "Shutdown and probes" (grace + cancel + drain, for example 45 s). A kill in the gap uses up lost claims. Decide and write down: does the task wait come out of the one grace period or add to it, and what do `/ready` and claiming do while tasks end? Then add a MODIFIED "Stopping" scenario. My pick: keep tasks first, state the full worst case, and hand `operations.md` and the app guide's Lifecycle paragraph to the docs session.
2. **Exits after the tasks start, with no scenario.** node.rs `serve`: tasks start before `Listening on` is written. A failed write, or a failed HTTP server (`serve_and_close`), stops the node and closes the store. Nothing says what happens to the tasks then. Under `run_with` on the host's runtime they would keep running. Say that every exit sets `Stopping` and waits for the tasks, and add a scenario. Also add scenarios for: an early `Ok` and an early `Err`, an error after the stop (exit 0, warning), and the HTTP-worker refusal (exit 2). The design names all of these, but the spec has only the panic.

## Notes

- ADR 0064 Status says "Builds on ADR 0011" (library only). It should be ADR 0027 (the app layer), and possibly 0042.
- proposal Impact names the E2E suite, but design Tests has no E2E test. Make them agree.
- Two tasks with the same name: refuse them (exit 2) or allow them? Say which.
- The HTTP-worker refusal message names only `on_start`. Say the new wording.
- Race: an early end and a signal at the same moment. The worker keeps a failure it already recorded. Say which one wins here.

## Questions for the owner

None.

## Not checked

Whether the build fits in 5 tasks (it looks like 4). Whether SQLite in memory works with paused time for the abort test. Other docs pages that mention `on_start` beyond the app guide and operations.md.
