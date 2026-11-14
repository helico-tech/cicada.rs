# Design

## One contract and one implemented behavior

ADR 0044 fixes the shared messages, state, entries, bounds and cross-reference invariants for the six agreed pieces.
This piece implements only a journaled send with optional create. Add all agreed wire types now, following the original
Sleep reservation, but do not expose helpers that pretend reserved behavior works. Every hosted flow's contract changes;
regenerate both test and sample-service flow snapshots. Event's own contract stays unchanged; its raw FlowState creation
must remain compatible. New fields default empty/null as specified; new value fields are required even when null.

Implement the ADR's structural checks now, including interrupted-main/cleanup wait overlap. Keep schema bounds and
typed cross-field checks distinct. Normal old step/sleep states still decode; do not add a new depth check to reads of
old result values. Continue in done/cancelled is a no-op. Reply and Cancel are permanently refused as needing a later
cicada-actors release; cancelling is refused. Running code that meets a wait or reply-enabled send fails replay
permanently; malformed state fails through ordinary state decoding. Reservation is not a runtime feature gate.

## Public API and replay

Add Target<'a,R,S: ?Sized=()> holding a borrowed ref and optional borrowed initial state plus CreateKind. Constructors
are Target::existing(&ref), Target::create(&ref,&state), Target::child(&ref,&state); From<&R> supplies existing. None
serializes or resolves anything. Re-export the public helper/types used in public fields. Step::send takes id,
impl Into<Target<'a,R,S>> + Send and impl Into<R::Message> + Send, returning HandlerResult as an async function.
Bounds include R: ActorRef + Sync + 'a and S: Serialize + Sync + ?Sized + 'a. Check ordinary &ref inference and
create/child inference in compile-tested examples; the flow future must stay Send. No closure is needed for send.

Caller argument construction still happens in replayed flow code and must be deterministic/free of external effects.
The promise is that Into conversions, ref.address(), message serialization and create-state serialization do not run
on replay, after a previous latch, or for an unpolled helper. Dropping arguments may run their ordinary Rust Drop;
this API cannot suppress caller/destructor effects. Request's later ReplyTo builder is a separate API.

On first poll, check the existing latch, then replay id/kind and reply==false, then journal room. A match advances the
cursor and returns Ok without touching the proposed target/message/state. Destination and create kind are saved facts,
not replay comparisons; changed arguments do not rewrite prior effects. A request entry cannot replay as a plain send.
Invalid/repeated ids, mismatch or the 1001st entry latch permanent failure. Keep existing early-return checks.

For a new send, set InProgress with kind/id before any user conversion. Convert the target, compute its address and
refuse the run's own address; convert/encode the message, then encode any initial state. An ordinary error replaces
the marker with Permanent. A caught panic leaves the marker, so later helpers run nothing and the returned flow fails
permanently. An uncaught panic follows the core's ordinary panic handling. There is no internal await; first poll
finishes or unwinds. Existing run/sleep behavior stays, with marker representation generalized only as needed.

Prepare a fresh MessageId and a send entry containing id, destination, message id, create kind and reply=false.
Latch New with that entry and an outbox holding message type/payload and optional create state. Return private Suspend;
the latch wins over the flow's eventual return. No new progress is committed just by building this outbox.

## Applying effects

After Flow::run returns with this latch, call create_raw/create_child_raw if requested, then send_raw with the saved
message id, incoming correlation id, immediate delivery, no retry policy and ordinary retention/header propagation.
Append the entry and arm Continue using the existing flow retry/correlation policy. Effects, progress and continuation
commit together. The core's create-before-send order makes a newly created target available in that commit.

Registry Store errors retain their typed error chain (HandlerError::new or ?) so the cause is node and the executing
Continue's retry policy applies. Invalid/SendRejected become permanent_from errors, cause refused. Keep step/address
context without flattening the store error into text or changing its retryability. Self/address/encode errors are
permanent before buffering; core validates NUL/depth/null initial state while applying the outbox. Validation runs
even if a create would reuse an existing actor. Any failure commits neither partial effects nor journal progress.

Independent create reuses existing state under core rules; child create cannot take over an actor with a different
parent. A collision or absent destination discovered at commit rejects that turn (cause commit) and stalls the run.
After correction a public Continue recovers it; no journal entry or request id survived the rejected commit. A failed
registry attempt may generate another id on retry. Host-supplied progress is still trusted, not verified against effects.

Replay never enqueues another message for that committed entry; target execution retains normal at-least-once rules.
No outgoing retry policy is inherited from the flow: a target handler failure ends that message unless its own design
handles recovery. Step::send promises enqueueing, not successful job completion. send_raw also checks registry/message
membership even when the corresponding typed Turn send would not. No send options API in this piece.

## Upgrade and verification

Use a breaking version chosen when built, not a stale exact version. Document supported deployment choices: drain and
archive before deliberate reset, or keep old binaries hosting old names while new binaries host new names. Inputs and
other co-hosted types still obey strict hashes. Add old-state decode/replay fixtures and an example showing independent
contract registration for old/new type names. Migration notes also cover exhaustive matches on FlowMsg, FlowStatus
and JournalEntry, and FlowState struct literals. Do not claim an upgraded binary retains the old schema automatically.

Same-job export/restore is a host-specific operation, not a utility shipped here. Its caveats belong in the guide:
all producers/workers quiesced, verified backups/typed decode/core limits, removed armed ids cleared, pending messages
lost, control/topology restoration and possible repeated uncommitted external effects with the same key. No automatic
conversion or claim of JSON Schema validation. Changed code and equal-hash feature releases still require all hosts
upgraded before new behavior is enabled; an old node can permanently refuse a later Reply or Cancel.

Unit tests pin every structural invariant and raw-null/required-field schema behavior, send replay/conversion counters,
caught panics, latch precedence, unpolled futures and capacity. Virtual-time actor tests prove atomic create/send,
replay after a later failure, wrong-parent/absent-target rollback, retryable registry faults and reserved refusals.
Check depth64/NUL/null initial state, correlation and no outgoing retry. An EventActor regression must create/start a
flow through its raw FlowState::new JSON, with Event's contract unchanged. Existing run/sleep tests stay green. Update
guides, architecture, ADR, changelog and appropriate existing G2/G4 proof links. Five implementation tasks plus finishing.
