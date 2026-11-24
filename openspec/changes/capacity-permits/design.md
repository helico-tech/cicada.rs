# Design

Picks of the change loop, 2026-11-24, on the proposal's list, checked against the code and the reference
(cicada-code-factory `src/actors/limiter.rs`), updated after design review 1. Decisions 7–9 are new: the reference
never meets a gone holder, as it creates each holder before it sends. ADR 0058.

## Decisions

1. **A fixed type name, `cicada.permits`, one pool per key**: `Permits::named(pool)`, hosted by `PermitsActor` with
   no settings, as the rate limiter and the batcher. Not a host-chosen type name: the pool holds no host types.
2. **Capacity and queue bound live in the state, fixed at the create**: `PermitsState::new(capacity, max_waiting)`,
   each 0 through 1,000; 0 capacity pauses the pool. The host or an operator creates the pool with it; a create of a
   pool that exists changes nothing. On the wire: `{"capacity":4,"maxWaiting":100,"nextGrant":1,"holders":[],
   "waiting":[]}`. The decoder checks the bounds, `nextGrant >= 1`, `waiting.len() <= maxWaiting`, unique request IDs
   and grants below `nextGrant`. Not the host's configuration (the reference's `max`): one type then has one capacity
   for all pools. Not a default at registration: a pool without a create cannot take messages.
3. **`Resize { capacity }` changes the capacity**; a growth grants to the queue's head. A shrink revokes nothing:
   holders keep their permits, and new grants wait until fewer hold than the capacity. `maxWaiting` stays fixed
   (delete and create to change it). Not revoking: the pool cannot stop outside work.
4. **`Acquire { request, to, messageType }`, one permit each.** The request ID (1–200 characters, no NUL) names the
   attempt; `to` and `messageType` name where notices go, the batcher's form, as the grant number is known only later
   (not a `StoredMessage`, as `container-actor` found). With room it grants; else it joins the FIFO queue; with the
   queue full it refuses and records nothing. A repeated `Acquire` gets the same answer: `granted` with the same
   number, or `queued` with its place kept; both keep the first target. **A fresh request ID per attempt**: IDs are
   not remembered after a release, so a reused ID is a new request and takes a new permit. Not reusing is the host's
   duty. Not several permits per `Acquire`.
5. **Fencing: a grant number that only goes up per pool.** Each grant takes `nextGrant` and raises it; the `granted`
   notice carries it. `Release { request, grant }` frees the permit only when that request holds that number, and is
   ignored otherwise. `Cancel { request }` takes a waiter out of the queue and ignores a holder: freeing a permit
   without its number would bypass the fence. A holder that gets `granted` for a request it cancelled releases it.
   The holder may pass the number to the resource it guards.
6. **Notices**: the pool sends `messageType` to `to` with `PermitNotice { pool, request, kind, grant }`, `kind`
   `granted`, `queued`, `refused` (queue full) or `check`; `grant` is set for a holder. A host's family takes it as
   `Permit(PermitNotice)`. Each has the crate's shared retry policy. Helper:
   `PermitsMsg::acquire(request, &holder_ref, "Permit")`. `queued` is new: every `Acquire` sends its answer in its
   commit, so a gone target or a type the target does not take is caught before the request joins the queue.
7. **Every other notice goes in a turn of its own.** A grant made by `Release`, `Cancel`, `Resize` or a freed
   permit (decision 8), and each `check` of an `Audit`, is sent by an internal `Notify { request, check }` that the
   pool sends itself and acts on only when it sent it, as the topic's `Deliver`. It reads the state when it runs and
   sends what is true then, or nothing for a request no longer in the books. So a gone holder fails only its own
   notice. Not in the same commit: one deleted waiter would make every release fail. Not creating the holder first
   (the reference): the pool does not know the holder's type or state.
8. **An undelivered grant comes back.** Each holder has `notified`: true once a `granted` notice to it committed (in
   the `Acquire`'s commit, a repeated `Acquire`'s, or a grant `Notify`'s). Each turn that sends a `Notify` also
   watches the pool itself (as the topic). A `Failed` the store wrote for the pool's own grant `Notify`, of a request
   that still holds a permit with `notified` false, frees that permit and grants the next waiter: its commit never
   happened, so the holder never learned the number and no work runs under it. A failed `check` frees nothing (that
   holder has its number). Other `Failed`, a client's, and `Stopped` are ignored. Not accepting the leak: with
   capacity 1, one deleted waiter (a `Cancel` that lost the race to its grant) would stop the pool.
9. **Bad input is refused for good, state unchanged**: a `capacity` above 1,000, a request ID out of bounds (in any
   message), a `messageType` or `to` the send refuses (no such type, a type that does not take the message, a type
   name out of bounds). Not clamping: a silent change is worse than a visible refusal.
10. **`Audit` frees nothing.** It sends `check` to every request in the books, holders and waiters. A holder whose
    work is confirmed stopped answers with `Release`; a waiter that gave up with `Cancel`; one still working ignores
    it. A delivered permit comes back only by `Release`, from the holder or an operator; never on a timeout.
11. **Grants after every message, in FIFO order**: while fewer hold than the capacity and the queue is not empty, the
    head becomes a holder in the commit that made room (the reference's loop); its notice follows (decision 7). So a
    queue is non-empty only when the pool is full, and an `Acquire` with room is granted at once.

## Contract and tests

The contract of `cicada.permits` is checked against `docs/reference/contracts/cicada.permits.json` in
`crates/cicada-actors/tests/wire_formats.rs` (the ten become eleven), with samples of each message, a state with
holders and waiters, and a notice of each kind taken by the schemas and the decoders. Tests in
`crates/cicada-actors/tests/permits.rs`, on virtual time with `TestActors`, cover every scenario of the spec: repeated
acquire, FIFO order, full queue, stale release, cancel while waiting or holding, resize up and down, audit,
undelivered grant freed, failed check kept, foreign `Notify` and `Failed` ignored, and each refusal of decision 9.
Unit tests of the decoder's bounds.

## Version

Additive: patch, in the unreleased 0.20.7 (unless that batch is released first). No guarantee, core wire format,
schema or command line change; a new actor contract, hence ADR 0058.

## Docs it breaks

None. No worked example in `examples/src`: the skill test asks a route for every example, and that route needs the
guide section first. The builder updates the rustdoc table of `crates/cicada-actors/src/lib.rs` (code). For the docs
handoff: a `## Permit pool` section in `docs/guide/actors.md` and its row in the table at its top (create with state,
a fresh request ID per attempt, the `Permit(PermitNotice)` variant and its four kinds, pass the grant number to the
resource, release after a cancel race, answer a check, an undelivered grant comes back, an operator `Release` for a
gone holder that had its number); the pool in the `cicada-actors` row of the AGENTS.md map and in
`docs/architecture.md`'s catalog; a capacity pattern in `docs/guide/design.md#coordinate-work-and-time`; the route
`docs/guide/actors.md#permit-pool` in `skills/cicada/SKILL.md`. After landing, link the change in issue #2.
