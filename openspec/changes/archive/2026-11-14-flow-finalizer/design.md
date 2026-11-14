# Design

## Cancellation-only hook and context

Add Flow::on_cancel(&self, cleanup:&mut Cleanup, input:Self::Input) -> impl Future<Output=HandlerResult>+Send,
with a default successful future. Existing flows compile; normal done never invokes it. The name avoids implying
finally-after-success. Cleanup wraps the cleanup-phase Step and Deref/DerefMut exposes its ordinary helpers. It owns
a fixed main-journal snapshot. Owned cloned input matches run. No separate trait, feature flag or retry-policy API.
Reconcile all five unbuilt predecessors; this remains in cicada-actors with no core/store/wire/hash/decoding change.

Every running Cancel commits cancelling, cancellation of older armed work (never the current message), and an
immediate cleanup Continue with F::retry() and Cancel's correlation. It invokes neither hook nor input clone. Even
default hooks use this two-turn transition: putting user cleanup in Cancel would roll back the phase on failure.

| Status | Continue | Reply | Cancel |
| --- | --- | --- | --- |
| running | Normalization/main replay | Main wait routing | Enter cancelling and arm cleanup. |
| cancelling | Normalize a non-armed id; otherwise on_cancel | Accept across journals; route only to cleanup wait | No-op, no state write. |
| done/cancelled | No-op | Ignore before value checks | No-op, no hook. |

The runner chooses phase from committed status, never message origin. Stale/public/redriven main Continues cannot
run main code after cancellation. Repeated Cancel does not reset progress, retries or armed work; Continue recovers
stalled cleanup. A default hook completes on the next executing Continue. Old cancelled runs are never reopened.

## Stable main-progress lookups

Cleanup provides these synchronous methods; it and MainReply are not serialized:
- main_key(&mut self,id:&str) -> Result<String,HandlerError>: valid id gives <run>/<id>, whether saved or not.
- main_result<T:DeserializeOwned>(&mut self,id:&str) -> Result<Option<T>,HandlerError>: saved step result.
- main_request<T>(&mut self,id:&str) -> Result<Option<Request<T>>,HandlerError>: committed main request handle.
- main_reply<T:DeserializeOwned>(&mut self,request:&Request<T>) -> Result<MainReply<T>,HandlerError>.
- main_journal(&self) -> &[JournalEntry]: raw immutable entries, including destinations and sleeps.
MainReply<T> is #[must_use] Open, Replied(T) or Closed, with ordinary Debug/Clone/PartialEq/Eq bounds.

MainReply consults whether a CLOSED MAIN wait covers the request. Without one it always returns Open, even if a reply
is saved or a cleanup wait later closes. With one it returns Replied for a retained value (including an early loser),
otherwise Closed. Main closed waits prohibit new replies, making this view stable. Never expose mutable early reply
availability as branchable lookup data: cleanup consumes Open requests only through its own journaled wait.

For all fallible lookups, first latch wins: after a latch return Suspend without work. Invalid ids, wrong entry kind
or a main_reply handle not belonging to a main reply-enabled send latch Permanent. An absent id returns None for
result/request; wrong kind does not masquerade as absence. main_key validates syntax only. Typed decode sets
InProgress first; success clears it, an error sets Permanent and a caught panic leaves the marker. Catching an error
and returning Ok cannot mark cancelled. Lookups consume no ids/cursor and may repeat. main_journal has no latch or
user decoding; host decoding of its raw values is ordinary replay code without these protections.

## Phase keys, waits and completion

Keep the real ActorAddress separate from the key prefix: main keys <run>/<id>, cleanup keys <run>/cancel/<id>.
Ids have no slash, so namespaces cannot collide; request routes and self-target checks still use the real address.
Cleanup replays its own journal from zero and appends only there. Main progress is never rewritten or replayed.
A successful hook with no latch marks cancelled/armed=null only after consuming every saved cleanup entry. Early
return/mismatch fails permanently; New/Close/Waiting/Parked and failure latches retain predecessor behavior.

A new cleanup wait may use earlier cleanup requests and main requests not covered by a closed main wait, mixing
both in one join. Reuse in another cleanup wait is refused. This includes never-awaited main requests and requests
in an interrupted open main wait, even after their cleanup wait closes. Closed main requests/first losers require
new cleanup requests. Keep the existing structural overlap rules, bounds and global send-id uniqueness unchanged.

During cancelling, reply acceptance checks known request across both journals, saved target sender, duplicate and
ANY closed wait covering it before value checks. Save eligible early replies, but close only the active cleanup wait.
Never close the interrupted main wait. A cleanup-closed request is closed even if main still lists it as open.
Partial joins keep armed; sufficient replies arm with incoming reply correlation. Cleanup deadlines start at the
claim creating the cleanup wait, not main's saved deadline. Zero, timeout races, first selection and partial results
are unchanged. Cleanup can sleep/park indefinitely; no timeout kills a target or forces successful finalization.

Hook/step handler failures follow the executing Continue's policy; permanent errors remain stalled cancelling.
A lost continuation or exhausted retries recover with public Continue, which normalizes then replays cleanup.
Unapplied commits and unknown acknowledgements use normal core lease/claim-limit rules; do not assume rollback or
handler retry after uncertainty. Default cleanup and final status commits are fenced/atomic like ordinary turns.
Delete is immediate with no hook; pause delays both phases. No library compensation or automatic child cleanup.

## External effects and the resource example

A main external step can act and then fail to save its result. main_result=None does not mean no resource exists.
Use main_key to release/query by the original stable key even without progress. Safe release also needs the external
owner to fence/tombstone late creation, or a protocol proving in-flight creation settled; no library guarantee covers
that race. The same rule applies to cleanup's external effects. For runner-managed progress, missing main_request
means its atomic Start send did not commit; supplied/restored journals remain trusted claims, not proof.

The worked example reserves capacity by key, starts a resource-owning actor by request, and cancels while waiting.
If a CLOSED main wait confirms Finished, use that stable result. Otherwise send an idempotent Stop request and await
Stopped before releasing capacity by the original reservation key. The owner acknowledges only after resource removal
and fences late Start retries. A main reply arriving during cleanup does not change an Open lookup branch. All actual
effects stay inside steps. An application notification is a separate send entry before the final cancelled commit,
not an atomic terminal-status notification. Successful phase changes emit no synthetic core Stopped/Failed.

## Rollout, documentation and checks

All hosts/workers need this release AND identical finalizer code, or new type names: an older cancellation host can
mark cancelled immediately and permanently skip cleanup; an older/default-hook host can do the same despite equal
hashes. An old host may also fail cleanup continuations/replies, leaving an indefinite wait without its lost reply.
Continue alone cannot recreate missing data: it resumes cleanup; recovery may need the target to reply again or an
existing timeout/application recovery path. Changing on_cancel while cleanup is active needs drain/versioned types.

Clarify ADR 0044; public descriptions may change but hashes/description-stripped schemas must equal the landed
cancellation predecessor. Existing ReplyTo/Event/other actor contracts stay unchanged. Add rustdoc, tested examples,
changelog and a docs-handoff.md for the docs session (AGENTS.md ownership): actors guide, architecture, upgrading and
operations receive the behavior/limits above after implementation. Reconcile old task paths with current doc ownership.
Five tasks plus finishing, with virtual-time TestActors for transitions, lookups, overlap, faults and resource safety.
