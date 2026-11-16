# turn-leases Specification

## Purpose
A server can lease turns to a worker outside its node: the worker renews or releases each lease by actor and epoch
through any server on the same store, and a renewal never shortens a lease it still holds.

## Requirements

### Requirement: A host leases turns through the public core API

ActorSystem::lease_turns SHALL accept a worker NodeId, a map of TypeNames to contract hashes, and a batch limit.
It SHALL return runnable TurnLeases or a ContractMismatch, without requiring a hosted implementation. Claims SHALL
include only actors unpinned or pinned to the supplied worker id, not the serving node id, before batch limits. A TurnLease
SHALL contain actor, epoch, full claimed message, state, version, claim time, lease expiry and granted TTL.

#### Scenario: A server without handlers

- **WHEN** a caller leases a registered type on an unstarted ActorSystem with no hosted types
- **THEN** an eligible due actor's head message and state are returned with the incremented attempt and current lease epoch
- **AND** no handler runs, no local hosted type or node row is added, and the server's running_turns count is unchanged

#### Scenario: All lease fields

- **WHEN** a turn with headers, correlation, retry/retention policy and prior failure history is leased
- **THEN** its complete Message and state/version match the claimed store record, including failures and lostClaims
- **AND** claimed_at and lease_until are store times and ttl is the server's configured lease duration

#### Scenario: Invalid requests

- **WHEN** the type map is empty or exceeds 100 entries, limit is outside 1..=100, or owner exceeds 200 characters
  or contains NUL
- **THEN** the operation returns OpError::Invalid before any store access or claim

#### Scenario: Contract mismatch

- **WHEN** any selected name is absent or its registered hash differs from the requested string
- **THEN** no turn is claimed and ContractMismatch identifies the first mismatch in sorted type-name order,
  with the registered hash or None and the requested string

#### Scenario: Contract replacement between requests

- **WHEN** an unused type is deleted and registered with a new hash before the next lease request
- **THEN** a request carrying its old hash gets ContractMismatch
- **AND** the check does not promise to pin a contract against replacement racing between the registry read and claim

#### Scenario: Local and remote contenders

- **WHEN** a local node and external lease callers compete for the same actor
- **THEN** ordinary store lease exclusivity permits only one current lease, and paused actors are not leased
- **AND** only unpinned actors or those pinned to the claimant are eligible
- **AND** each selected actor contributes only its due mailbox head in existing delivery order

#### Scenario: Worker visibility

- **WHEN** an external lease is held under a worker id with no node row
- **THEN** running_turn and list_running_turns show that id as owner but nodes does not invent a node row for it
- **AND** reusing an existing node id is permitted and can combine their running-turn display

### Requirement: Leases use server policy and conservative deadlines

Lease and renewal SHALL use the serving system's configured TTL and return that duration. Callers SHALL use their
monotonic request-start time plus the returned TTL minus min(TTL/10, one second) as the conservative deadline.
Renewal SHALL preserve any later existing expiry; callers may retain a later previously confirmed deadline. The core
SHALL discard unhanded leases whose analogous server deadline passed, both after claim and before return.

#### Scenario: A late claim answer

- **WHEN** the claim answer arrives at or after its conservative server deadline
- **THEN** the result is Leased([]), no handler or claim-limit observer starts, and all claims receive release attempts

#### Scenario: Post-processing consumes the TTL

- **WHEN** processing claim-limit failures delays a batch beyond its conservative server deadline
- **THEN** its runnable claims receive release attempts and are not returned, even though the initial claim was timely
- **AND** confirmed claim-limit outcomes already committed are preserved

#### Scenario: Different servers grant different TTLs

- **WHEN** a lease granted by server A is renewed through server B with a different configured TTL
- **THEN** renewal returns B's TTL and the actual expiry, atomically the later of the old expiry and store now plus B's TTL
- **AND** a shorter interval never invalidates the caller's previous confirmed deadline

#### Scenario: A shorter renewal response is withheld

- **WHEN** A grants 60 seconds at t=0, B applies a 5-second renewal at t=10 and B's response is delayed or lost
- **THEN** another worker cannot claim the actor at t=15 or before the retained t=60 expiry
- **AND** the holder's original conservative deadline near t=59 remains safe without knowing B's response

#### Scenario: Deadline overflow

- **WHEN** the new lease operation cannot represent its configured duration on the local monotonic clock
- **THEN** it returns Invalid without claiming rather than panicking
- **AND** a store-clock overflow otherwise retains the ordinary StoreError::Invalid result

