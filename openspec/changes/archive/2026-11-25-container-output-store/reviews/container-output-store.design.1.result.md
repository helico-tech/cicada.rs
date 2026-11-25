Verdict: not yet

## Blocking

1. **Input rules of the store are not decided** (design.md, decisions 3-5). `read_file(container, path)` and `read_logs(container, ..)` serve a host's HTTP route, so the name and path can come from a user. A folder store (Factory's) turns them into file paths; `../` would escape. `OutputErrorKind` dropped `InvalidSpec`, so nothing says what a relative path, a `..` part or an invalid container name gives, or whether `/a`, `//a` and `/./a` are one file. Fix: valid inputs are a name by the actor's name rule and a path by the shared path rule (`paths::path_parts`, which `copy_out` uses already). Add an `Invalid` kind (not retryable), make `check_output_store` test it, and add a spec scenario.
2. **ADR 0056 needs two consumers per layer** ("A layer with one consumer stays with that consumer"). The proposal names only Factory. Earlier container changes named both Factory and brood. Fix: name the second consumer, or record the owner's waiver in the proposal.

## Notes

- Spec deltas cover only logs. Files, `remove`, the empty-page cursor and the return count have no requirement or scenario.
- After `remove`, the stored cursor is `None` again. A stale archiver that gets `Conflict` reads it again and could archive the log again from the start. This belongs in `container-log-archive` and `container-output-retention`: name it there.
- A UI usually needs the tail or the line count, and no read gives the count without a write. Check that Factory's route pages from the start only.
- `check_output_store` cannot test atomicity across a crash, or a compare-and-swap under concurrent callers. Say that it is the host's promise. Also say whether the check needs an empty store, and whether a file at `/a` next to `/a/b` must work (a folder store cannot do that).
- There is no container page in `docs/guide/`. Name where the handoff prose goes. `LogLine` has no serde, so a durable store must write its own encoding.

## Questions for the owner

- Q1: Should stored data whose actor is gone be found later (a `containers()` list), or is retention through the actor enough?

## Not checked

No build or tests (nothing is built yet). Factory's `store.rs` not read. `openspec validate` not run.
