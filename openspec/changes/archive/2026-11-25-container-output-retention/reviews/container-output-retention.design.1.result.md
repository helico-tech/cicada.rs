Verdict: not yet

## Blocking

1. **The phase `Removed` does no work today, and the change does not say it gains some.** In `actor/mod.rs`, `work_left` is false in `Removed`, and `step` returns at once there. So `Check` "changes nothing" (spec rule "A lost poll comes back"). Design decision 3 depends on "the next `Check`" and on "the actor's existing re-arm" (the only re-arm is a host's `Check`). Neither works unless `work_left` is true while `outputs_until` is set. Fix: say so in the design, and add a MODIFIED delta of "A lost poll comes back" with a scenario: a purged retention poll followed by `Check` still deletes the outputs.
2. **The `max_errors` sentence in decision 3 can be read two ways.** "Tries again at the next `Check` or after the maximum backoff" could mean it stops and waits for a `Check`, or that it keeps polling at `max_backoff`. Fix: do what a removal does today ("a removal SHALL keep trying"): back off up to `max_backoff` with no limit. Then `max_errors` plays no part here.
3. **Error paths with no scenario in the spec delta.** The requirement says errors are retried and that this survives a restart, but it has only two scenarios. Add scenarios for: `Unavailable`, then deleted; a restart between removal and the due time; `Invalid` clears with a warning. Also decide, and give a scenario for, the case where the settings changed by the due time (no store any more, or `output_retention` now `None`). I would let the saved `outputs_until` decide. Without a store, it clears with a warning.

## Notes

- State the validity rule: `outputs_until` only in `Removed`, as `container-output-collect` does for `pending`.
- Compute `outputs_until` capped at `MAX_INSTANT`, as `arm_after` does. Say whether a retention of zero is allowed.
- The proposal left open what happens to the actor's own record after the deletion. The design keeps it but only implies this. Say it in one line.
- A simpler way, not required: reuse `armed`/`due` in `Removed` instead of adding a new field. The new field is clearer, and the contract changes in 0.23.0 anyway, so I would keep it.

## Questions for the owner

- None beyond the choice in blocking 3, if my pick is not wanted.

## Not checked

- The `LogArchiver` code (I trusted the round-trip after `remove` from `outputs/check.rs`).
- Whether the docs handoff covers the guarantee rows. None seem to be touched.
