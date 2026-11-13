## ADDED Requirements

### Requirement: A batcher has fixed validated settings

The actor SHALL use type name `cicada.batcher`. State SHALL hold target, messageType, maxItems, maxWait, optional target
retry, items, optional firstAt and armed. Settings SHALL validate at construction and decoding: maxItems is 1..=1000,
maxWait is positive through MAX_DELAY, and messageType is nonempty, has at most MAX_TYPE_LENGTH characters and no NUL.
There SHALL be no automatic instance or live settings message. Missing items SHALL default to empty.

#### Scenario: Invalid settings

- **WHEN** construction or decoding receives maxItems zero or above 1000, zero/oversized maxWait, or invalid messageType
- **THEN** it returns an error

### Requirement: Stored batches are smaller than the count limit

State decoding SHALL require fewer than maxItems items. Nonempty items SHALL require firstAt; empty SHALL have no
firstAt or armed. FirstAt SHALL NOT exceed MAX_INSTANT. Nonempty without armed SHALL be allowed for recovery.
The constructor SHALL create an empty batch. Optional retry SHALL default to none.

#### Scenario: Invalid initial progress

- **WHEN** a create supplies maxItems items, nonempty items without firstAt, empty items with a timer, or invalid firstAt
- **THEN** state decoding rejects it

### Requirement: Add appends in handling order and flushes inline at the limit

Add SHALL accept a JSON item and append it in handling order. Its typed serialization helper SHALL return EncodeError
on serialization failure. The first item SHALL set firstAt to store time at claim. Reaching maxItems SHALL forward
and clear in that same turn; no count-full state SHALL commit and no immediate self-Flush SHALL be added.

#### Scenario: Count flush

- **WHEN** maxItems is 3 and Adds a, b, c and d are already queued
- **THEN** handling c forwards [a,b,c] and clears atomically; handling d starts a new batch containing only d

#### Scenario: Single-item batches

- **WHEN** maxItems is 1 and an Add arrives
- **THEN** that turn forwards a one-item batch immediately without arming a timer

#### Scenario: A null item

- **WHEN** an Add contains JSON null
- **THEN** it counts as one item and is forwarded as null in the items array

### Requirement: The first retained item arms the longest wait

For a batch below maxItems, Add SHALL calculate `firstAt + maxWait`. If now is at/past it, Add SHALL forward the batch
including its new item. Otherwise only the first item SHALL arm public Flush at the deadline with the shared retry
policy. Later Adds SHALL preserve firstAt and the armed timer. Unsupported deadlines or overflow SHALL fail permanently
with no effects. A count-triggered immediate flush SHALL NOT need a future deadline.

#### Scenario: Time flush

- **WHEN** maxItems is 3, maxWait is 10 seconds, and one item arrives at time zero
- **THEN** Flush is due at time 10 seconds and forwards [a] when handled

#### Scenario: New batch deadline

- **WHEN** a batch is flushed at 2 seconds and a new item arrives at 4 seconds with maxWait 10 seconds
- **THEN** its new Flush is due at 14 seconds

#### Scenario: Overdue Add

- **WHEN** a batch has missed its deadline and an Add runs before any successful Flush
- **THEN** that Add forwards the retained items plus its new item in one turn

#### Scenario: Deadline overflow

- **WHEN** the first retained item would need a deadline beyond MAX_INSTANT
- **THEN** its Add fails permanently and keeps no item or timer

### Requirement: Public Flush sends the current batch and clears atomically

Any Flush SHALL send the current items as the configured message type with payload `{"items": [...]}`, cancel armed
unless it is its own message id, and clear items/firstAt/armed in the same commit. Empty Flush SHALL end done with no
effects. No sender/id check SHALL restrict it and the actor SHALL NOT stop itself. The inline flush SHALL use the
same delivery and cancellation rules.

#### Scenario: Early manual flush

- **WHEN** an operator sends Flush before the timer is due
- **THEN** it forwards the current partial batch and cancels the timer

#### Scenario: Timer flush and empty flush

- **WHEN** the armed Flush handles a batch
- **THEN** it delivers without attempting to cancel its own message
- **WHEN** another Flush runs before any new Add
- **THEN** it sends nothing and leaves the instance alive

#### Scenario: Equal-time Add and Flush

- **WHEN** a partial batch's timer and an Add have the same due time
- **THEN** sequence order decides whether the new item joins that batch or starts the next
- **AND** cancelling a pending timer cannot cancel the current message itself

### Requirement: A delivered batch carries the selected target policy

