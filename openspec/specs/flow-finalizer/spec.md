# flow-finalizer Specification

## Purpose
A cancelled flow runs its own durable cleanup, `Flow::on_cancel`, before it counts as cancelled: `Cancel` marks the
run cancelling, the cleanup replays a journal of its own with the ordinary step, send and wait helpers and read-only
lookups of the main progress, and only a successful cleanup marks the run cancelled.

## Requirements

### Requirement: A defaultable cancellation hook replays a separate cleanup journal

Flow SHALL provide a default-success on_cancel(&mut Cleanup, owned Input) Send future of HandlerResult. Cleanup SHALL
expose ordinary Step helpers through Deref/DerefMut over its own journal. The hook SHALL run only in cancelling,
never after normal done or an already-cancelled run. A successful hook SHALL record cancelled and armed=null only
with no latch and every saved cleanup entry consumed. All first-latch, replay, error and capacity rules SHALL remain.

#### Scenario: Default and custom hooks

- **WHEN** a running flow receives Cancel
- **THEN** that turn commits cancelling and a cleanup Continue without running the hook
- **WHEN** the executing cleanup Continue uses the default hook
- **THEN** it records cancelled in a separate turn
- **WHEN** a custom hook suspends through a helper
- **THEN** only that helper's latched outcome commits, and the hook is replayed later

#### Scenario: No normal-finally behavior

- **WHEN** a done run receives Cancel or a predecessor-cancelled run is read after upgrade
- **THEN** no hook runs and the terminal state stays unchanged

#### Scenario: Completion discipline

- **WHEN** the hook returns Ok before reaching a saved cleanup entry, or catches a permanent/in-progress helper failure
- **THEN** it fails permanently and cannot record cancelled
- **WHEN** a new cleanup entry would exceed 1000
- **THEN** it fails under the existing bound without changing main progress

### Requirement: Cleanup phase dispatch and keys cannot resume main work

Continue SHALL choose its execution phase from committed status. A non-armed Continue SHALL normalize as before;
an armed Continue in cancelling SHALL invoke on_cancel and never Flow::run. Repeated Cancel in cancelling SHALL be
an unchanged success, not a recovery poke. Cleanup external keys SHALL be <canonical run>/cancel/<id>; the real run
address SHALL remain the address for request routes and self-target checks. Main keys SHALL stay unchanged.

#### Scenario: Stale continuation and repeated Cancel

- **WHEN** a stale, public, retried or redriven main Continue reaches cancelling
- **THEN** it normalizes or executes only cleanup
- **WHEN** Cancel is repeated during cleanup
- **THEN** no state, armed id, retries or journal entries are reset and cleanup can continue

#### Scenario: Separate namespaces

- **WHEN** main and cleanup use the same step id
- **THEN** their external keys differ by /cancel/ and each journal replays independently
- **AND** ReplyTo.run and self-send refusal use the actual run address, not the cleanup key prefix

### Requirement: Cleanup exposes stable main-progress lookups with latch protection

Cleanup SHALL provide main_key, main_result<T>, main_request<T>, main_reply<T> and raw main_journal. Fallible lookups
SHALL honor the existing latch, validate ids/entry kinds, protect custom deserialization with InProgress and latch
permanent errors. They SHALL consume no step ids or replay cursor. main_key SHALL work for any valid id even if
unsaved; result/request SHALL return None only for absence, not wrong kind. Raw journal reads SHALL expose no reply list.

#### Scenario: Missing and wrong kind

- **WHEN** main_key names a valid unsaved id
- **THEN** it returns the original main external key
- **WHEN** main_result or main_request names an absent valid id
- **THEN** it returns None
- **WHEN** the id is invalid or names the wrong entry kind, or main_reply receives a non-main request
- **THEN** it latches permanent failure rather than treating it as absence

#### Scenario: Pure repeated lookup

- **WHEN** a valid lookup is repeated before a latch
- **THEN** it leaves both journals, used-id sets and cursor unchanged
- **WHEN** it is called after a latch
- **THEN** it returns Suspend without decoding or replacing the first latch

#### Scenario: Lookup decoding cannot be swallowed

- **WHEN** a typed lookup fails or a custom deserializer panics and user code catches it then returns Ok
- **THEN** the saved failure/marker prevents cancelled and later helpers do nothing
- **AND** saved data remains available for recovery after code correction

#### Scenario: Raw journal view

- **WHEN** a host reads main_journal
- **THEN** it gets only immutable saved entries, including step values and send destinations
- **AND** docs distinguish host decoding of these raw values from protected typed lookups

### Requirement: Main reply lookups hide mutable early-reply availability

MainReply<T> SHALL be Open when no closed main wait covers the request, even if a reply is already saved or cleanup
later closes it. When a closed main wait covers it, it SHALL be Replied(T) for a saved value, otherwise Closed.
These answers SHALL remain stable throughout cleanup. Open replies SHALL be consumed through cleanup waits that
record a decision, never exposed as changing branch data. MainReply and Cleanup SHALL introduce no wire schema.

