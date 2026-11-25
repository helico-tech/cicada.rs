# Proposal

## Why

From the code review of `container-output-collect` (2026-11-25): a collection turn that runs past the job's `MAX_TURN`
ends `failed` (`timeout`) and commits nothing, so it never counts as a runtime error. A copy that always takes longer
than the turn limit is retried by the step's retry policy (7 attempts), then the container waits in `Collecting` for a
`Check`, and its end is never sent: the owner waits forever.

## What Changes

- A collection that keeps timing out ends: the end is recorded and sent with an `outputs.error` such as
  `(no outputs: the copy took longer than the turn limit)`. The design picks how the actor learns of the timeouts (the
  step message's attempt number in the turn, a counter written before the long call in its own commit, or a smaller
  copy per turn).

- From the code review of `container-output-retention`: store failures while deleting outputs are logged only at
  `debug`; warn once the retries reach `max_errors`, and say in the rustdoc of `max_errors` that the deletion keeps
  trying.

## Order

After `container-orphan-alert`, before `placement-offers`.

## Impact

`crates/cicada-containers` (the container actor's collecting phase). Possibly a state field (a contract change): the
design says which, and whether it joins 0.23.0.
