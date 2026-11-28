# Design

Factory's reusable flows are started and awaited by flows and by actors. After `flow-ends` a run has its ends (done
with an output, cancelled, failed with a reason) but nobody hears about them. This change lets a caller start a run
with a reply route and get one outcome when the run ends. It follows ADR 0044, `reply-route` (ADR 0079) and
`flow-ends` (ADR 0080), and ships in the same version as `flow-ends`. The picks are mine, under the owner's A1.

## Decisions

1. **`FlowMsg::Start { reply }`.** `FlowMsg` is one family for every flow type, so the route it carries is untyped:
   the wire of `ReplyRoute` (ADR 0079), the same for every `T`.
   - The field is `reply: ReplyRoute<FlowOutcome<serde_json::Value>>`; `FlowMsg::start(route: impl
     Into<ReplyRoute<FlowOutcome<O>>>)` builds it from either typed form.
   - For a flow caller, `Step::request_flow::<F>(id, target)` sends `Start` and returns a
     `Request<FlowOutcome<F::Output>>`, so the output type is checked by the compiler. The target is usually
     `Target::child(&run, &FlowState::new(input))`, so the run and its `Start` commit together.
   - An actor caller creates the run and sends `FlowMsg::start(ReplyRoute::actor(me).correlation(id))` in one turn.
   - Not taken: putting the route in the created state. The route needs the request's id, which exists only when the
     message is built.
2. **What `Start` does.**
   - **The route saved:** the first route goes into the state's `reply` field. An actor route without a correlation id
     gets the `Start`'s own, so the outcome carries the caller's id whatever turn ends the run.
   - **A second `Start`:** with the same route it changes nothing but the restart below. With another route it is
     refused for good, so the second caller's message fails visibly instead of waiting forever. One run has one caller
     (design review 1, Q1). Not taken: a list of routes, which is a later change if wanted.
   - **On a running or cancelling run** it then acts as a host `Continue`: it cancels the armed `Continue` and arms a
     fresh one. So a `Start` also starts a run that was never started.
   - **On an ended run without a saved route:** it saves the route and sends the outcome in the same turn, so a late
     caller still learns how the run ended.
3. **The outcome: `FlowOutcome<O>`.** It is a tagged enum (`#[serde(tag = "type")]`), so the actor form can send it as
   a message:
   - `FlowDone { run, output }`
   - `FlowCancelled { run }`
   - `FlowFailed { run, reason }`

   Here `run` is the run's address, so an actor caller with several runs open can tell them apart, besides by
   correlation id. The output is wrapped one level deeper than it was saved, so `flow-ends` checks it at depth at most
   60, not 61 (design review 1); both ship in the same version. An actor caller's family holds variants of these names; the test pattern of `actor-ports` checks
   that. Not taken: untagged `{"outcome": ...}`. The actor form needs the type name as the message type.
4. **The outcome is sent in a turn of its own.** A send to an actor that does not exist rejects the whole commit. If
   the caller had been deleted, a reply sent with the end would keep the run from ever ending.
   - **The turn that commits the end** (done, cancelled or failed, as in `flow-ends`) also arms a `Continue` when a
     route is saved.
   - **That `Continue`, or any other `Continue`**, that reaches an ended run whose saved route has not been replied to
     sends the outcome through the route, sets `replied: true` and clears `armed`. When its send fails for good, the
     run stays ended, and the core reports the failed message to watchers as for any message. A public `Continue`
     sends it again later, for example after an operator purged the armed one, as a public `Continue` recovers a run
     in `flow-ends` (design review 1). An ended run that has replied, or has no route, ignores `Continue` as before.
   - **Correlation:** the outcome carries the route's correlation id (actor form), or is matched by the request id
     (flow form).
   - **Not taken:** the reply in the end turn's own commit, which is simpler but can block the end forever.
5. **A run without a route** sends nothing when it ends, as today.
6. **The contract and ADR 0081.**
   - `FlowMsg` gains `Start`, and the state gains `reply` (absent: none) and `replied` (absent: false).
   - Decode checks: `replied` only with a `reply`, and on a done, cancelled or failed run `armed` only while a saved
     route has not been replied to. This changes `flow-ends`' "nothing armed" invariant.
   - Every flow contract changes; this ships in the same version as `flow-ends`, so flow contracts break once.
   - ADR 0081 records `Start`, the outcome's wire, the separate outcome turn and the late `Start`.
7. **Tests** run on virtual time with `TestActors`:
   - A flow caller's `request_flow` gets `FlowDone` with the output, and `FlowFailed` and `FlowCancelled` for the
     other ends.
   - An actor caller gets `FlowDone` in its own family, with its correlation id.
   - A second `Start` keeps the first route.
   - A `Start` to an ended run replies at once.
   - A deleted actor caller (its run made with `Target::create`, not as its child): the run still ends, the outcome
     message fails without changing the run, and a later public `Continue` tries again.
   - A second `Start` with another route fails for good; one with the same route does not.
   - An output 60 levels deep is replied.
   - A run without a route sends nothing.
   - Decode refuses the broken invariants.
   - `FlowOutcome`'s variants decode as a caller's family.
   - The test flow's contract is regenerated.

## Docs this breaks

None beyond `flow-ends`' blocks: the items are new. The handoff asks for the actors guide's flow section on starting
and awaiting a run (both callers), the design guide's text on requests, the flow paragraph of `docs/architecture.md` (ended runs ignore `Continue`,
`Reply` and `Cancel`, except the outcome), and the upgrade note together with `flow-ends`'. The CHANGELOG names the
break for hosts that match `FlowMsg` exhaustively.
