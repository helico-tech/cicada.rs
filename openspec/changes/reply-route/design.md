# Design

A port such as Factory's `repos.mirror` is called by actors and by flows. A flow gets a `ReplyTo<T>` from
`step.request`, while an actor has only its address. Today the port needs two messages, or each project writes its own
enum. The owner agreed on one route in Cicada (P1, 2026-11-28); this change follows ADR 0044 for the flow side. The
picks below are mine, under the owner's A1.

## Decisions

1. **`ReplyRoute<T>`, in its own module.** `pub enum ReplyRoute<T> { Flow(ReplyTo<T>), Actor { to: ActorAddress,
   correlation: Option<String> } }` lives in `crates/cicada-actors/src/reply.rs` and is exported from the crate root. `Clone`, `Debug`, `PartialEq` and
   `Eq` are implemented by hand without bounds on `T`, as for `ReplyTo`. `From<ReplyTo<T>>` gives the flow form, so a
   flow writes `|r| Ensure { repo, reply: r.into() }`. `ReplyRoute::actor(to: ActorAddress)` gives the actor form
   without a correlation id, and `.correlation(id)` sets one. An actor writes
   `ReplyRoute::actor(turn.self_address().clone()).correlation(id)`. The constructor takes an address, not an
   `ActorRef`, because `ActorRef::address` can fail (design review 1, note). The private schema struct `ReplyRoute`
   in `flow.rs` is renamed to `ReplyToSchema`, so the name means one thing. The crate root's table of items gets a
   row. Not taken: the name `Reply`, because `FlowMsg::Reply`, `FlowReply` and `MainReply` already exist and a
   fourth "Reply" invites mistakes. `ReplyRoute` also says what it is: a route, not a reply. `flow.rs` already holds
   over 3,000 lines, so the type gets its own module.
2. **One `send(turn, &value)`.** It has the same shape as `ReplyTo::send`: it encodes and checks synchronously, and
   returns a `Send` future that borrows only the turn.
   - **Flow form:** it is exactly `ReplyTo::send`.
   - **Actor form:** it sends `value` as a message to `to`, with the same type-name checks as `Turn::send_to`. A
     caller whose family lacks the reply's type name, or whose type is not registered, makes the send fail with
     `TurnError::SendRejected` in the target's turn. The message carries the route's correlation id when it has one,
     else the turn's. It has the observers' headers and the default retention, and is due now with no retry policy,
     as `ReplyTo::send` does.
   - A value that does not encode as a message fails with `TurnError::Invalid` and buffers nothing. A `T` that is no
     tagged enum, such as `u32`, is such a value, and the rustdoc says the actor form needs one.
   - Not taken: two traits that bound `T` per form. Both forms share one field type, so the check can only happen at
     runtime.
3. **The wire.** The flow form is the wire of `ReplyTo`, `{"run": ..., "request": ...}`, so a saved `ReplyTo<T>`
   decodes as `ReplyRoute::Flow`. The actor form is `{"actor": "<address>"}`, with an optional `"correlation"`.
   - Decoding is decided by the key: an object with `actor` is the actor form, and one without it is the flow form.
   - An object with `actor` and also `run` or `request` fails to decode, and so does one missing its required
     fields. Unknown other fields are ignored, as in `ReplyTo`.
   - Not taken: a plain `#[serde(untagged)]`. It would silently read `{"run", "request", "actor"}` as the flow form,
     and its errors name no field.
   - The schema is the same for every `T`, named `ReplyRoute`: `oneOf` the `ReplyTo` schema and an object with a
     required string `actor` and an optional string `correlation`. The schema is looser than the decoder: it does
     not forbid `run` next to `actor`. JSON Schema can say this only with a `not`, which no other contract here uses,
     so the decoder is the authority.
4. **Correlation travels with the route.** A target can keep the route and reply in a later turn: a port that
   finishes its work after a timer, or a flow run that replies when it ends (`flow-request-target`, from a `Continue`
   turn). That turn's correlation id is not the request's (design review 1, blocking). So the actor form carries the
   caller's correlation id, and `send` uses it when it is set. A caller with several open requests sets a correlation
   id on each request's route and matches each reply by it, or the port puts a business key in its reply family.
   Without a correlation id in the route, the reply carries the turn's correlation id, which is right only when the
   target replies in the request's own turn. Not taken: the request's message id in the route, because the caller
   learns it only after the send. Also not taken: only stating the limit, because the next change needs the later
   reply.
5. **The match of the families stays a runtime check.** For the actor form, `T` is the port's reply family, and the
   caller's family must hold variants of the same names and fields. The test pattern is the one `actor-ports` uses:
   encode each variant, then decode it as the caller's family. A helper waits for `test-probe`. Compile-time checks
   stay out of scope.
6. **ADR 0079** records the route: one field type for flow and actor callers, its wire, decoding by key, and
   correlation by correlation id. It adds to ADR 0044, which stays in force; `ReplyTo` and its wire do not change.
7. **Tests.**
   - Unit tests in `reply.rs` for the wire: both forms round-trip; a saved `ReplyTo` decodes as `Flow`; the cases that
     fail to decode; and the schema is equal for two `T`s.
   - Tests on virtual time with `TestActors`: an actor caller gets the reply in its own family with its correlation
     id; a target that replies in a later turn sends the route's correlation id; a flow caller's `step.request` with
     `r.into()` gets it in `step.wait`; a `u32` sent by the actor form gives `Invalid` and buffers nothing; and a reply
     whose type name the caller's family lacks gives `SendRejected`.

## Docs this breaks

None. The items are new, and no doctest, guarantee row or skill route names them. The handoff asks for the actors
guide's text on requests, near `step.request`, and the design guide's section on ports from `actor-ports`: one field
`reply: ReplyRoute<T>` serves both callers. It also asks for an upgrade note: moving a field from `ReplyTo<T>` to
`ReplyRoute<T>` keeps saved messages readable, but changes the port's contract hash.
