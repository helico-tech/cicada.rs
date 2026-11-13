# Design

## Public operations, not a store interface

Export these from runtime with its operation types (no public Backend, transport trait or api wire DTO yet):
`lease_turns(&self, owner: &NodeId, types: &BTreeMap<TypeName, String>, limit: usize)` returns
`Result<LeaseTurnsResult, OpError>`; `renew_turn(&self, actor: &ActorAddress, epoch: i64)` returns
`Result<RenewTurnResult, OpError>`; `release_turn(&self, actor: &ActorAddress, epoch: i64)` returns
`Result<(), OpError>`. ActorSystem owns all three. Completion and Worker follow in their own changes.

`LeaseTurnsResult` is `Leased(Vec<TurnLease>)` or
`ContractMismatch { type_name: TypeName, registered: Option<String>, requested: String }`.
TurnLease has public actor, epoch, message (Message), state (Value), version, claimed_at, lease_until and ttl fields:
the existing Claim fields plus the granted Duration. `RenewTurnResult` is `LeaseLost` or
`Renewed { lease_until: Timestamp, ttl: Duration, cancel_requested: bool }`. Use constructible, cloneable core types
for later fault transports. Release returns no bool because Backend cannot distinguish a release from a stale no-op.

## Admission and identity

Before any store access validate limit in 1..=MAX_LEASE_BATCH (100), 1..=MAX_LEASE_TYPES (100) selected types, and
owner at most MAX_ID_LENGTH (200) characters with no NUL. NodeId retains its existing nonblank constructor rule.
Invalid input is OpError::Invalid. The BTreeMap prevents duplicate names; hashes are compared exactly as strings.
One list_types read checks every selected name/hash; return the first mismatch in type-name order (None if absent),
with no claims. Unknown types never silently disappear from the request. Do not require a server-side handler.

The hash check is deliberately not atomic with claim: deletion, re-registration and new work can race between them.
It catches changed contracts between requests but is not a transactional version pin. As with existing local nodes,
operators must stop old workers before replacing a type's contract; a stronger pin is outside this primitive.
Local and remote execution of an equal contract share ordinary store exclusivity. Owner is the recorded NodeId,
visible in running-turn reads; no node heartbeat/row, running_turns() count or concurrency slot is added for it.
An id that already has a node row can mix their display. Distinct worker ids are recommended, not enforced.

The authority for renew/release is address plus epoch, not owner or message id. S4 keeps epochs increasing across
delete/recreate. No read-then-write ownership check or signed token: the caller is a trusted executor with write
access, and this API is not an authorization boundary. Completion will check the message id through commit.

## Time and lifecycle

Use this server's NodeConfig lease_ttl and max_lost_claims; callers do not select them. Every grant/renewal returns
the interval requested from the store as ttl. Renewal atomically sets expiry to max(current expiry, store now + ttl)
on every backend, returning that actual expiry even when the old grant wins. A shorter peer TTL never invalidates
the holder's earlier deadline, including while a renewal response is lost. Change Backend::renew's behavior and
documentation, not its signature: memory under its lock, SQLite in its writer transaction, PostgreSQL under its row
lock/update. Both Renewed and CancelRequested return the retained maximum; no read outside the atomic operation.
Use request-start monotonic time + ttl - min(ttl/10, 1s); a worker can keep the maximum of its previously confirmed
deadline and that new candidate. Store instants are for handler time/display, not remote deadline calculation.
Checked monotonic overflow before claim is Invalid; store-time overflow retains StoreError::Invalid.

Lease and renew validate NodeConfig (OpError::Start for its usual problems), but do not initialize hosted actors,
register contracts or start dispatch. A new/unstarted system works with already registered types. Release needs no
config validation. Lease refuses an observed Stopping/Stopped state as StoreError::Unavailable; renew and release
remain usable in every node state. Store failures propagate normally, including Unavailable after store close.

