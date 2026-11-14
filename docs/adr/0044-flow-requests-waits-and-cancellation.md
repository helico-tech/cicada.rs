# 0044. Flow requests, waits and cancellation share a wire shape

Date: 2026-11-14

## Status

Accepted for the six-change line starting with `flow-step-sends`. Extends ADR 0031. The owner requested both flow
reply waits and cancellation and delegated design decisions to the design loop with an Opus adviser. Only the
behavior of each landed piece is available; defining a variant does not implement its later behavior.

## Context

Flows can run external steps and sleep, but cannot atomically send actor work or suspend for a reply. Cancellation
must stop normal replay while permitting durable user cleanup. FlowMsg and the journal are part of every hosted
flow's strict contract. Reserving the agreed representation in one breaking release avoids repeated contract resets,
as the initial journal reserved Sleep. This does not make mixed behavioral versions interchangeable.

## Decision

### Wire

FlowMsg remains tagged by type: Continue, Reply with required request (MessageId) and required value (any JSON,
including null), and Cancel. A missing value is not null. New enum wire names/fields use camelCase as below.

| State field | Form | Default and bound |
| --- | --- | --- |
| input | Flow's input | Required; ordinary state depth applies |
| journal | Journal entries | Empty; at most 1000 |
| status | running, cancelling, done, cancelled | running |
| armed | MessageId or null | null |
| replies | Objects with required request and required value | Empty; at most 2000 |
| cleanup | Journal entries | Empty; at most 1000 |

Existing step {id,result} and sleep {id,until} entries keep their shape. A send entry holds id, to (ActorAddress),
message (MessageId), create (null, independent or child; default null), and reply (boolean; default false). A wait
entry holds id, requests (1..100 MessageIds), mode (required all or first), until (Timestamp or null; default null),
outcome (null, received or timedOut; default null), and received (MessageIds, default empty, at most 100).
No message payload or initial target state is saved in a send entry. Input remains limited by the whole state;
new step results/reply values have at most 61 container levels because their state position adds three.

The schema describes field types, required presence, list lengths, id patterns and enums. Typed state checks also
enforce cross-field references; do not claim JSON Schema enforces those. Existing supported step/sleep states decode
with empty new lists; this does not promise to repair arbitrary historical or hand-written states.

### State invariants

- Ids remain 1..100 ASCII characters from A-Za-z0-9._~-:, unique within each journal. Main and cleanup have separate
  namespaces. Every sleep/deadline is at most MAX_INSTANT. Send message ids are unique across both journals.
- Each wait has unique request ids naming reply-enabled sends earlier in that journal. Cleanup may also reference
  a main request not closed by a main wait. Only the last entry of either journal may be an open wait.
- A request belongs to at most one wait per journal. Across journals, the only permitted overlap is an interrupted
  open main wait plus a cleanup wait for its requests, while cancelling/cancelled. The cleanup wait may itself close;
  the main journal remains unchanged. A closed main wait's requests cannot be waited on again in cleanup.
- received is empty for an open wait. For a closed wait it is a unique subset in requests order, with each value
  present in replies. received/all includes every request; received/first includes exactly one. timedOut requires a
  deadline; timedOut/all has a strict subset and timedOut/first is empty. Reply records are unique and name known
  reply-enabled sends. Closed waits prevent further acceptance for their requests, including losing first-wait requests.
- running has empty cleanup. done has empty cleanup, no armed id and no open main wait. cancelled has no armed id
  and no open cleanup wait; an open main wait is interrupted by its terminal status. cancelling permits an interrupted
  main wait and its own cleanup progress. Limits bound outstanding requests as well as retained replies; no unbounded inbox.

Define these checks in the first piece so later implementations do not tighten decoding of already-supported state.
Phase-specific feature refusal is separate from structural validity. Trusted supplied progress is not proof of effects.

### Sends and replies

Step retains no Turn. A new send/request prepares one entry and an outbox. The handler buffers optional create,
send, journal progress and continuation in one ordinary commit. Replay matches id, kind and request flag, and never
converts or serializes the proposed target/message/create state. The saved destination describes what actually ran;
changing unsaved arguments does not rewrite it. Callers still construct arguments in deterministic replayed code.

Before new user conversions/builders/serialization, set InProgress. A caught panic leaves it latched and prevents
later work or successful progress; an unpolled helper does nothing. This extends the existing latch protection without
preempting user code. Sends are enqueued once per committed entry, not an exactly-once target execution guarantee.

A request uses a newly generated message id saved with its send. Replies must name that request and come from its
recorded target address; the target's helper carries the run address and request id. Unknown, wrong-sender, duplicate
or closed replies do not grow state. A target can reply before a wait exists because its request entry was committed
with the send. Ignore checks precede value checks. A new eligible reply with NUL or more than 61 container levels
fails permanently.
Valid reply data is retained for replay; a typed decode mismatch fails wait replay permanently, keeping the outcome.
ReplyTo<T> is a required {run: ActorAddress, request: MessageId} route with a schema independent of T. Its phantom
type connects the target message to the flow's Request<T>; it carries no authenticated external identity.

