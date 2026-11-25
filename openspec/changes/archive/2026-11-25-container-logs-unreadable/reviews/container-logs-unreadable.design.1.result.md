Verdict: not yet

## Blocking

1. **The deltas ADD rules that contradict live specs.** In `openspec/specs/container-docker/spec.md`, "Docker's errors map onto error kinds" maps every other failure to `Other`. In `container-runtime/spec.md`, "Errors say whether a retry can help" defines `Unsupported` only as "a call the runtime does not implement". Both deltas are `## ADDED`, so after archiving the specs disagree. Fix: MODIFY both requirements: one names the log 501, the other widens `Unsupported` (decision 1). Optional: MODIFY the actor's "errors in a row" requirement to name the exception.

## Notes

1. "or that text in a stream error" (decision 2, docker spec) has no clear error variant. Once Docker is streaming, it writes errors into the log as text, not as an error. Map the 501 alone, or name the variant and its test.
2. Decision 5 names only the trait's rustdoc. `DockerRuntime`'s rustdoc (`docker/mod.rs:68`) also promises `Other` for `none`.
3. "`is_retryable()` is false" (runtime scenario) is already true for `Other`, so it proves nothing. Assert the kind.
4. "no poll is armed for the logs" (actor scenario) clashes with `end()`, which arms a removal poll when the goal is Remove. Say instead: `logs` is called once and the end is recorded in that turn.
5. The proposal says "and in the fake"; decision 5 says the fake needs nothing. The design wins, but say so in the tasks.

## Questions for the owner

- Q1: Patch or minor? Three places promise `Other` for this case: `logs_since`'s rustdoc, `DockerRuntime`'s rustdoc and the 0.21.0 changelog. `install.md` counts public APIs as compatibility boundaries, so decision 4's "could not tell apart before" goes too far. My pick: patch, and the Changed entry names the old promise.

## Not checked

Whether Docker answers 501 for a `none` container that was created but never run; I believe it checks the driver first, and the E2E will show. No tests run. `openspec validate --strict` passes.
