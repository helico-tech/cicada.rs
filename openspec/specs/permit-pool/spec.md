# permit-pool Specification

## Purpose
The ready-made permit pool `cicada.permits` caps how many requests hold a permit at once, queues the rest in order,
and fences each grant with a number that only goes up, so a stale release never frees a newer grant (ADR 0058).

## Requirements

### Requirement: A permit pool under a fixed type name

`cicada-actors` SHALL provide a permit pool, `cicada.permits/<pool>`, hosted by `PermitsActor` without settings. Its
state SHALL hold the capacity and the queue bound, each 0 through 1,000 and given at the create, the holders with
their grant numbers, the waiting requests in FIFO order, and the next grant number. Its contract SHALL match
`docs/reference/contracts/cicada.permits.json`.

#### Scenario: A pool from its create

- **WHEN** a pool is created with capacity 2 and queue bound 10
- **THEN** its state holds capacity 2, queue bound 10, no holders, no waiting requests and next grant number 1

#### Scenario: A state out of bounds

- **WHEN** a pool state with capacity 1,001, a request ID held twice, more waiting requests than its queue bound, or
  a next grant number of 0 is decoded
- **THEN** the decoder refuses it with the reason

### Requirement: Acquire grants one permit in FIFO order with a fenced number

`Acquire` SHALL ask one permit for a request ID and name the target and message type of its notices. With room it
SHALL grant at once; otherwise the request SHALL wait in FIFO order. After every message the pool SHALL make the head
of the queue a holder while fewer hold than the capacity, in the commit that made room. Each grant SHALL take the next
grant number, which only goes up, and its `granted` notice SHALL carry it.

#### Scenario: FIFO order

- **WHEN** a pool of capacity 1 holds a grant and requests `a`, `b` and `c` wait in that order, and the holder releases
- **THEN** the commit of the release makes `a` a holder with the next grant number, a later turn of the pool sends `a`
  its `granted` notice, and `b` and `c` keep waiting in that order

#### Scenario: Grant numbers only go up

- **WHEN** request `a` is granted and released, and then request `b` is granted
- **THEN** the grant number of `b` is higher than that of `a`

### Requirement: Every Acquire is answered in its commit and a repeated Acquire is idempotent

Each `Acquire` SHALL send its answer, `granted`, `queued` or `refused`, to its target in its own commit. An `Acquire`
with the request ID of a holder SHALL send `granted` again with the same grant number; one with the ID of a waiting
request SHALL send `queued` and keep its place. Both SHALL keep the target of the first `Acquire`. A request ID is
not remembered after its release.

#### Scenario: A repeated acquire of a holder

- **WHEN** a holder of grant 4 sends `Acquire` again with the same request ID
- **THEN** its target gets `granted` with grant 4, and the pool holds one permit for it

#### Scenario: A repeated acquire of a waiter

- **WHEN** waiting request `b`, second in the queue, sends `Acquire` again
- **THEN** its target gets `queued`, and `b` is still second in the queue

#### Scenario: An acquire with a gone target

- **WHEN** an `Acquire` names a target that does not exist
- **THEN** its commit is rejected, and the pool records no grant and no waiting request

### Requirement: A full queue refuses an Acquire

When the pool has no room and the queue holds its bound, an `Acquire` of a new request SHALL send a `refused` notice
to its target and record nothing.

#### Scenario: A full queue

- **WHEN** a pool of capacity 1 and queue bound 1 is full and one request waits, and request `x` asks
- **THEN** the target of `x` gets `refused`, and the state does not change

### Requirement: Bad input is refused for good

A message with a field out of bounds SHALL be refused for good and leave the state unchanged: a capacity above 1,000,
a request ID that is empty, longer than 200 characters or holds a NUL, and a target or message type that the send
refuses.

#### Scenario: A resize above the bound

- **WHEN** a pool of capacity 2 gets `Resize` with capacity 1,001
- **THEN** the message is refused for good and the capacity stays 2

