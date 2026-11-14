# Design

## Four typed helpers over the same wait representation

Add these async Step methods, with T:DeserializeOwned+Send and no T:Sync bound:
- wait_all(id:&str, requests:&[Request<T>]) -> Result<Vec<T>,HandlerError>
- wait_all_timeout(id:&str, requests:&[Request<T>], timeout:Duration) -> Result<Vec<Waited<T>>,HandlerError>
- wait_first(id:&str, requests:&[Request<T>]) -> Result<(usize,T),HandlerError>
- wait_first_timeout(id:&str, requests:&[Request<T>], timeout:Duration) -> Result<Waited<(usize,T)>,HandlerError>

All results follow input order. Timed all returns one Received/TimedOut slot per request, retaining partial data; at
least one TimedOut means the wait timed out. First returns the winning input index and value. Reuse Waited rather
than adding a result hierarchy; Waited<Vec<T>> would lose partial results. No builder or runtime mode API. A slice
works with Vec and introduces no iterator user code; named handles can be cloned into an array cheaply.

One private wait engine handles the existing single helpers and these four, with pure sufficiency/selection helpers.
Reconcile against the three landed predecessors: they are ready but unbuilt at design time. No core/store change,
new wire/hash, tighter state decoding, parallel mutable Step calls, automatic watches or target cancellation.

## Input, replay and decoding

Check the first latch, then replay id/kind and exact ordered requests, mode and deadline presence. For a new wait,
check id/room, length 1..100, unique handles, earlier reply-enabled sends in this phase unused by another wait, then
deadline bounds before inspecting saved replies. Empty, 101, duplicates, unknown/reused handles and entry 1001 fail
permanently. Reordering or changing requests on replay fails; do not build replay lists from HashMap iteration.
Timed calls reuse the saved deadline, ignoring their new duration, exactly as the predecessor does.

A join means more than one request or mode first. A one-request all wait has exactly the single wait's wire shape:
wait and wait_all([r]) replay each other, as do their timed counterparts. Their return containers differ but their
saved decision does not. Mode first with one request is allowed and remains distinct. Rejecting one-item all joins
or adding a helper-name discriminator would be arbitrary or change the wire; matching is structural.

Only closed outcomes decode T. Decode selected values under InProgress in request order, clearing after success;
an ordinary error latches Permanent, a caught panic leaves InProgress, and both block later helpers and keep the
saved decision. Missing all-timeout positions and first losers are never decoded. Use the saved received selection,
not every value that happens to exist in replies. Helpers after a latch and unpolled helpers do nothing. Caller
argument construction and Drop remain ordinary code. Verify Send with a non-Sync T using real compile tests.

## Select by acceptance, return by request order

For all, sufficiency means every listed request has a reply; selection filters requests in their original order.
For first, sufficiency means any listed request has a reply; select the listed request appearing earliest in the
replies array. That array preserves acceptance order, including for early replies. Thus [a,b] with b accepted first
returns index 1 even when both replies predate the wait. Supplied progress trusts that array order; restore must keep
it. No timestamps or new validation are needed. A closed outcome is never recalculated. Input-priority selection
would choose differently depending on whether the wait existed when the second reply arrived.

## Reply handling and atomic decisions

Keep status/known-request/sender/duplicate/closed/value checks in the existing order. Save a new eligible reply, then
inspect the active last open wait only if it lists this request. A partial all wait saves the reply with armed and
its timer correlation unchanged. A sufficient all/first wait saves received using the pure selection, cancels older
armed work and arms now with the completing reply's correlation and Flow::retry. Reply turns read no clock and run
no flow code. An unrelated early reply leaves the active wait alone, even for supplied already-sufficient progress.

For new or replayed open waits:

| Progress | Decision |
| --- | --- |
| Enough saved replies | Append/close received and arm now, before checking whether a saved deadline elapsed. |
| New, insufficient, untimed | Append open and park with armed=null. |
| New, insufficient, timed | Append open and arm At(until), even for zero duration and partial replies. |
| Saved open, insufficient, timed and due | Close timedOut: all selects the present subset in request order; first selects none. |
| Saved open, insufficient, timed before due | Waiting: re-arm At(saved until), no new entry. |
| Saved open, insufficient, untimed | Parked, no new entry or continuation. |

Every deciding path suspends and commits its entry/selection/armed effects atomically before data can reach later
flow code. First latch wins. A join uses one entry; 100 requests plus the join use 101. Reply depth remains 61; ids
in received add no nesting to values. Failed decisions save nothing; committed choices survive later failures/replies.
The inherited claim-time deadlines, mailbox-order race, public Continue normalization, pause and missing-timer recovery
remain. A partial reply does not reset a deadline. New timed bounds are checked even when enough early replies exist.

A closed wait closes every listed request, including first losers and missing timeout positions. Late replies are
ignored before value checks without state/version changes. Already retained early loser values stay bounded in replies.
No target is stopped. A later finalizer cannot wait again on a closed main request; it must send new cleanup work.
The pure selection logic is phase-independent, but only running/main executes now; Cancel/cancelling stay refused.

## Examples, rollout and verification

Extend the request example with fan-out followed by wait_all_timeout and a separate first-quote case. Result effects
stay inside durable steps, not plain replayed code. Show ordered Received/TimedOut slots and the stable winner index.
Existing flow and actor contracts must match the landed timeout predecessor; a one-item all wait is already supported
there, while real joins are not. Upgrade every host/worker before enabling joins despite equal hashes.

Clarify ADR 0044's acceptance order, structural matching and closed losers; update guide, architecture, operations
and changelog/version at build. Test input/replay/decode limits, early/opposite arrival orders, partial/zero timeouts,
supplied progress, equal-time races, immutable retries, pause and missing timers with virtual-time TestActors. Five
implementation tasks plus finishing. No new guarantee, migration, aggregate size/depth limit or finalizer API.
