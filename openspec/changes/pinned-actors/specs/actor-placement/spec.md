## ADDED Requirements

### Requirement: An actor may have an immutable optional claim-owner pin

Each actor SHALL have an optional pinned_to NodeId independent of its address, parent and pause state. It SHALL be
set only at creation and SHALL NOT require a nodes row. A pin SHALL be nonblank, at most 200 characters and NUL-free.
Existing create calls SHALL default to no pin; options variants SHALL support typed/raw, independent/child creates,
external clients and remote completion. Children SHALL NOT inherit pins implicitly.

#### Scenario: Pin a future executor

- **WHEN** an actor is created pinned to an id absent from the nodes table
- **THEN** creation succeeds with the normal registered-type/state checks and the snapshot records the pin
- **AND** a child is unpinned unless its own create explicitly supplies placement

#### Scenario: Preserve idempotent creation

- **WHEN** an existing actor is created again with a different or absent pin
- **THEN** it retains its original pin/state/parent and the existing created-versus-exists result applies
- **AND** a foreign-child create still rejects the whole commit
- **WHEN** an actor is deleted and recreated
- **THEN** its new life may choose a different pin and prior-life fencing still applies

#### Scenario: Reject inconsistent buffered creates

- **WHEN** creates for one address in one turn disagree on child flag or pin, including None versus Some
- **THEN** the conflicting call returns TurnError::Invalid without buffering it
- **AND** a supplied malformed commit returns StoreError::Invalid before checking its epoch
- **AND** a failed turn or rejected commit applies no pin or other buffered effect

#### Scenario: Identify the executing owner

- **WHEN** a Rust handler reads Turn::node_id on a local node or worker
- **THEN** it reads that executor's configured claim-owner id, not an HTTP server id
- **AND** it may explicitly use it in a child's CreateOptions

### Requirement: Placement limits eligibility before claims mutate state

Every backend SHALL admit an actor to a claim only if its pin is absent or equals the claim owner, in addition to
the existing type/due/pause/lease conditions. Selection SHALL apply placement before candidate limits and mutations.
S8 progress SHALL require an eligible executor continuing to poll the type; pins SHALL NOT imply machine authentication.

#### Scenario: Wrong owner cannot consume candidates

- **WHEN** earlier due actors have foreign pins and a later eligible actor exists with batch limit one
- **THEN** the eligible actor can be claimed without foreign actors consuming the batch
- **AND** foreign actors' epoch, attempts, lost claims and outcomes remain unchanged

#### Scenario: Target is unavailable

- **WHEN** no executor with the pin polls the actor's type, or only wrong owners poll
- **THEN** messages remain pending indefinitely, without pin-triggered failures, retries or reassignment
- **AND** paused actors stay excluded even for the matching owner
- **AND** when an eligible owner returns, expired-lease recovery and claim limits apply normally

#### Scenario: Local and remote placement share semantics

- **WHEN** a remote caller leases under owner B through a server A
- **THEN** it can claim unpinned actors and actors pinned to B, but not actors pinned only to A
- **AND** B need not have a node row, and all grants retain normal epoch/lease fencing
- **AND** duplicate configured B identities do not provide exclusive physical-machine placement

### Requirement: Placement is visible and uses compatible optional request fields

CreateActorRequest and remote ActorCreateView SHALL accept optional pinnedTo, defaulting missing/null to None and
omitting None when serialized. Actor snapshots and state listings SHALL expose pinnedTo as string or null, accepting
missing fields from older responses. Existing unknown-field and state/schema validation SHALL remain intact.

#### Scenario: Wire and client compatibility

- **WHEN** old unpinned requests or fixed portable lease fixtures are serialized/decoded
- **THEN** their existing request/effect bytes remain unchanged and their creates remain unpinned
- **WHEN** a new client or returned handler supplies an invalid pin
- **THEN** core create/completion returns OpError::Invalid, turn buffering TurnError::Invalid, direct store calls
  StoreError::Invalid, and wire-effect conversion RequestError before mutation; HTTP maps invalid pins to
  400 bad-request, and CLI invalid flag values are usage errors
- **AND** HTTP callers GET the actor to verify placement because create returns only address and created
- **AND** OpenAPI describes placement and updated typed clients carry it to direct and remote create paths

#### Scenario: Operator diagnosis

- **WHEN** an operator uses create actor --pinned-to ID and then inspects the actor or a wide state listing
- **THEN** local and HTTP modes show the stored placement, including JSON output and actor-detail TUI views
- **AND** existing paginated state listings and mailbox listings support finding pins and inspecting pending work
- **AND** no pin metadata is duplicated per message, no node-detail expansion is added, and normal controls remain usable

### Requirement: The migration preserves data and requires eligible claim code everywhere

Both SQL backends SHALL append migration 8 adding nullable pinned_to with old rows unpinned and a partial
(pinned_to, type, next_due_at) index for nonpaused rows with next_due_at, retaining actors_due. Memory SHALL share the
same semantics. Deployment SHALL stop/drain old direct-store runtimes before migration/restart and upgrade all HTTP
servers before pinned requests, because older already-open claimers cannot enforce placement.

#### Scenario: Upgrade an existing store

- **WHEN** a version-7 store is migrated
- **THEN** state, parent, epochs and mailbox data are preserved and every existing actor is unpinned
- **AND** previous migrations and old portable case bytes are unchanged
- **AND** new lease fixtures cover a valid pinned create, invalid-pin conversion failures and
  TurnEffectsView.creates(pinned), with no regeneration of historical expectations
- **AND** upgrading guidance does not describe mixed old/new direct-store operation as safe

#### Scenario: Changed public structs and guarantees

- **WHEN** the change is prepared for landing
- **THEN** the handoff names S8's eligible-executor qualification, stable unique ids, wire additions, migration steps
  and downstream Rust struct-literal changes
- **AND** exact docs-owner fixes for broken examples/routes/guarantee rows are integrated before landing
