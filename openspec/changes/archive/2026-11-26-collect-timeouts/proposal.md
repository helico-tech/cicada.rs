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

- Factory's request (2026-11-26, its epic 0007): a cap on the bytes `copy_out` reads in total, skipped data included
  (on `CopyLimits` or `Outputs`, default a few times `max_bytes`). Today one big file over the limits is read in full on
  every try; with a turn limit alone it is still read on every try, each timed-out try leaving a Docker read and up to
  64 MiB in memory that `Stop` cannot end. Over the cap, the copy stops reading, reports the rest as skipped with a
  reason, and the end goes out with what was stored. The design checks whether `CopyLimits` can take a field without a
  break.

## Order

After `container-orphan-alert`, before `placement-offers`.

## Impact

`crates/cicada-containers` (the container actor's collecting phase). Possibly a state field (a contract change): the
design says which, and whether it joins 0.23.0.
