Verdict: not yet

## Blocking

1. **One odd file name drops all outputs** (design 4–6). With decision 6, `put_file` refuses a control character in a file name, which a container can create. Decision 5 then ends the whole collection on `Invalid`. Skip that file into `skipped` instead. Also map `Unsupported` from `copy_out`: end at once.
2. **Contradicts the spec "A deadline and errors in a row".** At `max_errors`, a container without an end ends `runtimeErrors`, and none is recorded during collection. MODIFY that requirement and say which outcome the end keeps. "Counts so far": the last attempt's, or the store's `files()`?
3. **The report's contract is open.** `SkippedFile` has no `size` (only in the reason text). `OutputsReport` has no `error` field, but decision 5 and the spec use one. Also open: `outputs` with a store but no `outputs()`.
4. **No ADR for a wire-format change.** AGENTS.md requires one, and ADR 0065 does not cover this field. Name ADR 0066.
5. **No scenarios for:** a removed container (`Conflict`), an `InvalidSpec` path, a store `Unavailable` then back, no store, and a 0.22 end without `outputs` decoding.

## Notes

- container-progress sends its last progress in the end's turn. If collection delays the end, say which turn sends it.
- The rustdoc of `logs_since`, `copy_out` and `LogArchiver` says "outside actor turns". Name these rustdoc changes.
- `ContainerEnd` is not `non_exhaustive`: the new field breaks struct literals. Add that to the upgrade handoff.
- The store delta should MODIFY "refuses names and paths". At 5 tasks: split decision 6 out if the work grows.

## Questions

- CO-A: skip a file the store refuses (my pick), or end the collection?
- CO-B: after `max_errors`, keep the outcome (my pick), or end `runtimeErrors`?

## Not checked

The container-progress worktree, the Docker backend, the fixtures.
