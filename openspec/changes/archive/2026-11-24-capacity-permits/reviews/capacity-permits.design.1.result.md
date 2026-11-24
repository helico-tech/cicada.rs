Verdict: not yet

## Blocking

1. **A gone waiter takes a permit for good,.** (design.md decisions 4, 7, 8; spec "A deleted waiter".) A queued `Acquire` sends nothing, so a gone target queues unchecked. "Cancel, then delete" races a grant: `Cancel` is ignored for a holder, and a deleted actor cannot release. Once granted, its `Notify` fails; the audit's `check` fails too, so only an operator reading the state frees it. With capacity 1, the pool stops. Fix: the pool watches itself, as the topic does (`dropped`); when its own `Notify` fails with cause `commit` for a holder whose number was never delivered (a `notified` flag per holder), it frees the permit. Safe: the holder never learned the number, so no work runs under it. Or accept the leak and document safe deletion.
2. **Error paths have no rule and no scenario.** A `Resize` above 1,000, an `Acquire` with a bad request ID, or a `messageType` the target does not take. "SHALL set the capacity, within the bounds" can mean clamp or refuse. Say: refused for good, state unchanged; add scenarios.

## Notes

- "FIFO order" and "A resize up" say "granted in the commit", but the notice comes later (decision 7). Separate state from notice.
- Decision 4 asks a fresh ID per attempt; "Grant numbers only go up" reuses `a`. An `Acquire` resent after a release takes a permit nobody releases. Pick one rule.
- No scenario: a foreign `Notify` is ignored.
- Decoder: also check `waiting.len() <= maxWaiting` and `nextGrant >= 1`.

## Questions for the owner

- Free undelivered grants automatically (finding 1, my pick), or accept leaks only an operator clears?

## Not checked

Builds, contract hashes, retry and dead letters for failed `Notify` turns.
