## ADDED Requirements

### Requirement: A host leases turns through the public core API

ActorSystem::lease_turns SHALL accept a worker NodeId, a map of TypeNames to contract hashes, and a batch limit.
It SHALL return runnable TurnLeases or a ContractMismatch, without requiring a hosted implementation. A TurnLease
SHALL contain actor, epoch, full claimed message, state, version, claim time, lease expiry and granted TTL.

#### Scenario: A server without handlers

- **WHEN** a caller leases a registered type on an unstarted ActorSystem with no hosted types
- **THEN** a due actor's head message and state are returned with the incremented attempt and current lease epoch
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
