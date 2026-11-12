# bulk-message-actions Specification

## Purpose
An operator cancels or retries many messages in one call, chosen by their ids or by a
filter, and each message is treated as the single action would treat it.

## Requirements

### Requirement: Many messages can be cancelled or retried at once

`POST /messages/bulk-cancel` and `POST /messages/bulk-retry` SHALL take a body with exactly one of `ids` (at most 1000) or
a filter (`prefix` and/or `messageType`), else `400`, and an unknown key is `400`. They SHALL answer `200 {affected,
more}` and act on at most 1000 messages per call; `more` is true when matches beyond the cap remain.

#### Scenario: Cancel by filter

- **WHEN** a client posts `{"prefix": "sync/", "messageType": "Run"}` to `/messages/bulk-cancel` and two matches are
  pending
- **THEN** the answer is `{"affected": 2, "more": false}` and both end `cancelled`

#### Scenario: Beyond the cap

- **WHEN** 1003 matching failed messages exist and a client bulk-retries by filter
- **THEN** the answer is `{"affected": 1000, "more": true}`, and a second call retries the remaining three

#### Scenario: Neither ids nor a filter

- **WHEN** a client posts `{}`
- **THEN** the answer is `400`

### Requirement: Each message gets the treatment of the single action

Each message of a bulk call SHALL get the same treatment as the single action, and messages that are not eligible SHALL
be skipped and not counted. A filter on retry walks the `failed` outcomes and skips messages already retried. A call is
not one transaction: a call that fails half way keeps what it did.

#### Scenario: Skips what is not eligible

- **WHEN** a client bulk-retries ids of which one has failed and one is pending
- **THEN** the answer is `{"affected": 1, "more": false}`

#### Scenario: Retrying twice sends nothing twice

- **WHEN** a client bulk-retries by filter and calls again
- **THEN** the second call does not send the messages the first call already retried
