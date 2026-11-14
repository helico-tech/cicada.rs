# Design

## Scope and public API

Use ADR 0044's reserved wire unchanged. The predecessor is ready but unbuilt; reconcile its Target, outbox, latch and
state checks before implementing. This piece enables requests and one indefinite reply wait. Cancel, cancelling,
deadlines and joins remain unsupported. No core/store change, automatic watches or HTTP callback service.

Request<T> holds a private MessageId and PhantomData<fn() -> T>, with id(), Clone and Debug without T bounds. It is not
serialized and has no public constructor. ReplyTo<T> carries required public run: ActorAddress and request: MessageId,
plus a private phantom. Its JSON is {run,request}, independent of T. Implement Serialize/Deserialize, Clone, Debug,
PartialEq and JsonSchema without T bounds; ignore unknown fields. Pin one stable ReplyTo schema for all T. A target
message field ReplyTo<JobResult> supplies the inference that an untyped route would lose.

Step::request<'a,R,S,T,B>(&mut self,id:&str,to:impl Into<Target<'a,R,S>>+Send,build:B)
returns Result<Request<T>,HandlerError> asynchronously. Bounds: R:ActorRef+Sync+'a, S:Serialize+Sync+?Sized+'a,
B:FnOnce(ReplyTo<T>)->R::Message+Send. Step::wait<T:DeserializeOwned+Send>(&mut self,id:&str,request:&Request<T>)
returns Result<T,HandlerError> asynchronously. Both futures stay Send; verify inference for &ref and create/child.
Example: request("start", &job, |reply| JobMsg::Start { reply }).await?, then wait("done", &request).await?.

ReplyTo<T:Serialize>::send<'t,A:Actor>(&self,turn:&'t mut Turn<A>,value:&T) returns a Send future of
Result<MessageId,TurnError> + 't. Encode/check value synchronously, capture owned route/value/result, then send_raw
Reply when polled. The future borrows neither self nor value; T need not be Sync. Refuse encoding/NUL/depth>61 as
TurnError::Invalid before buffering. No latch here: this is an ordinary target handler helper. Preserve raw-send
Store/Invalid/SendRejected errors for the caller; copy current correlation, immediate delivery, default retention,
observer headers, no retry. A missing run rejects the whole replier commit. No options API is needed.

## Request replay and creation

Check existing latch, replay id/kind/reply=true, then room. Replay returns the saved id without target conversion,
builder invocation or serialization; plain sends do not match. Invalid/repeated ids, mismatch and entry 1001 refuse.
For a new request set InProgress before Into/address/build/encoding. Refuse self target, generate a fresh id, build
ReplyTo with the run address and id, invoke the builder, encode message and optional state, then latch New/outbox and
Suspend. Store the ActorAddress in Step. An unpolled helper does nothing; caller construction/Drop is ordinary code.
Caught user-code panics leave InProgress, blocking later helpers and failing the returned flow permanently. Uncaught
panics follow core rules. Apply creates/send/progress/Continue atomically through the predecessor path. Registry
Store errors keep cause node/retryability; Invalid/SendRejected are permanent refused. Rejected commits save no id.

## Reply turns

Reply executes no flow code. Done/cancelled succeeds without state writes; cancelling remains permanently refused.
For running, locate a reply-enabled send in either journal, verify incoming from equals its saved to, then ignore
already-stored replies or requests closed by a wait. Unknown/non-request/wrong-sender/duplicate/closed replies succeed
without touching state, before checking value. Required-field decoding still occurs before this handler.

Check a new eligible value for NUL/depth>61 and fail permanently without progress if invalid. Null is valid. Push one
reply. If it completes a supported last open main wait (one request, all, no deadline), close that entry as received,
select its request, cancel the prior armed id if not the current message, and arm now with Flow::retry() and this
reply's correlation. Otherwise retain an early reply without changing armed. A reserved open join/deadline wait is
not executed: an eligible reply for it fails permanently with a later-feature diagnostic, saving nothing.
The 2000 reply bound follows from unique replies to at most 2000 sends; there is no independent inbox-full case.

Sender checking trusts actor addresses, not lifetimes or human identities. HTTP SendRequest has no from field; lease
completion supplies the leased actor as sender. Children/delegates and dead-letter redrive use their own addresses
and do not qualify; a delegate must send its result through the original target. Workers are trusted under the core
lease protocol. Target failure/deletion produces no automatic reply; an indefinite wait can remain parked forever.

## Wait decisions and recovery

Check latch, replay id/kind, then require requests == [handle.id], mode all, until null. Only a closed received entry
returns data, decoded as T from the saved reply. Decode errors latch permanent failure, preserving the saved outcome;
fix code and poke to recover. Protect custom Deserialize with InProgress during decoding and clear it on success;
a caught panic must not permit later work. This does not change existing Step::run replay behavior.

A saved open wait with a reply (possible in supplied progress) latches Close; without one it latches Parked. A new
wait checks room and an earlier reply-enabled send in this phase, unused by another wait. It latches a new closed
entry if the reply is present, otherwise an open entry. Every new/closing/parking path suspends; first latch wins.
The runner appends or closes the entry and arms now for closed decisions. For open/Parked it sets armed=null, without
sending Continue or adding an entry on replay. The current armed message completes normally. No handler stays open.

Data reaches code after wait only on replay of an already committed closed outcome, even for early replies. The
closing turn never calls Deserialize<T>. Public Continue keeps the existing normalization turn, followed by replay:
an open wait parks again without journal growth; a lost/purged continuation after closure recovers. Extra Continue
uses the existing cancellation/re-arm rule. Paused runs handle queued replies on resume. All changes are atomic.
An early reply closed by Continue uses that Continue's correlation, since stored replies carry no correlation.

## Contracts, rollout and checks

No flow wire or strict-hash change relative to the predecessor; verify the actual generated test/sample-service
flow snapshots and existing ready-made actor contracts remain equal. Pin ReplyTo's required fields/address/id schemas
and equality for different T, plus a new target contract fixture. Adding ReplyTo to a host target changes its contract.
Upgrade every host/worker before enabling requests/waits: a predecessor node refuses Reply and loses it; dead-letter
redrive cannot repair its sender. A predecessor stalled on request replay recovers by Continue after upgrade.

Clarify ADR 0044 and its index; update architecture, actors guide, operations, examples and changelog/version at build.
Use virtual-time TestActors for early/late replies, atomic rollback, pause/recovery, first-value wins, sender identity,
61/62/63 depths, null/missing value, correlations, no outgoing retry, request/wait capacity and latch counters/panics.
Unknown or ignored replies leave state/version unchanged; message completion itself is still a committed effect.
Five implementation tasks plus finishing. No timeout/join/finalizer implementation, or new G/S guarantee.
