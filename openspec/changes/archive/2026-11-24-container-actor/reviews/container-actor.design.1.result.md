Verdict: not yet

## Blocking

1. **Stop or Remove in phase `new` loops on errors.** design.md decision 4, row "stop or remove, before the end": with no container (`Remove` without `Start`, `Start` then `Stop` before the first poll, or an outside removal), `logs` is `Conflict` (trait). Each poll counts an error, so after `max_errors` backed-off polls (minutes) the end is `failed`. Fix: keep `stop`, which catches a container whose create commit was lost. On `Missing`, end `stopped` without calling `logs`. Say whether a later `Start` gets that end. Add scenarios.

2. **An end that cannot be sent blocks the actor for good.** Decision 3 follows the batcher: an unknown target type is refused, and a deleted target rejects the commit. The end is saved only in that commit, so the poll fails and is lost. `Check` fails the same way, and `Remove` never passes "before the end", so the container is never removed. Decide: save the end as not delivered when the send is refused, and/or make "the target outlives the end" a host rule. Add a scenario.

## Notes

- `ImageNotFound` and `InvalidSpec` also wait `max_errors` backed-off polls.
- Decision 5's "key": the decoded name or the percent-encoded key?
- Tests need `FakeRuntime` and `TestActors`, but `testing` is off in the workspace. Name the dev-dependencies and where the tests live.
- At the deadline, does that same poll stop the container? A goal change arms a poll at once, skipping the backoff. Does `stopped` carry the exit code?
- The image pull and `stop_grace` run inside a turn, under `max_turn`. A poll that always times out is lost until `Check`: put this in the handoff.

## Questions

- Should `ImageNotFound`/`InvalidSpec` end `failed` at once?

## Not checked

The contract-test machinery. container-docker in full. The full texts of ADRs 0031, 0052 and 0053.
