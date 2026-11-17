## ADDED Requirements

### Requirement: The node-view rustdoc describes observability without execution authority

The public documentation of ActorSystem::nodes SHALL describe the node view as observability data, including
node-down alerts, and SHALL state that the table grants no authority to claim, move or fence actors; leases
decide who runs what. It SHALL retain the description of node ordering, status from store time and running turns.

#### Scenario: Reading the node-view API documentation

- **WHEN** a consumer reads the rustdoc of ActorSystem::nodes
- **THEN** it describes observability including node-down alerts instead of saying display only
- **AND** it preserves the boundary between node observations and execution authority
- **AND** it does not claim that calling nodes publishes alerts or starts a monitor

#### Scenario: Comment-only correction

- **WHEN** this correction is implemented
- **THEN** the runtime source change is confined to the ActorSystem::nodes doc comment
- **AND** behavior, signatures, wire formats, contracts and handwritten guides remain unchanged