### Requirement: Unhanded leases receive cancellation cleanup

The core SHALL retain a cleanup guard until a lease batch is handed to the caller. A cancelled future SHALL cause
one release attempt for each leftover claim, including results arriving after the caller disappeared. Cleanup SHALL
be best effort; after handoff the caller SHALL own lease cleanup, and dropping a TurnLease SHALL have no effect.

#### Scenario: Cancellation during a claim

- **WHEN** a caller drops lease_turns while the detached store claim is in progress
- **THEN** claims that later arrive receive release attempts without starting handlers
- **AND** after successful cleanup a next claim preserves lostClaims and increments attempt normally

#### Scenario: Cancellation during filtering

- **WHEN** a caller drops lease_turns during an at-limit failure commit in a mixed batch
- **THEN** all unhanded claims receive release attempts and old-epoch fencing protects any subsequent claim
- **AND** a failure commit that already applied is not undone or duplicated by cleanup

#### Scenario: Cleanup cannot reach the store

- **WHEN** release fails or runtime shutdown prevents the background cleanup from running
- **THEN** held leases may expire and a replacing claim counts them lost under the existing rule
- **AND** the API does not claim that cancellation always avoids lost claims

#### Scenario: Loss after handoff

- **WHEN** a batch was returned but the caller loses it or a later HTTP response carrying it is lost
- **THEN** the core does not recover the batch from a receipt, and unreleased leases expire and count lost if replaced

### Requirement: Renew and release are fenced by actor and epoch

ActorSystem::renew_turn SHALL extend a currently held epoch and return expiry, granted TTL and the cancellation
flag, or LeaseLost. release_turn SHALL relinquish a held matching epoch and otherwise do nothing. Neither operation
SHALL check owner identity. The holder SHALL stop work before release and not release a turn whose handler ended.

#### Scenario: Renewal through a peer

- **WHEN** a caller renews or releases a lease through another ActorSystem on the same database
- **THEN** the operation works without state in the system that granted the lease

#### Scenario: Operator cancellation

- **WHEN** cancel_turn records a request for the held epoch and the worker renews
- **THEN** renewal extends the lease and reports cancel_requested
- **AND** it does not itself end the message; release would leave it pending
- **AND** a later completion honors cancellation only when it wins against handler end under the existing arbitration;
  a cancellation observed after the handler ended is ignored

#### Scenario: Stale or released epochs

- **WHEN** an epoch was replaced, released or completed, or its actor was deleted and recreated
- **THEN** renewal returns LeaseLost and release is a successful no-op that cannot affect the new lease

#### Scenario: Expired but not replaced

- **WHEN** lease_until has passed but the same epoch remains held with no replacement
- **THEN** renew can still succeed and release can clear it, as in the existing backends
- **AND** this does not authorize workers to run past their conservative local deadline

#### Scenario: Idempotent release

- **WHEN** release is called twice for the same epoch
- **THEN** both calls succeed and a subsequent claim retains lostClaims while increasing attempt

### Requirement: Server lifecycle governs admission, not remote holders

Lease and renew SHALL validate NodeConfig without initializing local handlers or registering their contracts.
Lease SHALL refuse an observed Stopping/Stopped state before store work, after claim and before handoff. Renew and
release SHALL work in any node state while the store remains available. Remote leases SHALL not join local task counts.

#### Scenario: Stopped server

- **WHEN** lease_turns observes the server stopping or stopped
- **THEN** it answers StoreError::Unavailable and attempts release of any unhanded claims
- **AND** a stop racing after the final admission check does not recall a returned lease

#### Scenario: Existing holders during stop

- **WHEN** the serving ActorSystem stops while an external worker holds a lease
- **THEN** stop does not wait for or release it and renew/release remain available through that system or a peer

#### Scenario: Invalid server configuration

- **WHEN** lease or renewal finds invalid NodeConfig values
- **THEN** it returns OpError::Start without registering or validating hosted actor definitions
- **AND** release remains available without configuration validation

#### Scenario: Closed store

- **WHEN** otherwise valid lease, renew or release operations reach a closed store
- **THEN** they return the ordinary unavailable store error

### Requirement: A trusted executor completes a leased turn with a report

ActorSystem::complete_turn SHALL accept actor, epoch, message id and a Returned, Failed or Cancelled report. It SHALL
derive the outcome and commit state and effects atomically through the existing fenced commit. Reports SHALL not
choose the input message's retry policy, arbitrary outcomes, sender identities or notification event ids.

