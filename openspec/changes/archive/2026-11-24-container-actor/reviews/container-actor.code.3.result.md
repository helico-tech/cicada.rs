Verdict: pass

## Blocking

None. Round 2's finding is fixed (`6dfcded`, `crates/cicada-containers/src/actor/mod.rs`):

- `follow` resets `errors` only after a `Running` inspect, or a `Created` inspect followed by a start that worked
  (`mod.rs:406`). A start that always fails now counts on every poll and ends `failed` at the limit.
- `stop` no longer resets after its inspect. `logs` failures count on top of earlier errors, and at the limit
  the end gets `(no logs: …)`. `end` and `remove` still reset, and so does `create` after its start.
- No new counting bug: every path either ends, resets after a completed step, or adds one through `error`/`logs`.
  The removal still keeps trying past the limit (`state.end` is set) at `max_backoff`.
- The new tests match the backoff (2 s interval: 2 s, then 4 s, then 8 s). `logs_that_always_fail_end_at_the_limit_without_logs`
  checks the code, the `(no logs: daemon unavailable…` text, `errors` 0, no poll armed and one delivered end.
  `a_created_container_whose_start_always_fails_ends_failed_at_the_limit` checks three starts and the
  "3 times in a row" reason. The handoff line on resets is true to the code. The test count of 22 is right.

## Notes

1. `openspec/changes/container-actor/design.md:50` still says "Any success resets `errors`". The code and the
   handoff now say a completed step resets it. Change the design line to match before archiving.
2. Round 1's note 1 (a log read after failed inspects can hit the limit at once) is now the documented
   behaviour, as round 2 accepted.

## Not checked

I ran no build or tests. I relied on the builder's gate evidence for 6dfcded and checked the timings of the new
tests by reading `backoff` and the default settings. I read only `1a073a7..6dfcded` and the counting code at head.
I ran nothing against real Docker.
