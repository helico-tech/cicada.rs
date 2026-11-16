## MODIFIED Requirements

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
