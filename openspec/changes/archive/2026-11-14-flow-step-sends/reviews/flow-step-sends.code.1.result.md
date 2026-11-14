Verdict: fix needed

Blocking findings:

1. `crates/cicada-actors/src/flow.rs:514`: `check_waits` skips every non-Wait entry before checking `waits.open`. Consequently, decoding accepts a running journal containing a reply-enabled send `r` (message `m1`), an open all-wait `w` for `m1`, then `{ "kind": "step", "id": "after", "result": null }`. A sleep or send after the open wait also passes; the same defect applies to cleanup while cancelling. This violates ADR 0044 and the requirement that an open wait be the last entry, allowing invalid supplied progress into the reserved wire that later releases must support. Check the open-wait condition for every subsequent entry (or require an open wait's index to be the journal's final index). Add rejection regressions for step/sleep/send successors in both journals; the current regression only places another wait after it.

Notes: The send outbox, conversion/latch ordering, typed Store-error chain, permanent refusals, generated wire checks and old/new registration example otherwise match the agreed design. Reviewed the documentation handoff under owner decision 24a; its upgrade caveats are consistent. The reported permanent-error formatting departure preserves the required refusal behavior.

Not checked: I ran no tests, build, clippy, formatting, E2E or fault experiment; assessed the builder's reported checks and inspected the source/tests at e0c50cc. The finding above follows directly from the decoder's control flow.
