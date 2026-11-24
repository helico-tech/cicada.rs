# Docs handoff for capacity-permits

Version: 0.20.8 (opens the unreleased batch after 0.20.7). Refresh the dependency pins of `docs/how-to/install.md`
once it is published.

## What changed

`cicada-actors` gets a ready-made permit pool (ADR 0058): at most `capacity` requests hold a permit at once, the rest
wait in FIFO order, and each grant carries a number that only goes up per pool, which fences a late release.

Public items (all re-exported at the crate root):

- `Permits::named(pool)`: the ref, `cicada.permits/<pool>`. `PermitsActor`: the actor type, no settings.
- `PermitsState::new(capacity, max_waiting) -> Result<_, InvalidPermits>`: each 0 through 1,000; capacity 0 pauses the
  pool. Fields: `capacity`, `max_waiting`, `next_grant` (starts at 1), `holders: Vec<PermitHolder>` (`request`, `to`,
  `message_type`, `grant`, `notified`), `waiting: Vec<WaitingRequest>` (`request`, `to`, `message_type`). On the wire
  `{"capacity":4,"maxWaiting":100,"nextGrant":1,"holders":[],"waiting":[]}`; a create may leave out `nextGrant`,
  `holders` and `waiting`. The decoder refuses bounds out of range, `nextGrant` 0, more waiting than `maxWaiting`, a
  request ID twice in the books, a grant not in `1..nextGrant`, and request IDs or message types out of bounds.
- `PermitsMsg`: `Acquire { request, to, messageType }` (helper `PermitsMsg::acquire(request, &holder_ref, "Permit")`),
  `Release { request, grant }`, `Cancel { request }`, `Resize { capacity }`, `Audit`; and the pool's own `Notify
  { request, check }`, `Failed`, `Stopped`. A request ID is 1–200 characters without NUL.
- `PermitNotice { pool, request, kind, grant }` with `PermitNoticeKind` = `granted` | `queued` | `refused` | `check`;
  `grant` is set for a holder (on `granted` and on a holder's `check`), null otherwise. A host's family takes it as
  `Permit(PermitNotice)` (any variant name; it is the `messageType` of the `Acquire`).
- `InvalidPermits`.

Behaviour:

- Every `Acquire` is answered in its own commit: `granted` (room and nobody waiting), `queued`, or `refused` (queue
  full; nothing recorded). So an `Acquire` whose target is gone fails its commit (cause `commit`) and records nothing.
- A repeated `Acquire` of a holder answers `granted` with the same number; of a waiter, `queued` with its place kept.
  Both go to the first `Acquire`'s target, whatever the repeat names. A request ID is forgotten at its release: a reused
  ID is a new request and takes a new permit and number.
- After every message the head of the queue becomes a holder while fewer hold than the capacity, in the commit that
  made room. Its `granted` notice, and each `check` of an `Audit`, go out in a later turn of the pool (`Notify`, sent to
  itself with the shared retry policy and the correlation id of the message that made it), which sends what is true
  when it runs. A holder that is gone fails only its own notice.
- `Release` frees a permit only when that request holds that grant number; otherwise it is ignored. `Cancel` takes a
  waiter out of the queue and is ignored for a holder. `Resize` grows (grants to the queue) or shrinks (revokes
  nothing; new grants wait until fewer hold than the capacity). `Audit` frees nothing.
- No `check` goes to a holder whose `granted` notice has not committed yet: an audit skips it (the next audit checks
  it), and a waiter checked by an audit but granted before its `check` runs gets only its `granted`. So a grant number
  reaches a holder only through `granted`: start work on `granted`, never on a `check`.
- The pool watches itself once it sends a `Notify`. When its own grant `Notify` fails for a holder whose `granted` never
  committed, it frees that permit and grants the next waiter. A failed `check` frees nothing. A `Notify` or `Failed`
  a client sends does nothing.
- Refused for good, state unchanged: `Resize` above 1,000, a request ID out of bounds in any message, a message type out
  of bounds, a target without a registered type or whose type does not take the message type. A NUL is refused by the
  core at the send.

Proved by `crates/cicada-actors/tests/permits.rs` (27 tests on virtual time, one or more per spec scenario), the unit
tests in `crates/cicada-actors/src/permits.rs`, and `tests/wire_formats.rs` (contract against
`docs/reference/contracts/cicada.permits.json`, samples of each message, states with holders and waiting requests, a
notice of each kind, and a state the pool writes). No worked example in `examples/` (design: the skill test would ask
a route for it, which needs the guide section first).

## 1. A permit pool section in the actors guide

**Where.** `docs/guide/actors.md`, a new section `## Permit pool` (anchor `#permit-pool`), after `Batcher`; and a row
in the table at the top: `| Cap how many jobs run at once | PermitsActor / cicada.permits | [permits tests](../../crates/cicada-actors/tests/) |`.
**Text.**
- Create the pool with its state: `PermitsState::new(capacity, max_waiting)`, then host `PermitsActor`. A create of a
  pool that exists changes nothing; `maxWaiting` changes only by delete and create.
