# undecodable-messages

## MODIFIED Requirements

### Requirement: A payload that is refused at the door says why once

A node SHALL refuse a send over HTTP whose payload does not match the schema of its message type, and a node that
hosts the type of the target SHALL also refuse one that does not decode. Both SHALL answer `422`. On a node that hosts
the type the detail SHALL hold the decoder's reason once.

#### Scenario: A send with a payload that does not fit

- **WHEN** a client sends a message of the type `Add` whose payload is an object with a field of the wrong kind, to
  a node that hosts the target's type
- **THEN** the answer is `422`, and its detail holds the decoder's reason, which names the field, exactly once
