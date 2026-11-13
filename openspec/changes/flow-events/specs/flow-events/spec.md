## ADDED Requirements

### Requirement: An event actor keeps explicit bounded subscriptions

EventActor SHALL use type name cicada.event, one instance per name. EventState SHALL hold flows in subscription order,
defaulting to empty. Decoding SHALL validate at most 100 unique TypeName-parseable names. Subscribe and Unsubscribe
SHALL parse names and refuse invalid ones permanently. Existing Subscribe and absent Unsubscribe SHALL be no-ops.
A new 101st subscription SHALL fail permanently. Registration SHALL be checked on publication, not subscription.

#### Scenario: Subscription limits

- **WHEN** 100 flow names are subscribed and an existing name is subscribed again
- **THEN** it succeeds without duplication
- **WHEN** a different 101st name is subscribed
- **THEN** it fails permanently with the list unchanged

#### Scenario: Invalid subscription state or command

- **WHEN** a state contains duplicate/invalid names or over 100 entries
- **THEN** decoding refuses it
- **WHEN** Subscribe or Unsubscribe contains an invalid name
- **THEN** the command fails permanently

#### Scenario: Explicit startup subscription

- **WHEN** a host creates an Event actor and subscribes a valid but not-yet-registered type
- **THEN** the subscription is kept without a registry lookup
- **AND** repeating create-and-subscribe preserves existing subscriptions without duplicating the name

### Requirement: Publications have explicit general and batch forms

Publish SHALL carry input as any JSON value. PublishBatch SHALL carry an items array and be equivalent to Publish with
input `{"items": items}`. One batch SHALL start one run per subscribed type, not one per item. Empty batches SHALL
be valid. Missing input or nonarray items SHALL fail decoding. Serialization helpers SHALL return EncodeError for
values they cannot serialize; no extra recursive depth check SHALL be added.

#### Scenario: General JSON input

- **WHEN** Publish carries an object, array, scalar or explicit null
- **THEN** that exact value becomes each new run's input

#### Scenario: Direct batcher composition

- **WHEN** a batcher sends message type PublishBatch with payload `{"items":[1,2]}`
- **THEN** each subscribed flow receives one run with input `{"items":[1,2]}`

#### Scenario: Empty batch

- **WHEN** a client sends PublishBatch with an empty array
- **THEN** subscribed flows receive one run each with input `{"items":[]}`

#### Scenario: Missing publication fields

- **WHEN** Publish has no input, or PublishBatch items is not an array
- **THEN** decoding refuses it, rather than silently treating it as a null input or empty batch

### Requirement: Publication creates and starts all runs in one commit

Publication SHALL derive each run address from the subscribed type and its own envelope id as key. It SHALL buffer an
independent create with FlowState<Value>::new(input), then send Continue with the shared crate retry policy. All
creates and sends SHALL commit atomically. Existing runs SHALL keep their state. Empty subscription lists SHALL
complete with no effects. No run execution order across types SHALL be promised.

#### Scenario: Two subscribed flows

- **WHEN** publication id event-17 is handled with subscriptions shop.email and shop.fulfil
- **THEN** one commit creates shop.email/event-17 and shop.fulfil/event-17 with the input, empty journal, running status
  and armed null, and sends each Continue

#### Scenario: No subscribers

- **WHEN** an event with no subscribers receives Publish
- **THEN** it completes without starting runs
- **AND** a later Subscribe does not replay that successful publication

### Requirement: Publication keys use canonical raw-name construction

Publication SHALL construct addresses with ActorAddress::of from the raw envelope id, not parse an interpolated
address. An address-construction error SHALL fail permanently with cause refused and no effects. Literal `.` and `..`
SHALL therefore be refused when subscribed runs are needed. With no subscribers no address SHALL be constructed and
both ids SHALL remain successful no-ops. There SHALL be no alternate mapping or id prefix.

#### Scenario: Reserved actor keys

- **WHEN** either Publish or PublishBatch has id `.` or `..` and at least one subscription
- **THEN** it fails permanently before creating or starting any run
- **WHEN** the same kind of publication has no subscriptions
- **THEN** it completes successfully with no effects

#### Scenario: Recovery with a new key

- **WHEN** a publication refused for a dot id is retried/redriven with a generated valid id, or republished under a valid id
- **THEN** the subscribed runs use the new id and start normally

#### Scenario: Percent-encoding is applied once

- **WHEN** distinct publications have raw ids `a/b` and `a%2Fb`
- **THEN** their run address suffixes are `a%2Fb` and `a%252Fb`, respectively, without collision

### Requirement: Initial continuation context is deliberate

Each initial Continue SHALL have a new id, Event actor as sender, the publication's correlation id, default retention
and the shared crate retry policy. Incoming arbitrary headers SHALL NOT be copied; observer headers still apply.
The flow's normalization turn SHALL select F::retry() before flow code executes. A publication envelope's policy SHALL
control only that publication turn.

#### Scenario: A flow with its own policy

- **WHEN** a publication starts a flow whose retry policy differs from the crate default
- **THEN** the initial Continue uses the crate policy and the subsequent armed execution uses the flow's policy
- **AND** the publication's correlation id follows the continuation chain

#### Scenario: Batch correlation

- **WHEN** a batcher publishes without a correlation id
- **THEN** its runs have no continuation correlation id

### Requirement: Event-turn failures commit no partial fan-out

Call-level create/send rejection SHALL fail publication permanently; store errors SHALL follow its envelope policy.
Any failure SHALL leave all of that turn's creates and sends uncommitted. Core terminal failure notifications SHALL
apply. A stale subscription SHALL block publication until corrected. No event or target watch SHALL be installed
automatically; later flow failures SHALL NOT roll back a successful publication.

