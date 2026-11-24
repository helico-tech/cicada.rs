Verdict: fix needed

## Blocking

1. `crates/cicada-containers/src/actor/mod.rs:399` and `:438`: `errors` is now reset right after a successful
   inspect, before the step's next runtime call. If that call always fails, `errors` goes 0 → 1 on every poll and
   never reaches `max_errors`, so the container never ends:
   - `logs` failing every time (after `Exited`/`Failed`, or in `stop`): the design's `(no logs: …)` at the limit
     can no longer happen. The actor polls forever and the target gets no end.
   - `Created` with `start` failing every time: it never ends `failed`. Before this round the reset came after the
     start.
   How to see it: `FakeRuntime`, `max_errors = 3`, the container exits and `logs` always fails. `end` stays `None`
   and `errors` stays at 1.
   Fixed: a step resets `errors` only when it completes. Put the reset back after the start, and do not reset
   before `logs` (`end` resets anyway). That keeps the behaviour round 1's note 1 was about, which is acceptable.
   Rewrite `a_successful_inspect_resets_the_errors_before_the_log_read` and the handoff line "a successful inspect
   too". Add tests: logs that always fail end with `(no logs: …)` at the limit, and a `Created` container whose
   start always fails ends `failed`.

## Notes

1. Round 1's blocking finding is fixed: `Start` checks the message type before it saves anything (`mod.rs:239`).
   The test covers an empty type, a 201-character type, and a valid `Start` afterwards.
2. The handoff and the doc comment now say an end can arrive more than once; that matches the code. The contract
   changed only in its description.

## Not checked

I ran no build and no tests; I relied on the builder's gate evidence for 1a073a7. I found finding 1 by reading the
code, not by running a test. I did not check the cause name `refused`, and nothing ran against real Docker.
