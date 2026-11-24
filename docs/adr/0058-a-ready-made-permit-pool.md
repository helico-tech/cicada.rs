# 0058. A ready-made permit pool

Date: 2026-11-24

## Status

Accepted for capacity-permits. Builds on ADR 0028.

## Context

The rate limiter spaces out starts, but nothing caps how many jobs run at once. A build farm, a pool of licensed
workers or a host with room for N containers needs that cap. cicada-code-factory has one (`factory.limiter`): a
capacity from its configuration, holders, a FIFO queue, and an audit. It never meets a holder that is gone, because it
creates each holder before it sends to it. Issue #2 adds two rules: a repeated acquire returns the same grant, and a
stale release never frees a newer grant. A permit must not come back on a timeout alone, as the holder's outside work
may still run.

## Decision

- A ready-made permit pool in `cicada-actors` under the fixed type name `cicada.permits`, one actor per pool. Its
  capacity and queue bound are in its state, given at the create; `Resize` changes the capacity, and a shrink revokes
  nothing.
- One permit per `Acquire`, keyed by a request ID. A repeated `Acquire` answers the same grant; a full queue refuses
  and records nothing. Grants go in FIFO order, in the commit that made room.
- Every grant takes a number that only goes up per pool. `Release` names the request and the number, and is ignored
  for another number. `Cancel` only leaves the queue. The holder may pass the number to the resource it guards.
- A permit whose holder received its number comes back only by an explicit `Release`. `Audit` asks every holder and
  waiter to check in, and frees nothing itself.
- Notices go to a target and message type named in the `Acquire`, with a fixed payload. Every `Acquire` is answered
  in its commit (`granted`, `queued` or `refused`), so a gone target or a wrong type never joins the queue. Every
  other notice is sent in a turn of the pool's own, so a holder that is gone blocks no other request.
- The pool watches itself. When its own grant notice fails before the holder ever received the number, it frees the
  permit: no work can run under a number that was never delivered.
- Bad input (a field out of bounds, a target or type the send refuses) is refused for good, the state unchanged.

## Consequences

- Pools under the one type name can have different capacities, and a capacity changes without a migration.
- A holder that is gone after it received its number keeps its permit until an operator releases it: no timeout
  guesses that its work stopped. Hosts release before they delete a holder. One that was deleted before its grant
  reached it costs nothing.
- Request IDs are not remembered after a release: a host uses a fresh ID per attempt, and the grant number fences a
  late release of the old one.
- A notice that is not part of an `Acquire` costs one more turn of the pool; throughput is not a goal.
