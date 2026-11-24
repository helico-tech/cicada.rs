Verdict: not yet

## Blocking

1. **The one-batch release is not decided.** `design.md` § Version relies on one batch for all three changes. `releasing.md` says `release now` once a behaviour change lands and no change is ready or in build. Cancel will still be a proposal, so `job-graph` ships alone, and cancel's new statuses then break every host type that takes `JobGraphEnd`. Fix: accept two releases and name the break in `job-graph-cancel`, or add `skipped` and `cancelled` to the enum now.
2. **A refused job send contradicts itself and has no scenario.** ADR 0059 says it is "refused for good, the state unchanged". Decision 5 ends the attempt `failed` on any `Failed` of the run's own `Dispatch`. Fix: say in the ADR and the spec that the attempt ends `failed (refused)`. Add a scenario for an unregistered target type, or a type that does not take `messageType`.
3. **The attempt rules disagree.** Decision 8 keeps 20 attempts and drops the oldest. Decision 12's decoder wants attempts "consecutive from 1", which fails after a drop. Fix: "consecutive, ending at the current one".

## Notes

- The report target "gets one report", yet anyone may resend `Report`. Say receivers must expect repeats.
- For `job-graph-cancel`: watching `to` misses work handed on (decision 6), and cancel needs a cancel message per job, a new state field. Put both in that proposal.
- `job-graph-retry` says each attempt "has its own name", but attempts here are numbers. Align the wording.
- Size: borderline for 5 tasks.

## Questions

- Hold the release for all three changes, or accept two contract changes?

## Not checked

No build. Not checked: whether the retry policy retries a commit that a gone target rejected, creates on a node that does not host the type, or brood's code line by line.
