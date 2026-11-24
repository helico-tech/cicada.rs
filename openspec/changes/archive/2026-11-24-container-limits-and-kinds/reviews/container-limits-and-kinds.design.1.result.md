Verdict: not yet

## Blocking

1. **Breaking change shipped as a patch.** design.md decision 5, Version; proposal Impact. `docs/how-to/install.md:55-56` says public Rust APIs and contract hashes are compatibility boundaries, and a break raises the minor. This change breaks both: a `ContainerEnd` literal needs `kind`, and every container job type and end target changes its hash. The 0.20.8 precedent does not apply: `JobGraphEnd` was new in 0.20.8 itself. Fix: make the batch 0.21.0. Rename the heading and rewrite the 0.20.9 summary, which still says "Additive" (releasing.md). Or the owner waives the rule, and ADR 0062 records the waiver.
2. **The proposal contradicts the design.** The proposal still says `timeout(&self, &Request)`, and its kinds leave out `runtimeEnded`. Update it once the owner accepts the new form.

## Notes

- "out of memory" becomes a contract between runtime and actor. No rustdoc (runtime.rs:160) or spec states it. Write it, as an exact match, in that rustdoc and the ADR.
- `notStarted` can be false. A replayed create step on an already running container, where `spec` or `inputs` gives an `Err` (container-inputs decision 4), ends `notStarted` while the container runs. Name it in rustdoc.
- Decision 2: "the decoder" has two readings: the state's `RawState` check only, or `ContainerEnd`'s own decode; I pick the second, so targets get it.
- Decision 1 says the deadline is "capped at MAX_INSTANT", but today `create` drops a deadline past it (no limit). Pick one.
- A start refused with something other than `InvalidSpec` (for example `Other`) still ends `runtimeEnded` later. Name this limit.

## Questions for the owner

- 0.21.0, or waive the rule?
- Accept the three-argument `timeout`?

ADR 0062: decisions 2–4 are enough, plus the version.

## Not checked

Docker's real status for a refused start. Nothing built.
