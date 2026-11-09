## ADDED Requirements

### Requirement: Finished messages are listed by status

`GET /outcomes` SHALL list the retained, visible outcomes of finished messages, ordered by `(finishedAt, id)`, paged by
`cursor` and `limit`. It SHALL take `status` (`done`, `failed`, `cancelled` or `purged`; absent for all), `prefix` (a
prefix of the actor address) and `since` (only outcomes that finished at or after it). A status outside the four SHALL
be refused with `400`. An outcome hidden by its message's retention policy, or past retention, SHALL NOT be listed.

#### Scenario: One status

- **WHEN** an actor has a done, a failed and a cancelled message and a client asks for `status=failed`
- **THEN** only the failed message is listed, with its envelope and error

#### Scenario: Every status

- **WHEN** a client asks for outcomes without a status
- **THEN** the done, failed, cancelled and purged messages are all listed in `(finishedAt, id)` order

#### Scenario: An unknown status

- **WHEN** a client asks for `status=pending`
- **THEN** the answer is `400` with the problem type `bad-request`

#### Scenario: A hidden record

- **WHEN** a message was sent with a retention policy that removes its done outcome
- **THEN** the message is listed neither with `status=done` nor without a status

### Requirement: Failures are an outcomes query

The failure watcher SHALL read the failed outcomes, and `GET /failures` SHALL no longer exist.

#### Scenario: The watcher forwards a failure

- **WHEN** a message fails
- **THEN** the failure watcher reads it through the outcomes of status `failed` and forwards it to the dead letters