A wait records one durable decision before code branches on it. Reply turns run no flow code. A reply that completes
the active wait saves the outcome and arms Continue; an expired wait saves timedOut and arms in a separate turn before
later user code can run. The deadline is the earliest time an executing Continue may choose timedOut; reply turns
never consult time, so a reply handled first is received even after the deadline. A new wait never times out in its
creating turn. Timeout is data. Handling order chooses a race; pauses, commits, scheduling and recovery can
delay it. Retained values support single waits, homogeneous typed joins, first-winner selection and partial timeouts.
For first waits, the earliest accepted listed reply wins, using the replies array's order even when replies arrive
before the wait. All selections and partial timeout results follow request order. Supplied progress is trusted,
including reply order; restoration must preserve it. Replay uses the saved selection, never recalculates it.
A join means more than one request or mode first. A one-request all wait is structurally identical to a single wait,
so the single/all helpers may replay it interchangeably when deadline presence matches. Closing a first wait closes
all listed requests; losing targets keep running and cannot be waited on again. Cleanup needs a new request for them.
Target failure/deletion adds no automatic watch; targets report business failure as data or the flow uses a timeout.

Continue armed by a reply uses that reply's correlation id. The reply helper copies its current turn's correlation;
there is no root-correlation field. An early reply closed later by Continue uses that executing Continue's correlation,
not the earlier reply's. A parked indefinite wait has armed=null and sends no polling continuation. The reply count
bound follows from unique replies to bounded send entries. Replying to a deleted run can reject the replier's entire
commit under core rules.
Hosts must not reuse run keys for new jobs: external step keys remain address-based even though request ids are random.

### Cancellation and finalization

Cancel is handled in mailbox order after any current turn, not an interrupt of a running handler. It cancels the
armed continuation and stops normal flow replay. The cancellation piece marks cancelled with no user finalizer; the
finalizer piece changes this to cancelling followed by a separate finalizer over cleanup. The main journal is never
rewritten to mark an interrupted sleep/wait. Finalization may inspect it and await still-open main requests.
Before the finalizer release, cancelling refuses every message. Cancel in done/cancelled is a no-op, and cancelled
ignores late replies before value checks. Cancel acknowledges enqueueing only; an older host's refusal leaves the
run running, so resend after upgrading. Runs cancelled before finalizers are available are not finalized later.
FlowMsg::Cancel is distinct from cancelling one pending message or asking the runtime to cancel a running turn.

The finalizer has its own ids and keys `<run address>/cancel/<id>`, distinct from `<run address>/<id>` because ids
contain no slash. It uses the saved input and ordinary steps/sends/waits. Successful completion alone records cancelled;
failure keeps cancelling progress for recovery. It does not automatically stop children or manage external resources.

Done/cancelled state remains readable. No synthetic core Stopped is emitted: that notification means removal.
Use status reads/change hints or an application message. A final application send and terminal status are separate
turns, not atomic. Deleting a run stays immediate and runs no cleanup.

Flow::on_cancel is a cancellation-only, default-success hook over Cleanup, which exposes Step helpers through Deref.
Even default flows enter cancelling and arm a separate cleanup Continue. Cancel while cancelling is a no-op; public
Continue recovers cleanup. The active phase comes from status, not the continuation's origin. Cleanup uses F::retry.
Main lookups read the fixed main journal; main_key also works for unsaved ids. main_result/main_request return absent
or typed data, refusing wrong kinds. MainReply is Open without a closed main wait (even with an early reply), Replied
for a retained value under a closed main wait, or Closed without one. Thus mutable early replies cannot change cleanup
branches; Open requests must be consumed through cleanup waits. Fallible lookups share Step's permanent/in-progress
latch protections. Raw main_journal reads expose no mutable reply list and host decoding is ordinary replay code.
During cancelling, any closed wait in either journal rejects further replies; only cleanup's active wait is closed.

A missing external-step result does not prove no effect happened. Cleanup by main_key needs an idempotent external
owner that resolves/fences late creation, or another application protocol proving cleanup completed. The library
cannot infer ownership or compensate effects. Mixed crate versions or host binaries using a default/older hook can
record cancelled without intended cleanup despite equal hashes; upgrade all hosts or version the flow type.

### Upgrade and staged rollout

This changes every flow contract and requires a breaking release, but no SQL migration. Hosts can drain/archive old
runs, stop old hosts, deliberately delete completed runs and reset their registrations before upgrading. Alternatively
keep a separately pinned old binary hosting the old types while new code uses new type names on the same store;
other co-hosted types must still have matching contracts. Merely renaming within an upgraded old host is insufficient.

Saved old progress remains decodable, but that alone is not an automatic migration. Export/delete/restore at the same
address needs a tested host-specific procedure for the same job: quiesce all producers/workers, verify backup and
typed/core-limit validation before deletion, clear removed armed ids, restore controls/topology deliberately, then
poke running runs. Mailboxes and instance identity change, children/address watches complicate deletion, and an
uncommitted external step may repeat with its old key. This line does not ship a generic migration utility or promise
lossless arbitrary live migration. Never describe drain-and-delete as preserving the deleted journals.

Equal hashes across the later releases do not enforce feature availability. Upgrade every host/worker of a type
before enabling new flow code or sending newly implemented messages; otherwise older hosts can permanently refuse
them. A new versioned type name is the alternative. Piece 1 refuses Reply/Cancel and cancelling, and cannot replay
wait/request entries; it already ignores Continue in terminal states. Later pieces enable behavior without changing
the reserved shape. Each piece has its own reviewed tasks; unforeseen wire changes require an explicit new decision.

## Consequences

No core/store changes, G/S changes, automatic compensation or exactly-once external execution. There are six small
changes: flow-step-sends, flow-reply-waits, flow-wait-timeout, flow-wait-join, flow-cancellation, flow-finalizer.
The flow keeps bounded but potentially large state. Unsupported features are explicit failures, not silently active
behavior. A helper wait actor would add another lifecycle without removing the need for flow replies; in-band
correlation alone cannot distinguish requests, and core watches are not business-completion notifications.