Check stopping before store work, after claim and immediately before handoff. A stop observed at those checks releases
the guarded batch and answers Unavailable. A stop racing after the final check does not recall a handed-off lease.
stop() does not drain remote holders or renew/release calls; the host owns request shutdown. After close, cleanup
attempts may fail. Remote leases do not belong to the local task tracker and are not force-released on node stop.

## Claim handoff and cleanup

Generalize the existing private claim helper to accept owner/types and retain ClaimBatch until handoff; local
dispatch/manual execution explicitly take its claims and keep their existing behavior. The store claim runs in its
existing separate task. If the waiting future disappears, its unsent/unreceived guarded batch releases on arrival.
Keep every unhanded claim guarded across post-processing; cancellation drops the guard and makes one background
release attempt per leftover claim. No detached claim-limit executor or new task tracker is introduced.

Record claim_sent before the store request. Check its conservative deadline when the answer arrives and again after
post-processing before return. A passed deadline returns Leased([]), releases leftover claims and runs no handler.
A late initial answer also starts no claim-limit commit/observer. Cleanup is best effort: a closed/unavailable store
or runtime shutdown can prevent release, so those leases expire and are counted lost if replaced. Do not promise
zero lost claims when cleanup fails. Once the batch returns, the caller owns cleanup; dropping a TurnLease does
nothing. A lost later HTTP response leaves leases to expire. No receipts, acknowledgement or long-poll is added.

## Claim-limit filtering

For each at-limit claim, make one fenced failure-commit attempt with cause ClaimLimit and the existing
`claim limit: <lostClaims> lost claims` reason, ignoring retry policy. Process these concurrently within the bounded
batch, borrowing claims still in its guard. Reuse the local executor's request construction and successful
notification/observer path, but not its retry loop or hosted-type lookup. No handler is loaded. On confirmed commit,
remove the claim from cleanup, wake and report observer start then end. On any other result, keep it for release,
log the failure and keep observers silent; wake conservatively after an uncertain commit. Local execution is unchanged.

Never return an at-limit claim. A failed or uncertain commit followed by release is safe: an applied commit cleared
the lease; otherwise the same pending message stays at the limit and fails on its next claim. No handler ran, so
there is no G6 after-handler release. Cancellation during a commit may race release; epoch fencing prevents both an
old commit and a newer claim's commit applying. No inline commit retries or heartbeat; slow store calls can consume
the batch's TTL, which the final deadline check catches. An empty result can mean all claims were filtered or late.

## Renewal, release and checks

Renew maps the existing result, including cancel_requested while extending the lease. It does not cancel work or
complete a message. Renew/release can succeed after wall-clock expiry while the epoch remains held and current;
they fail/no-op after replacement, release, completion or deletion. Release is the holder's relinquishment, never
an operator force-release: stop work first and do not release after the handler ended. A cancellation request still
needs a cancelled completion when it wins against handler end in the successor; a request observed after handler
end is ignored under the existing arbitration. Releasing now leaves the message pending. Repeated release is Ok.

Test virtual-time public behavior and private fault boundaries, mixed limited/runnable batches, peer renewal with
different TTLs and withheld responses, stop during claim/post-processing, cleanup failure, unknown commit, clock
overflow, and fencing after delete/recreate. Add nonshortening renewal conformance for all backends, including
concurrent long/short renewal and the cancellation flag. Keep local runtime/claim-limit behavior tests. ADR 0037 and
the core guide record these limits. No migration or weaker G1–G6/S1–S10; add references for new guarantee tests.

## Review resolution

The first review found that a shorter peer renewal could expire a lease before a holder learned of it. Atomic
nonshortening renewal resolves that race. A returned ttl describes the requested extension interval, not the whole
remaining lease; the actual store expiry may be later. This requires the backend behavior change described above.
The second review is ready for tasks. Preserve the expired-renew/commit conformance scenario by advancing past the
retained expiry, not just a short renewal interval, and retain handler-end arbitration for cancellation.
