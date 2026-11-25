Verdict: not yet

Round 1 blocker 1 (shutdown budget) is resolved by Decision 4. Blocker 2 is resolved in the design (Decision 5), but not in the spec.

## Blocking

1. **The shutdown order is still written two ways.** design.md Decision 3 says that on an early end "the other tasks are told to stop and waited for, then the node, the server and the store, as for a signal". That is tasks first, which Decision 4 rejected. The node would keep claiming, with `/ready` at 200, for up to a grace period before its own grace period starts. The ADR index row (docs/adr/README.md, 0064) also still says "told to stop first at shutdown". Fix: in Decision 3, say `Stopping` is set and the node's stop starts at the same moment, exactly as for a signal. Rewrite the README row to match.
2. **The other exit paths have no scenario.** The spec requirement "An early end of a task stops the node" says every exit path stops the tasks, but its scenarios cover only a panic and an early `Ok`. Add the scenario round 1 asked for: when `serve` cannot write `Listening on` (or its HTTP server fails) after the tasks started, every task sees `Stopping` and ends before the store closes.

## Notes

- Decision 4: say whether an aborted task is waited for, and for how long. A task that never yields cannot be aborted. If it is awaited with no limit, the stop hangs.
- The worker help text (command.rs `WORKER_HELP`, "A binary with App::on_start cannot run over HTTP") goes into docs/reference/cli.md. If it is changed to name tasks, cli.md changes too, but design Docs says it does not.
- The handoff should also name docs/architecture.md (line 387, which says `on_start` is refused over HTTP), not only the app guide and operations.md.
- Spec "A worker over HTTP": say "before it reads the config file", as the `on_start` refusal does, not only "before it sends a request".
- Decision 6: say whether duplicate task names also refuse the operator commands, or only `serve` and `worker`.

## Questions for the owner

None.

## Not checked

Whether the paused-time abort test works on SQLite in memory. Whether the build fits in 5 tasks (it looks like 4). E2E behaviour.
