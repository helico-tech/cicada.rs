# Spec Delta

## ADDED Requirements

### Requirement: A ref that writes a unit must also read as one

A ref that serde writes as a unit SHALL be a singleton only when it also declares no field to serde. A ref that
writes a unit and reads a string SHALL have no address, so that a send to it or a create of it fails at the call.

#### Scenario: A ref whose two serde halves disagree

- **WHEN** a ref writes itself as a unit and reads itself from a string, and a turn sends to it
- **THEN** the send fails at the call, and no actor with the key `_` is created or reached