#### Scenario: Stale subscription after a valid one

- **WHEN** publication buffers one valid run then encounters an unregistered subscribed type
- **THEN** it fails permanently and creates/starts neither run
- **WHEN** that subscription is removed and the failed event is retried
- **THEN** the new publication starts only the remaining subscribed types

#### Scenario: Store outage

- **WHEN** a publication with retry cannot read the registry, and the store recovers before its next attempt
- **THEN** no partial runs exist from the failure and retry starts each subscribed run once

#### Scenario: Failure after publication

- **WHEN** publication commits but one run's continuation fails terminally
- **THEN** the publication remains done and the host recovers the run through Continue or its own watcher

### Requirement: Input checking retains raw-create boundaries

Publication SHALL use create_raw/send_raw checks. A hosting node SHALL decode prospective input/state and refuse the
whole publication when invalid. A node not hosting the flow SHALL NOT claim semantic validation; the run may fail
later with cause state. Registry acceptance SHALL NOT be treated as proof that a type implements Flow. The guide SHALL
require subscribing only compatible flow types.

#### Scenario: Invalid hosted input

- **WHEN** an Event actor runs on a node hosting a subscribed typed flow and its input does not decode
- **THEN** the entire publication fails permanently with no new runs

#### Scenario: Invalid unhosted input

- **WHEN** a node hosting Event but not the subscribed flow publishes input that the flow cannot decode
- **THEN** raw creation can commit, and the run's first message fails later with cause state
- **AND** other valid runs from that committed event are unaffected

#### Scenario: An ordinary actor is subscribed

- **WHEN** a registered subscribed actor lacks Continue
- **THEN** publication is refused without partial creates
- **AND** the guide states that an actor accepting Continue still cannot be proven to be a flow by the registry

### Requirement: Deduplication distinguishes runtime retries from new publications

A runtime retry SHALL retain the envelope id and run keys. Within the core dedup window, a repeated incoming id SHALL
add no effects. After expiry a repeated id MAY publish again using current subscriptions: retained run addresses keep
their original state, done ignores Continue and running can recover. New subscriptions or deleted addresses MAY create
runs. No permanent event ledger SHALL be added.

#### Scenario: Same id inside the window

- **WHEN** a client repeats an accepted publication id within core retention
- **THEN** no further creates or Continue messages are added

#### Scenario: Same id after the window

- **WHEN** an id is republished after core dedup expiry while its runs are retained
- **THEN** their original input and journal remain, and no extra run address is created for those types

#### Scenario: New subscription after expiry

- **WHEN** an id is republished after expiry and a new flow has been subscribed since its first publication
- **THEN** that newly subscribed type can get its first run at that id while existing run states remain unchanged

#### Scenario: Prospective input is still checked

- **WHEN** an id is republished after expiry with input invalid for a locally hosted flow whose run already exists
- **THEN** create_raw can refuse the publication before its idempotent create would preserve that existing run

### Requirement: Operator retries and redrives use fresh run keys

Operator retry and dead-letter redrive SHALL keep their core behavior of creating a new publication id, hence new run
keys. A failed publication has committed no runs to duplicate. Repeated operator retry calls MAY start multiple fresh
publications. Recovery of an already-started run SHALL use Continue rather than republishing its successful event.

#### Scenario: Redrive a failed publication

- **WHEN** a failed publication is redriven after its bad subscription is corrected
- **THEN** runs use the redriven message's new id, not the failed publication's id

### Requirement: Run identity and independent lifecycle are documented

The guide SHALL prohibit reusing keys for different jobs, including across event names or after retention/deletion,
and require manual run keys to avoid publication keys. Unsubscribe or deleting Event SHALL NOT remove existing runs.
Fresh scheduler/forwarder/batcher publication ids SHALL create fresh runs. Recreating the same address SHALL retain
ADR 0031's same external step idempotency keys, not claim a new job identity.

#### Scenario: Two scheduler slots

- **WHEN** a scheduler publishes the same input on two slots with fresh message ids
- **THEN** they create distinct run addresses

#### Scenario: Removing an event

- **WHEN** a host unsubscribes a flow or deletes the Event actor after publication
- **THEN** its independent runs remain

### Requirement: Publication wrappers respect the core depth limit

Publish SHALL allow input through 63 container levels as permitted by its envelope and run state. PublishBatch's run
state adds three containers before an item, leaving at most 61. Existing create_raw state validation SHALL refuse an
oversized prospective run without partial effects. No additional checker SHALL run when there are no subscriptions.
The guide SHALL distinguish this from the batcher's 62-level item limit and explain downstream failure recovery.

#### Scenario: Batch depth boundary

- **WHEN** a subscribed flow receives PublishBatch with a 61-deep item
- **THEN** its run state fits the core depth limit
- **WHEN** an item is 62 deep
- **THEN** publication fails permanently at create_raw and no run starts, even though the batcher could forward it

### Requirement: Contracts and composition have examples and fixtures

The Event contract and state/message samples SHALL be pinned. The sample service SHALL host Event without automatic
instances/subscriptions. Tested examples and guide doctests SHALL cover explicit subscription, general and batch input,
watches/recovery, and composition with scheduler, limiter, debouncer and batcher. Existing flow and actor contracts
SHALL remain unchanged.

#### Scenario: Host-defined batch input

- **WHEN** a flow declares Input with an items array and a batcher targets PublishBatch
- **THEN** the documented example compiles and delivers one run for each flushed batch without an adapter actor
