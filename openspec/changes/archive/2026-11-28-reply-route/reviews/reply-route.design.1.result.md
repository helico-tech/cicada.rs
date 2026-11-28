Verdict: not yet

## Blocking

1. **Correlation fails for a reply sent in a later turn.** Where: design.md Decision 4, the spec requirement "One reply route serves flow and actor callers", ADR 0079 Decision. The actor form sends with the correlation id of the turn that sends it. That is the request's id only when the target replies in the request's own turn. A port that keeps the route and replies later (after a long mirror, say) sends the id of some other message, and `send(turn, &value)` has no way to set one. `flow-request-target`, next in the order, replies at the end of the run. That happens in a `Continue` turn whose correlation id comes from whatever message came in last (flow.rs ~2556). So Decision 4 breaks for the change that is meant to use this route. Fix: the actor form carries the caller's correlation id, `{"actor", "correlation"?}`, and `send` uses it when set. Or the design states the limit and `flow-request-target` takes it on.

2. **The `SendRejected` error path has no scenario, and the ADR contradicts the design.** Decision 2 checks the reply's type name against the registry, as `send_to` does. So a caller whose family lacks the variant, or whose type is not registered, gets `TurnError::SendRejected` in the target's turn. The spec has no scenario for this. ADR 0079's Consequences say a wrong family "shows only when a reply arrives", but a wrong name shows at the send, and only wrong fields show at delivery. Add the scenario and fix the ADR sentence.

## Notes

- `ReplyRoute::actor(&me)`: `ActorRef::address` can fail. Decide whether `actor` returns a `Result` or takes an `&ActorAddress`.
- flow.rs:906 already has a private schema struct `ReplyRoute`. Rename it.
- Moving a field to `ReplyRoute<T>` keeps saved messages, but it changes the port's contract hash. Say so in the ADR and in the handoff (upgrade note).
- The schema's `oneOf` accepts `{"actor","run"}`, but the decoder refuses it. Accept that gap or state it.
- The crate-root table in lib.rs probably needs a `ReplyRoute` row.

## Questions for the owner

- Does Factory's `repos.mirror` reply to `Ensure` in the same turn, or later?

## Not checked

Factory's code; HTTP workers' `check_send`; a build.