- Use a fresh request ID per attempt, such as `<job>-<attempt>`. Send `PermitsMsg::acquire(request, &me, "Permit")`;
  the holder's family takes `Permit(PermitNotice)`. Rust block from the `PermitsMsg::acquire` doc example in
  `crates/cicada-actors/src/permits.rs`.
- The four kinds: `granted` (start the work; keep `grant`), `queued` (wait), `refused` (queue full: try later or give
  up), `check` (an audit asks: release if the work is confirmed stopped, cancel if you gave up waiting, ignore it while
  working).
- Pass the grant number to the resource you guard, so it can refuse work under an older number.
- Release with `Release { request, grant }` when the work ends. A release with another number is ignored.
- A `Cancel` that loses the race to its grant does nothing: the holder then gets `granted` for a request it cancelled,
  and must release it.
- An undelivered grant comes back: a holder deleted before its `granted` notice reached it costs nothing.
- A holder deleted after it got its number keeps the permit: release before deleting a holder, or an operator sends
  `Release` with the number read from the pool's state. No timeout frees a permit.
- `Resize` changes the capacity; a shrink revokes nothing.
**Caveats.** One permit per `Acquire`; no priorities, weights or expiry. Request IDs are not remembered after a
release. A notice that is not an `Acquire`'s answer costs one more turn of the pool.

## 2. The AGENTS.md map

**Where.** `AGENTS.md`, `Map`, the `crates/cicada-actors` row.
**Text.** Add `permits` to the list of ready-made actor modules (after `batcher`).

## 3. The architecture catalog

**Where.** `docs/architecture.md`, the `cicada-actors` row of the layer table.
**Text.** Add: a permit pool (`cicada.permits`) that lets at most its capacity of requests hold a permit at once,
queues the rest in FIFO order up to a bound, answers each `Acquire` in its commit, fences each grant with a number that
only goes up, frees a delivered permit only on a `Release` of its number, and frees a grant whose notice never reached
its holder (ADR 0058).

## 4. A capacity pattern in the design guide

**Where.** `docs/guide/design.md`, `Coordinate work and time`.
**Text.** To cap how many jobs run at once (a build farm, licensed workers, a host with room for N containers), use
the permit pool (a link to `actors.md#permit-pool`): a job asks with a fresh request ID, starts on `granted`, passes the grant
number to the resource, and releases when its work is confirmed stopped. The pool frees nothing on a timeout.

## 5. The skill route

**Where.** `skills/cicada/SKILL.md`, the `Ready-made actors` route row.
**Text.** Add `docs/guide/actors.md#permit-pool` and `docs/reference/contracts/cicada.permits.json` once section 1
exists (the skill test checks that every route resolves, so this lands with or after the guide section).

## After landing

Link the change in issue #2 (https://github.com/helico-tech/cicada.rs/issues/2).

## Blocking

None. No doctest, link, guarantee row or skill route needs new prose: no guide, README or skill route names the pool
yet.

## Status

Checks run on this branch: `cargo test -p cicada-actors` (all pass), `cargo test -p cicada-examples --test docs` and
`--test skill`, and the finishing checks of task 6. All sections above are proposed text; no prose is written yet.