#### Scenario: A returned handler

- **WHEN** a valid Returned report sets state and buffers sends, creates, cancellations, watches and controls
- **THEN** they commit together in the existing effect order, and Completed returns the applied outcome, state version,
  actual sent envelopes and cancelled ids
- **AND** no handler is executed by complete_turn

#### Scenario: No change, new state and stop

- **WHEN** a Returned report selects Unchanged, Set(value) or Stop
- **THEN** state/version remain unchanged, are updated, or the actor is removed according to existing turn semantics
- **AND** Stop returns the version before removal, while Set(null) is an invalid new value rather than Unchanged

#### Scenario: Derived send identity

- **WHEN** a report sends messages with caller-chosen ids
- **THEN** the server supplies the committing actor as from and the claimed message id as caused_by
- **AND** each send keeps its supplied id across an identical report retry

#### Scenario: Failed and cancelled reports

- **WHEN** a caller reports Failed or Cancelled
- **THEN** its report carries no user state or effects, and completion applies only the derived outcome and normal
  store-generated notifications

#### Scenario: Cancel wins before handler end

- **WHEN** the worker's existing handler-phase arbitration records cancellation before handler end
- **THEN** it may report Cancelled and the server commits cancelled without user effects
- **AND** a cancellation observed after handler end is ignored by the worker; the server does not arbitrate that race

### Requirement: Completion derives failure policy from the stored message

Only Handler, Node and Timeout failures SHALL be eligible for retry. Nonretryable reports MAY also use Refused,
Message, State and Commit. ClaimLimit or any other invalid combination SHALL fail before store access. For an eligible
failure the server SHALL use the stored message's retry policy and failure count, preserving the local executor policy.

#### Scenario: Failure of a currently running message

- **WHEN** a retryable report names a Running message of the actor and its policy permits another failure
- **THEN** completion reschedules the same message using failures+1 and the policy's capped backoff, with no user effects

#### Scenario: An expired but still held message

- **WHEN** status shows Pending because the lease expired, but this epoch is still held and current
- **THEN** completion may still use that message's policy and commit; a Running status is not required for validity

#### Scenario: No retry remains

- **WHEN** the report is nonretryable, the message has no policy, or its permitted attempts are exhausted
- **THEN** the derived outcome is failed, with the supplied cause and sanitized reason

#### Scenario: Bad failure combination

- **WHEN** the caller selects ClaimLimit or requests retry for Refused, Message, State or Commit
- **THEN** completion returns OpError::Invalid without store access or a lease mutation

#### Scenario: Status read fails or races replacement

- **WHEN** reading the retry snapshot fails
- **THEN** completion returns the store error without a commit
- **WHEN** the message changes through another commit or its lease is replaced after the snapshot
- **THEN** the old epoch cannot apply the derived retry; the commit fence remains authoritative

#### Scenario: Reason normalization

- **WHEN** a failure reason includes NUL or is longer than the reschedule error limit
- **THEN** NUL is replaced by U+FFFD and rescheduled lastError is truncated as in local execution
- **AND** terminal failure reasons retain the local policy without a new core length cap

### Requirement: Remote completion validates storage limits without changing handler checks

The server SHALL apply existing send-envelope, delivery, create-state and own-state storage checks to reports.
It SHALL not add JSON Schema or hosted-target decoding at completion. The executor SHALL retain call-time registry,
type, watch and buffering checks; a report SHALL describe its finished buffer rather than a sequence of Turn calls.

#### Scenario: An invalid send or creation value

- **WHEN** a send violates an existing envelope/delivery/value limit or a create supplies a null, NUL-containing or
  over-deep state
- **THEN** the server derives a no-effects nonretryable Commit failure using the existing refusal text
- **AND** that derived failure is still subject to the epoch and message-id checks at commit

#### Scenario: An invalid new own state

- **WHEN** Set contains null, NUL or a value beyond the state depth limit
- **THEN** completion derives a nonretryable Handler failure with the existing State cannot be stored prefix
- **AND** all buffered effects are discarded

#### Scenario: A server hosts a target implementation

- **WHEN** the same report is completed through servers with different hosted actor sets
- **THEN** those local sets do not change the completion's validation or result
- **AND** worker-side Serde, raw lookup and hosted-target checks remain that worker's responsibility

#### Scenario: Raw effects are not an operation log

