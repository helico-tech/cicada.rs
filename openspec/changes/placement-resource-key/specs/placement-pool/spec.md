## ADDED Requirements

### Requirement: One live node holds a resource

An offer MAY name a resource key; the pool SHALL ignore an offer whose key a different live node holds, mark the
offering node in conflict with that resource, and send one `conflict` alert per mark, and SHALL let the next offer with
the key take it once its holder is silent or gone.

#### Scenario: Two nodes offering one Docker daemon

- **WHEN** two nodes on one machine offer the same daemon id as their resource key
- **THEN** the first keeps its places, the second's offers are ignored, and one `conflict` alert goes out

#### Scenario: The holder stops

- **WHEN** the first node goes silent and the second keeps offering the key
- **THEN** the second node's offer takes the key and its places count