Forwarding SHALL preserve item handling order and use a new message id, the batcher as sender, state.retry, no
correlation id and default retention. Incoming headers SHALL NOT be copied; normal observer headers still apply.
The incoming Add/Flush envelope policy SHALL govern only that turn. Per-item correlation or reply data belongs in items.

#### Scenario: Distinct policies and context

- **WHEN** an Add carrying correlation c and policy P triggers forwarding with target policy Q
- **THEN** the batch has policy Q, no correlation id, a new id and the batcher as sender
- **AND** target retries bypass the batcher

### Requirement: Failed forwarding rolls back the triggering turn

Call-level send rejection SHALL be permanent (refused); a missing instance SHALL reject the commit (commit).
Store errors SHALL use the incoming envelope's retry policy. Failures SHALL preserve the earlier batch and any other
timer. A flushing Add's new item SHALL NOT be retained if the turn fails. Terminal failures SHALL notify watchers
under core rules; no target watch SHALL be installed.

#### Scenario: Count flush to an absent target

- **WHEN** [a,b] is retained with maxItems 3 and Add c tries to flush to an absent instance
- **THEN** Add c fails with cause commit, [a,b] remains, and the timer is not cancelled
- **AND** further failing Adds cannot grow stored items past two

#### Scenario: Failed final Add is redriven

- **WHEN** the target is restored and the failed Add c is redriven while [a,b] is still retained
- **THEN** [a,b,c] is forwarded once and the batch clears

#### Scenario: Refused message type

- **WHEN** forwarding names an unregistered target type or unsupported message type
- **THEN** the Add or Flush fails permanently with cause refused and earlier items remain

#### Scenario: Temporary registry failure

- **WHEN** a timer Flush's registry read fails and the store recovers before its policy retries it
- **THEN** the batch remains until the successful retry forwards it once

#### Scenario: Target cannot decode the batch

- **WHEN** forwarding commits but the target rejects its payload
- **THEN** the target's message fails with the whole batch and the batcher stays cleared

### Requirement: Recovery flushes current data

Manual Flush, operator retry and dead-letter redrive SHALL flush current items, including a later batch, without
restoring an old snapshot. Timer loss SHALL leave items retained. A subsequent Add SHALL recover on count or at/past
the old deadline; below the limit before that deadline it SHALL keep the old timer reference. The guide SHALL name
explicit Flush as recovery and explain that cancellation/purge are silent.

#### Scenario: A cancelled timer

- **WHEN** a timer is cancelled, then an Add arrives below the count limit before the deadline
- **THEN** the items remain and no replacement timer is armed
- **WHEN** Flush is sent manually or another Add arrives at/past the deadline
- **THEN** the current batch is forwarded

#### Scenario: Old flush redriven into a new batch

- **WHEN** an old failed Flush is redriven after its original items were delivered and new items are waiting
- **THEN** it flushes the new batch early

#### Scenario: A restored partial batch without a timer

- **WHEN** a host explicitly creates nonempty items with firstAt and no armed timer, then adds below the limit
  before the deadline
- **THEN** the items remain unarmed
- **WHEN** an Add arrives at/past the deadline or an operator sends Flush
- **THEN** the current batch is delivered

### Requirement: Core depth checks protect the batch transaction

Items SHALL fit the core's limits in both state and outgoing payload: the two wrapping containers leave at most 62
levels per item. An Add's deeper item SHALL fail without committed state, timer or send effects. The existing
post-handler state guard SHALL handle retained items; send_raw SHALL guard the outgoing payload.

#### Scenario: Depth boundaries

- **WHEN** a 62-deep item is added
- **THEN** it can be retained and forwarded
- **WHEN** a 63-deep item is added to a partial batch
- **THEN** retention fails with cause handler, or an immediate send is refused, and the earlier batch is unchanged

### Requirement: Creation and documentation retain the core semantics

The guide SHALL show explicit and atomic create-and-add, count/time flushing, group aggregation, recovery and its
failure trade-off. It SHALL distinguish bounded item count from bytes and scheduled maxWait from execution time.
The contract SHALL be pinned and the sample service SHALL host the type without automatic instances.

#### Scenario: Repeated create

- **WHEN** a producer creates an existing batcher with different settings and adds an item
- **THEN** the original settings and prior items remain, with the new item handled normally

#### Scenario: Pause and delete

- **WHEN** a timer becomes due during pause
- **THEN** forwarding waits for resume and a claim
- **WHEN** the host deletes the batcher
- **THEN** retained items are discarded but previously forwarded batches to independent targets remain