#### Scenario: A bad request ID

- **WHEN** a pool gets `Acquire` with a request ID of 201 characters
- **THEN** the message is refused for good, no notice is sent, and the state does not change

#### Scenario: A message type the target does not take

- **WHEN** a full pool gets `Acquire` whose message type the target's type does not take
- **THEN** the message is refused for good and the request does not join the queue

#### Scenario: A bad request ID in a release

- **WHEN** a pool gets `Release` with an empty request ID
- **THEN** the message is refused for good and the state does not change

### Requirement: Only a Release with the current grant number frees a delivered permit

`Release` SHALL free a permit only when its request ID holds the grant number it names, and SHALL be ignored
otherwise. `Cancel` SHALL take a waiting request out of the queue and SHALL be ignored for a holder. No timeout,
failed `check` or audit SHALL free a permit.

#### Scenario: A stale release

- **WHEN** request `a` holds grant 7, and a `Release` of `a` with grant 3 arrives
- **THEN** the release is ignored and `a` still holds grant 7

#### Scenario: Cancel while waiting

- **WHEN** a waiting request `b` sends `Cancel`
- **THEN** `b` leaves the queue, is never granted, and the requests behind it move up

#### Scenario: Cancel of a holder

- **WHEN** a holder sends `Cancel` for its request
- **THEN** the pool still counts its permit

### Requirement: An undelivered grant comes back

A notice that does not answer an `Acquire` SHALL be sent by a `Notify` the pool sends itself, in a turn of its own
that reads the state when it runs. The pool SHALL watch itself. When its own grant `Notify` fails for a holder whose `granted` notice never
committed, the pool SHALL free that permit and grant the next waiter. A failed `check` SHALL free nothing.

#### Scenario: A deleted waiter

- **WHEN** a waiting request's target is deleted, and a holder releases
- **THEN** the release commits, the deleted waiter's grant notice fails, and the pool frees that permit and makes the
  next waiter a holder

#### Scenario: A failed check

- **WHEN** a holder that got its `granted` notice is deleted, and the pool gets `Audit`
- **THEN** the `check` to that holder fails, and the pool still counts its permit

#### Scenario: A message the pool did not send itself

- **WHEN** a client sends the pool a `Notify` for a waiting request, or a `Failed` that names one of its grant
  notices
- **THEN** the pool sends no notice and frees nothing

### Requirement: Resize changes the capacity and revokes nothing

`Resize` SHALL set the capacity. A growth SHALL make the head of the queue holders in its commit. A shrink SHALL leave
every holder its permit; new grants SHALL wait until fewer hold than the new capacity.

#### Scenario: A resize up

- **WHEN** a full pool of capacity 1 with two waiting requests is resized to 3
- **THEN** the commit of the resize makes both waiting requests holders, and later turns of the pool send each its
  `granted` notice

#### Scenario: A resize down

- **WHEN** a pool of capacity 3 with three holders is resized to 1, and one holder releases
- **THEN** the other two keep their permits until they release, and no waiting request is granted after that release

### Requirement: Audit asks every request to check in

`Audit` SHALL send a `check` notice to every request in the books, holders with their grant number and waiting
requests without one, except a holder whose `granted` notice has not committed: a grant number SHALL reach a holder
only through `granted`. It SHALL free nothing: a delivered permit SHALL come back only by a `Release`.

#### Scenario: An audit frees only on a release

- **WHEN** a pool with two holders gets `Audit`, and only one holder answers its `check` with `Release`
- **THEN** both holders got a `check`, and only the holder that released lost its permit

#### Scenario: A holder not yet told

- **WHEN** a holder's `granted` notice has not committed, and the pool gets `Audit`
- **THEN** that holder gets no `check`; once its `granted` notice committed, the next audit sends it one

#### Scenario: A waiter granted before its check

- **WHEN** an audit checks a waiting request, and a release grants it before its `check` runs
- **THEN** it gets its `granted` notice and no `check`