- **WHEN** a manually constructed report includes a send and a cancellation of that send's id
- **THEN** the backend's existing cancel-before-send order applies
- **AND** the Rust executor's conversion instead preserves Turn's existing rule that cancelling a buffered send
  removes it before a report is produced

#### Scenario: Identity and structural refusal

- **WHEN** a report names the wrong claimed message, cancels its own input, conflicts on child/control effects or
  violates a target/id/ancestor rule
- **THEN** the existing backend checks refuse atomic application
- **AND** no promise is made that epoch fencing precedes every structural request check

### Requirement: A completion call makes at most one commit attempt

Completion SHALL return Completed on confirmed commit and LeaseLost on the store's lost-lease result. A known
rejection SHALL return Rejected with the exact nonretryable failure report to submit next. An uncertain store error
SHALL not cause a different commit, release or internal retry. The caller SHALL retain its current report across retries.

#### Scenario: A known rejection

- **WHEN** the store rejects effects or refuses their data and no commit applied
- **THEN** completion returns Rejected with the shared executor fallback cause and reason, without committing it
- **AND** the caller replaces its current report with Failed(that failure), permanently discarding the original effects

#### Scenario: A refused reschedule

- **WHEN** a derived retry's delivery is outside the store's time range
- **THEN** Rejected carries the original handler cause and reason with the refusal appended, marked nonretryable
- **AND** submitting that failed report can end the message without effects

#### Scenario: A refused terminal failure

- **WHEN** a no-effects terminal failed decision is itself refused, including one with a wrong message id
- **THEN** completion returns OpError::Invalid rather than another fallback report
- **AND** no outcome or effects are applied and the caller must abandon without release

#### Scenario: Failure before the primary commit applies

- **WHEN** a store error prevents the primary commit from applying
- **THEN** completion returns the error without release, fallback or handler execution
- **AND** the caller may retry its identical report through any server within its conservative deadline

#### Scenario: Failure after the primary commit applies

- **WHEN** the primary commit applies but its acknowledgement is lost
- **THEN** a retry of the same report cannot duplicate state, sends, outcomes or watcher notifications
- **AND** the store's LeaseLost response is returned without claiming it proves the first attempt succeeded

#### Scenario: Cancellation of the completion request

- **WHEN** the caller drops complete_turn while its backend commit is in flight and retries the identical current
  report through another system on the same store
- **THEN** no release occurs and at most one outcome and effect set apply
- **AND** the caller treats the dropped request as uncertain, including a SQLite commit that settles after cancellation

#### Scenario: A lost rejection response

- **WHEN** a rejection response is lost before the caller learns of it
- **THEN** no failure fallback was attempted by the server
- **AND** retrying the original report may re-evaluate its effects against the current store, including a target
  created since the unacknowledged rejection

#### Scenario: An acknowledged rejection followed by an uncertain failed commit

- **WHEN** the caller receives Rejected, switches to the provided Failed report, and that failed commit's response is lost
- **THEN** every later retry uses only that Failed report, even if the original effects would now be valid
- **AND** the original state/effects can never reappear through the compliant caller's retry path

#### Scenario: Concurrent reports

- **WHEN** identical or differing reports race for one actor epoch
- **THEN** at most one commit applies and clears the lease, and later attempts cannot apply their effects

### Requirement: Completion preserves lifecycle and acknowledgement uncertainty

complete_turn SHALL work without server initialization, through any peer and after node stop while the store is
available. It SHALL never release the lease or notify server observers. Only an acknowledged Completed SHALL let the
worker finish its observers. The caller SHALL not rerun a finished handler or release it after any uncertain completion.

#### Scenario: Peer and stopped servers

- **WHEN** a lease from one system is completed through an unstarted or stopped peer with different configuration
- **THEN** the same report policy and atomic checks apply without process-local lease state

#### Scenario: Retention hides the result

- **WHEN** a completion applied but retention hides its outcome before a lost reply is retried
- **THEN** the epoch still prevents duplicate effects
- **AND** a message-status read is not promised to resolve the caller's uncertainty

#### Scenario: Server observers and wakes

- **WHEN** completion commits or may have committed without acknowledgement
- **THEN** it wakes local dispatch as appropriate but does not start/end server observers for that remote turn
- **AND** the worker ends its own observer only on Completed, otherwise eventually dropping it on abandonment or lease loss

#### Scenario: Closed store

- **WHEN** completion reaches a closed store
- **THEN** it returns the ordinary unavailable error and performs no release or automatic fallback
