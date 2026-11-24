Verdict: not yet

## Blocking

1. **A retry drops the cancel notice of the attempt it replaces.** design.md decision 6; the spec delta retries no cancelled job. jc 2 sends `JobCancel` only "while that job is `cancelled` with that last attempt". When `Retry` comes before the run handles `SendCancel {N}`, the job is `waiting` or runs N+1, so attempt N's worker is never told and keeps working next to the new attempt, often on the same target. Cancelling the replaced attempt is the point of the notice. Fix: send it when attempt N is a kept, dispatched attempt closed `cancelled`, whatever the job's status; amend jc 2 in this delta. Add scenarios "Retry a cancelled job" and "Retry before its `SendCancel` is handled".
2. **Decision 1 contradicts the spec.** "Not starting it in the `Retry` commit" conflicts with "Retry a failed job" (starts in the same commit) and jf 7. Reword: "no start path of its own; the normal start runs in the `Retry` commit".

## Notes

- `ends` has no type or overflow rule. Make it a `u32` and refuse `Retry` at its largest value, like attempt numbers.
- A `Retry` before end 1's `Report` turn means end 1 is never reported, so end numbers can have gaps. Put this in the docs handoff.
- ADR 0059 says a target "treats a repeat for the same run as one". ADR 0061's Status should say it refines that, and the `JobGraphEnd` and `Report` doc comments need the same update (name it in a task).
- A retried job cancelled before it starts is reported `cancelled` with the old attempt's `failed` reason. Put this in the guide.

## Questions

None.

## Not checked

Brood's reference, the full text of ADRs 0059 and 0060, the existing tests. No builds.