#### Scenario: Reply arrives during cleanup

- **WHEN** a main request has no closed main wait and its reply arrives between cleanup replays
- **THEN** main_reply returns Open before and after arrival
- **AND** a cleanup wait can durably select that reply before later code receives it

#### Scenario: Closed main winner or loser

- **WHEN** a closed main wait covers the request
- **THEN** main_reply returns Replied for any retained value, including an early saved loser, or Closed if no value exists
- **AND** no future reply can change that lookup result

### Requirement: Cleanup waits may consume still-open main requests

Cleanup waits SHALL accept earlier cleanup requests and main requests not covered by a closed main wait, including
never-awaited requests and interrupted main waits. A join MAY mix them. Each request SHALL remain usable in only one
cleanup wait. Main progress SHALL never be rewritten. Any closed wait in either journal SHALL block new replies;
accepted replies in cancelling SHALL affect only the active cleanup wait, preserving existing selection/time rules.

#### Scenario: Interrupted main and closed cleanup overlap

- **WHEN** cleanup waits for a request also in an interrupted open main wait and receives its reply
- **THEN** cleanup can close and eventually finish while the main wait remains open and unchanged
- **AND** later duplicate or late replies are ignored because the cleanup wait closed it

#### Scenario: Mixed join and early reply

- **WHEN** cleanup joins an unclosed main request and a new cleanup request
- **THEN** both are eligible under existing all/first/timeout rules
- **AND** early replies accepted in cancelling retain acceptance order without closing a main wait

#### Scenario: Closed or reused request

- **WHEN** cleanup tries to await a main request covered by a closed main wait, including a first-wait loser,
  or a request used in another cleanup wait
- **THEN** it fails permanently and directs the host to new cleanup work where needed

#### Scenario: Cleanup time and reply races

- **WHEN** cleanup creates a timed wait
- **THEN** its deadline starts at that cleanup turn's claim, independent of the interrupted main deadline
- **AND** zero-timeout open-first behavior, partial all results and actual handling-order races remain unchanged
- **AND** a sufficient reply handled before timeout selection may win after the deadline

### Requirement: Cleanup failures and observations follow ordinary flow recovery

Cleanup continuations SHALL use Flow::retry; handler failures SHALL keep cancelling progress, while unapplied/unknown
commits follow core lease and claim-limit rules. Public Continue SHALL recover the active cleanup phase; repeated
Cancel SHALL not. Delete SHALL stay immediate without finalization and pause SHALL delay cleanup. No automatic
resource cleanup or synthetic core notification SHALL be promised. Application sends and terminal status are separate turns.

#### Scenario: Exhausted or lost cleanup continuation

- **WHEN** cleanup retries exhaust or its armed message is purged
- **THEN** the run stays cancelling and public Continue normalizes then replays saved cleanup progress
- **AND** a parked cleanup wait remains parked until its reply or supported timeout, without polling

#### Scenario: Commit uncertainty

- **WHEN** phase-entry or final-status commit acknowledgement is lost
- **THEN** callers cannot assume rollback or re-run a returned handler merely on that assumption
- **AND** rereads and ordinary fenced recovery preserve whichever transaction committed

#### Scenario: Notification and deletion

- **WHEN** cleanup sends an application notification as its last helper
- **THEN** that send commits before the later turn recording cancelled, not atomically with it
- **WHEN** the run is deleted while cancelling
- **THEN** deletion runs no hook; ordinary core child/removal semantics apply
- **AND** successful finalization emits no synthetic Failed/Stopped

### Requirement: Cleanup guidance covers uncertain external effects and mixed implementations

Guidance SHALL state that absent main-step progress does not prove absence of an external effect. Cleanup by main_key
requires an idempotent owner/protocol that resolves late creation; the library SHALL infer no resources. All hosts
and workers of a type SHALL run the intended finalizer before relying on it, despite equal hashes. Existing hashes
and description-stripped schemas SHALL remain equal to the landed cancellation predecessor.

#### Scenario: External effect without saved result

- **WHEN** an external reservation succeeds but its main step does not commit a result
- **THEN** main_result may be None while main_key still identifies the resource for idempotent cleanup
- **AND** the worked example verifies cleanup after this failure, including its owner's late-creation fencing protocol

#### Scenario: Resource confirmation before capacity release

- **WHEN** a resource-owning request is interrupted before a stable main completion is known
- **THEN** the example sends a durable idempotent Stop request and awaits confirmed removal before releasing capacity
- **AND** a reply arriving during cleanup cannot flip the earlier Open lookup branch

#### Scenario: Old host or default hook

- **WHEN** a predecessor host handles Cancel or an older application binary uses the default hook
- **THEN** it may mark cancelled without intended resource cleanup despite matching the hash
- **AND** deployment guidance requires identical intended flow code everywhere or a new type name

#### Scenario: Lost cleanup reply on an older host

- **WHEN** an older host refuses a cleanup Reply
- **THEN** Continue can resume cleanup but cannot recreate the missing value
- **AND** recovery may require the target to reply again or an existing timeout/application recovery path
