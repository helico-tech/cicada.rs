## ADDED Requirements

### Requirement: A deleted holder frees its place

The pool SHALL watch an ask's target with `watch_existing` from the commit that first records the ask, granted or
queued, and on a `Stopped` for that target SHALL free every place it holds and remove its waiting asks, exactly once,
and SHALL unwatch a target that holds and waits for nothing. An ask whose target is the pool itself SHALL be refused.

#### Scenario: A run deleted after its grant

- **WHEN** a holder's target is deleted while it holds a place, and another ask waits
- **THEN** the place is freed and granted to the waiting ask

#### Scenario: A run deleted before its grant

- **WHEN** a waiting ask's target is deleted before the turn that would grant it a place
- **THEN** it gets no place, its ask leaves the line, and the place stays free

#### Scenario: Two holders of one target

- **WHEN** one target holds two places and is deleted
- **THEN** both places are freed, once each

#### Scenario: The last release

- **WHEN** a target's only holder releases its place
- **THEN** the pool unwatches that target

#### Scenario: A failed message of a holder

- **WHEN** a message to a holder's target fails and the pool gets a `Failed` for it
- **THEN** nothing in the pool changes
