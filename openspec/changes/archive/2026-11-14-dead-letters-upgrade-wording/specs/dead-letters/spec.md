## MODIFIED Requirements

### Requirement: A breaking dead-letter upgrade preserves exported letters explicitly

The changed state contract SHALL have a new hash. The guide SHALL provide a tested backup/restore procedure for
independent, childless actors with no incoming watches, covering all names, states, pause flags and outgoing watches.
It SHALL require quiescence, verified backups and exports, stopping old hosts, and checks before deletion that each
restore state decodes as the new typed state, is at most 64 levels deep and contains no NUL in strings or keys, and
that no address repeats in the export. These example checks SHALL NOT be described as JSON Schema validation.
It SHALL require verification before resuming and disclose unsupported topology, new instance identity and notification gaps.

#### Scenario: Maintenance safeguards

- **WHEN** an operator prepares the upgrade
- **THEN** the guide requires quiescing watched work and senders, draining turns and notices, and stopping every old
  local/remote host and bootstrapper before replacing the registration
- **AND** it explains that hash checks do not evict already-running local nodes and requires checking for parents,
  children and incoming watches before deleting any instance
- **AND** those topologies require a host-specific migration because deletion removes descendants and emits Stopped

#### Scenario: Incomplete export or unquiesced work

- **WHEN** an export, backup or restore validation is incomplete
- **THEN** the documented procedure stops before destructive operations
- **WHEN** work cannot be quiesced
- **THEN** the guide discloses the notification gap, outcome retention limits and that business retries may repeat
  external effects rather than reconstruct historical letters

#### Scenario: Old raw letters survive restore

- **WHEN** an operator follows the maintenance recipe for multiple eligible actors holding raw letters and outgoing watches
- **THEN** restored letters keep their message ids, payload meaning, metadata and prior `redrivenAs` values
- **AND** pause settings and watches are restored before work resumes, and a letter already redriven is not sent again
- **AND** the guide states that versions, epochs and creation times belong to new instances and cached leases cannot be reused

#### Scenario: Readable legacy deep export

- **WHEN** a readable export contains a raw payload too deep to restore inside a letter
- **THEN** the documented exact-parsing conversion can move it to `payloadJson` without discarding the letter
- **AND** the check before deletion accepts the converted state, redrive still refuses payload depth greater than 64, and Discard works

#### Scenario: Raw legacy state is not automatically repaired

- **WHEN** a legacy raw letter of depth 70 remains in a readable store state and a turn tries to write that state
- **THEN** the ordinary state-depth check still refuses it; this change adds no automatic live repair
- **AND** the guide directs the operator to export/convert/restore or deliberate Discard, retaining a backup for unreadable data

#### Scenario: Old hosts and readers

- **WHEN** old code starts against the new registration, or an old remote worker tries to lease the changed type
- **THEN** the existing contract mismatch rules refuse it
- **AND** an old typed state reader cannot decode a `payloadJson`-only letter, so readers must upgrade too
