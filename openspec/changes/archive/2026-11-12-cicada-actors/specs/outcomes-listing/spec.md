# outcomes-listing

## REMOVED Requirements

### Requirement: Failures are an outcomes query

**Reason**: the requirement is about the failure watcher of `cicada-extras`, which read the failed outcomes on a
timer. The watcher is gone: dead letters are told of a failure by a watch.

**Migration**: a host that polled with `FailureWatcher` sets a watch for its dead-letters actor. Reading failures
by hand is still `GET /outcomes?status=failed`.

## ADDED Requirements

### Requirement: There is no route for failures

`GET /failures` SHALL NOT exist. Failed messages SHALL be listed as the outcomes of status `failed`.

#### Scenario: The failed outcomes

- **WHEN** a client asks for `GET /outcomes?status=failed` after a message failed
- **THEN** the message is in the list

#### Scenario: The old route

- **WHEN** a client asks for `GET /failures`
- **THEN** the answer is `404`
