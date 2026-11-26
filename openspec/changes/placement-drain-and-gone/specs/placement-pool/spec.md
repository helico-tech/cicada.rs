## ADDED Requirements

### Requirement: A node can be drained

`Drain` SHALL stop new places on a known node while it keeps its holders and its offers, and `Undrain` SHALL allow them
again and serve the line; an offer SHALL not clear the drain.

#### Scenario: Draining a node with work

- **WHEN** a node with two holders is drained and work asks for a place only it could take
- **THEN** the ask waits, the two holders keep their places, and after `Undrain` the ask is granted

### Requirement: A silent node can be declared gone

`Gone` SHALL be refused for a live or unknown node, and for a silent node SHALL remove it, free its places, and send
each of its holders one answer of kind `gone` with its grant, each in a turn of its own.

#### Scenario: A machine that will not come back

- **WHEN** a node whose status is `silent`, with two holders, is declared gone
- **THEN** both holders get one `gone` answer, the node is removed, and a later release of those grants frees nothing

#### Scenario: Gone for a live node

- **WHEN** an operator sends `Gone` for a node whose status is `live`
- **THEN** it is refused and nothing changes

#### Scenario: An unknown node

- **WHEN** an operator sends `Drain`, `Undrain` or `Gone` for a node the pool does not know
- **THEN** each is refused for good and nothing changes

#### Scenario: A TellGone from someone else

- **WHEN** an actor other than the pool sends a `TellGone`
- **THEN** it is refused for good and nothing changes

#### Scenario: One holder's target is gone

- **WHEN** a silent node with three holders is declared gone and one holder's target was deleted
- **THEN** the other two holders still get their `gone` answer
