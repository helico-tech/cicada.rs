## ADDED Requirements

### Requirement: Stored outputs are deleted after a retention from the removal

When the actor has an `OutputStore` and `ContainerSettings::output_retention` is set, the step that removes the
container SHALL save the due time in the state and arm a poll for it, and that poll SHALL call `OutputStore::remove`
for the container and clear the due time. A retryable store error SHALL be retried with the backoff without limit,
also after a restart; the saved due time SHALL decide even when the settings change later.

#### Scenario: A week after removal

- **WHEN** the retention is 7 days and a container with stored outputs is removed
- **THEN** its outputs are still there after 6 days and gone after 7

#### Scenario: No retention

- **WHEN** `output_retention` is not set
- **THEN** the outputs of a removed container stay

#### Scenario: A store that is down at the due time

- **WHEN** the store fails with `Unavailable` at the due time for three tries and then answers
- **THEN** the outputs are deleted at the fourth try and the due time is cleared

#### Scenario: A store that refuses the name

- **WHEN** the store answers `Invalid` at the due time
- **THEN** the due time is cleared with a warning and nothing is retried

## MODIFIED Requirements

### Requirement: A lost poll comes back

`Check`, and a goal asked again, SHALL replace the armed poll with a fresh one due when the old one was due, or at
once when that time has passed. With no work left they SHALL change nothing; a removed container whose outputs wait for
deletion has work left.

#### Scenario: A purged poll

- **WHEN** the armed poll of a running container is purged and the actor gets `Check`
- **THEN** a fresh poll is armed and the container is followed to its end

#### Scenario: A lost retention poll

- **WHEN** the armed poll of a removed container whose outputs wait for deletion is purged and the actor gets `Check`
- **THEN** a fresh poll is armed and the outputs are deleted at the saved time

