Verdict: not yet

## Blocking

1. **The file/folder check is too narrow.** Decision 3 and the spec refuse only a file at a folder's own path. A file at a folder's parent passes: dirs `/a/b`, file `/a`. `make_dirs` makes `/a/b`, then `copy_in` of `/a` replaces the folder `/a` and its contents, as Docker does (the design says so). `/out/.` and `/out` are also one path. Fix: refuse a file at or above any folder, comparing cleaned paths. Add both cases to *Inputs that cannot be built*.
2. **`Conflict` from `make_dirs` has two meanings.** It means "a file where a folder was asked", and also "the container is not `Created`": Docker's `make_dirs` answers `Conflict` for a gone container. Decision 2 rejects reading `copy_in`'s `Conflict` for this reason, yet decision 3 reads `make_dirs`' `Conflict` as the job's mistake. The earlier `inspect` can be out of date. So a container removed just before `make_dirs` ends `failed`, while one that `inspect` sees removed backs off. Fix: after a `Conflict`, `inspect` again. Still `Created` means `failed`; anything else backs off. Add a scenario.

## Notes

- A host runtime without `make_dirs` answers `Other` through the default. The actor then backs off `max_errors` times (minutes) and ends with "the runtime failed 10 times", which `container-limits-and-kinds` would mark `runtimeErrors`, not `notStarted`. Say so in the rustdoc of `inputs`, or end at once.
- The spec's "`InvalidSpec` … SHALL end it `failed`": name the calls, `make_dirs` or `copy_in`.
- An `Err` from `inputs` on a replayed step whose container already runs ends `failed` and leaves it running until `Remove`. `spec` does the same today; say so in the rustdoc.

## Questions

None.

## Not checked

Docker replacing a folder with a file (from memory and round 1, not run). `container-copy-out` in its worktree, the whole fake, builds.
