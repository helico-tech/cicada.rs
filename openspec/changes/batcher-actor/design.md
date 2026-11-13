# Design

## Scope

One actor batches by count or time on the public core API. The owner's Opus 5.5 adviser chose the policies below;
the owner authorized designing all remaining changes autonomously. Four implementation tasks plus finishing checks.
`BatcherActor` is a unit struct; `Batcher { name }` with `named` addresses `cicada.batcher/<name>`.

## Decisions

### Fixed settings and a bounded item count

`BatcherState` has public `target: ActorAddress`, `message_type: String`, `max_items: u32`, `max_wait: Duration`,
`retry: Option<RetryPolicy>`, `items: Vec<Value>`, `first_at: Option<Timestamp>`, `armed: Option<MessageId>`.
Serde uses camelCase and ISO-8601 durations; retry/firstAt/armed default to None, items to empty; unknown keys ignored.
`BatcherState::new(&target_ref, message_type, max_items, max_wait) -> Result<Self, InvalidBatch>` creates an empty
state and validates the ref address and settings. InvalidBatch implements Error. A checked serde conversion also
validates settings and state: `1 <= maxItems <= 1000`, `0 < maxWait <= MAX_DELAY`, and a nonempty messageType at most
`MAX_TYPE_LENGTH` characters, without NUL. The type is checked against the registry only at forwarding.
Items must number less than maxItems. Nonempty requires firstAt; empty requires no firstAt or armed. Nonempty with
no armed is valid for recovery. FirstAt must not exceed MAX_INSTANT. Retry applies to the target, not Add or Flush.

No default maxItems: a host must choose it. The 1,000 cap keeps whole-state rewrites modest, like dead-letter capacity;
it bounds item count, not bytes. No third expected-count rule: one key per group with maxItems as its expected count
and maxWait as timeout already aggregates. Live updates and counters would add contract surface without a request.

### Inline flushing enforces the count

`BatcherMsg` is tagged `Add { item: Value } | Flush`. `BatcherMsg::add(item: impl Serialize)` returns
`Result<Self, EncodeError>`; scalars and JSON null are valid items. Add appends in handling order, setting firstAt to
`turn.now()` only when previously empty. If the count reaches maxItems, send and clear in this turn immediately.
Otherwise compute the deadline `firstAt + maxWait` with checked arithmetic. If now is at/past it, send and clear
including the new item. If not, the first item arms Flush at that deadline with the shared retry policy and saves
its id; later items keep that timer. No intermediate count-full state commits. maxItems 1 sends each Add directly.

Not chosen: a separate immediate Flush on count. Previously queued Adds could run before it and exceed maxItems;
while the target was absent state could keep growing. Inline flushing makes each stored state strictly smaller
than maxItems, even during failure. It couples the final Add to forwarding, a deliberate trade-off.
Example: maxItems 3, maxWait 10 s; a at 0, b at 1, c at 2 sends [a,b,c] at 2 and cancels the timer; d at 4 arms 14.

### Public Flush and atomic delivery

Any Flush sends the current items, or ends done when empty. Send the configured message type with payload
`{"items": [...]}`. In the same commit cancel armed unless it is the current message id, clear items/firstAt/armed.
The outgoing message gets a new id, batcher as sender, state.retry, no correlation id, default retention and normal
observer headers. Incoming Add/Flush headers are not copied. Per-item correlation/reply data belongs in the item.
Not chosen: a correlation id picked from one item (misrepresents mixed producers), or own-only Flush (blocks redrive).
A manual Flush, deliver-now or redriven old Flush intentionally delivers the current batch early, even a later batch.
An empty Flush does not stop the instance. All settings remain fixed at creation.

### Failures and recovery

A call-level send rejection is permanent (refused), store read errors propagate normally for the incoming envelope's
retry policy, and an absent target instance rejects the commit (commit, no retry). Failure preserves the earlier
state and any other timer; the current failing message itself ends or reschedules under core rules. Terminal failures
notify watchers; the batcher watches nothing. A target decoding/handling failure occurs after clearing and refunds none.
When an Add triggers a failed flush, its new item is not retained: that Add's failed outcome identifies it, and dead
letters keep it where they watch. Retrying/redriving Add appends that item to the then-current batch and can reorder it.
A failed Flush is recoverable by public Flush, retry or redrive; an Add at/past the deadline also flushes the batch
with the new item. A new Add before the deadline keeps the old timer reference, even if it was cancelled or purged;
it recovers only on count, on/after deadline, or through explicit Flush. Core cancel/purge notify nobody.

### Time, depth and creation

Use store time at claim. Check arithmetic and the chosen deadline against MAX_INSTANT; overflow fails permanently
with no effects. The count-full path needs no deadline and may send immediately near MAX_INSTANT. Core delivery
rounds up to microseconds and clamps a past deadline at commit. maxWait is a scheduled wait, not a wall-time guarantee
through pauses, outages, retries or slow commits. Equal-time Add/Flush follow the core's `(deliverAt, seq)` order.

Items sit two levels down in state and in the outgoing payload, leaving at most 62 container levels per item. Add's
own envelope allows 63. Use the existing core guards: a too-deep retained state fails after the handler (handler);
a too-deep outgoing payload is rejected by send_raw and refused permanently. Either rolls back all Add effects.
No separate depth decoder is needed. Scalar null remains an item, not a missing batch. The guide states this boundary.
Hosts create explicitly, or create-and-add atomically. Repeated create keeps state. Delete discards retained items;
the actor never stops itself. Deleting it does not recall already forwarded batches to independent targets.
No ADR: a new actor contract alters no existing wire format/guarantee, following the scheduler's settled precedent.

## Integration and checks

Pin the contract and empty/nonempty state/message samples; host the type in the sample service without instances and
update its snapshots/texts. Add a tested example/guide doctests with a target enum containing `Items { items: Vec<_> }`.
Show count/time behaviour, early/recovery Flush, failed final Add, group aggregation and count/depth/time limitations.
Update guide tables, README, crate docs, architecture's crate row, AGENTS map and changelog. Use virtual-time TestActors
for count, timers, FIFO handling, failures/recovery, depth, creation, envelope and same-time ordering; the existing
SQLite registry-failure fixture covers store failures if needed. Unit tests cover validation and checked timestamps.

Review 1 found no blockers. Its note adds a restored nonempty state without armed: a pre-deadline Add keeps it
unarmed; an overdue Add or explicit Flush recovers it. The spec and implementation tests cover this state.
