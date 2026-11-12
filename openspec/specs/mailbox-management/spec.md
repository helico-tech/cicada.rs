# mailbox-management Specification

## Purpose
An operator sees which message of a mailbox is running, pauses and resumes an actor so that
its messages wait, and purges its mailbox.

## Requirements

### Requirement: The mailbox listing shows which message is running

`GET /actors/{type}/{key}/mailbox?cursor=&limit=` SHALL replace `GET /actors/{type}/{key}/messages` and list the pending
messages of the actor in delivery order, each with `state` `running` or `pending`. `POST /actors/{type}/{key}/messages`
SHALL stay as the send.

#### Scenario: A turn is running

- **WHEN** a turn handles the head message and a client gets the mailbox
- **THEN** that message has `state` `running` and the others `pending`

### Requirement: An actor can be paused and resumed

`POST .../mailbox/pause` and `POST .../mailbox/resume` SHALL answer `204`, or `404` for an unknown actor. A paused actor
SHALL not be claimed; its messages still queue up and a running turn finishes. On resume the dispatch loop is woken. S8
applies only to actors that are not paused.

#### Scenario: Paused

- **WHEN** an actor is paused and a message is sent to it
- **THEN** the message queues up and no turn starts until the actor is resumed

#### Scenario: A running turn at pause

- **WHEN** an actor is paused while a turn runs
- **THEN** the turn finishes and commits as usual

### Requirement: A mailbox can be purged

`POST .../mailbox/purge` SHALL atomically give outcome `purged` to every pending message except one being handled under a
valid lease, and answer `200 {purged}`, or `404` for an unknown actor. The actor stays. Purging a message whose lease
expired SHALL void that lease.

#### Scenario: A message is running

- **WHEN** a turn handles one message and two are queued, and a client purges the mailbox
- **THEN** the answer is `{"purged": 2}`, the two end `purged` and the running one finishes normally
