Verdict: not yet

## Blocking

1. **Status after `stop` is undecided** (design 6, 8; spec status requirement). `Failed` covers "killed", but `stop` kills by design; Docker reports exited 143/137. The fake must pick, and `container-actor` reports it. My pick: `Exited` with the code; `Failed` only for OOM, start error, dead. Add a scenario.
2. **Repeat rules miss states** (design 6, 9). Undecided: `start` on `Failed` (pick: `Ok`, no-op); `copy_in` on running/exited (`Ok` or `Conflict`); `stop` on `Created` (stays `Created`, so a later `start` runs it?). One sentence and scenario each.
3. **`sweep` has no error path** (design 11). A labelled network or volume in use by an unlabelled container: skip it and return `Ok`, or fail with `Conflict`, losing what was already removed? Decide; add a scenario.

## Notes

- ADR 0011's Status line is not updated ("Partly superseded by 0056", as ADR 0030 does).
- Design 1 says no workspace crate depends on it; spec and ADR 0056 allow examples and tests. Align.
- Decide `#[non_exhaustive]` for `ErrorKind` (likely yes), `ContainerStatus`, `Mount`, `NetworkSpec`, `VolumeSpec`, `CopyFile`.
- "`async-trait` and `thiserror` only" means normal dependencies; tests need an executor. Nothing enables `testing`, so under `cargo test --workspace` doctests and `tests/` files using `FakeRuntime` don't compile or are skipped. Keep fake tests as unit tests, and say so.
- `ensure_network` compares `internal`; `create` and `ensure_volume` compare nothing. Write why.

## Questions for the owner

- Blocking 1: is a stopped container the program's exit or the runtime's?

## Not checked

Factory and brood sources (not in repo); real Docker behaviour beyond my knowledge; no build or tests. `openspec validate container-runtime` passed.
