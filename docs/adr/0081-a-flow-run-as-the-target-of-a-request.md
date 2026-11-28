# 0081. A flow run as the target of a request

Date: 2026-11-28

## Status

Accepted for flow-request-target. Adds to [ADR 0044](0044-flow-requests-waits-and-cancellation.md),
[ADR 0079](0079-one-reply-route-for-flow-and-actor-callers.md) and [ADR 0080](0080-flow-runs-end-with-an-output-or-a-failure.md).

## Context

Reusable flows are started and awaited by flows and by actors. A run has ends since ADR 0080, but nobody hears about
them, and the input is fixed in the created state before a request id exists.

## Decision

- `FlowMsg::Start { reply }` carries a `ReplyRoute` (untyped on the wire, as ADR 0079 allows). A run has one caller:
  the first route is saved (with the `Start`'s correlation id when an actor route has none), and a `Start` with another
  route fails for good. On a running or cancelling run `Start` acts as a host `Continue`; on an ended run without a
  route it saves the route and replies at once.
- The outcome is `FlowOutcome<O>`, tagged by type: `FlowDone { run, output }`, `FlowCancelled { run }`,
  `FlowFailed { run, reason }`, so the actor form sends it as a message.
- The turn that ends a run with a route arms a `Continue`; any `Continue` that reaches an ended run whose route has not
  been replied to sends the outcome, so a lost one can be sent again. A failed send leaves the run ended.
  The reply is never part of the end's own commit, because a send to a missing actor rejects the whole commit.
- `Step::request_flow::<F>` types a flow caller's wait as `Request<FlowOutcome<F::Output>>`.

## Consequences

- `FlowMsg` and the state (`reply`, `replied`) change every flow contract, in the same version as ADR 0080.
- An ended run may have one armed `Continue`: the outcome's. The output is limited to 60 levels, one less than a step's
  result, because `FlowDone` wraps it.
- A caller's family holds the three outcome variants; that match is checked at runtime only.
