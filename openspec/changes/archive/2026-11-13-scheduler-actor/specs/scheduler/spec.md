## ADDED Requirements

### Requirement: A scheduler keeps a schedule, a target and a message

A scheduler SHALL have the type name `cicada.scheduler`, with one instance for each schedule. Its state SHALL hold a
`Schedule`, a target address, a stored message and an optional retry policy for the delivered message, set at its
create. A scheduler is started when `armedTick` is set in its state. No slot SHALL deliver until it is started.

#### Scenario: Not started

- **WHEN** a scheduler is created, gets no `Start`, and the clock passes a slot
- **THEN** nothing is delivered

### Requirement: A scheduler sends its message to its target on each slot

`Start` SHALL arm a tick at the next slot of the schedule, on the store clock. Each tick SHALL arm the next one and
send the scheduler a `Run` with its slot. A `Run` SHALL send the stored message, with its type name and payload, to
the target, with the scheduler as its sender and the scheduler's retry policy for it. A delivery that reaches the
target's mailbox SHALL add 1 to `runs` and set `lastRun` to the store's time of the `Run`.

#### Scenario: A daily schedule

- **WHEN** a scheduler with the schedule `daily(2, 0)` is started at 10:00 and the clock reaches 02:00 the next day
- **THEN** the target has the stored message in its mailbox once, sent by the scheduler, `runs` is 1, and `lastRun`
  is 02:00 of that day

#### Scenario: Each slot delivers

- **WHEN** a scheduler with the schedule `every(60)` is started and the clock moves three minutes on
- **THEN** the target has the stored message three times, one for each slot

#### Scenario: A second start

- **WHEN** a started scheduler gets `Start` again and the clock reaches the next slot
- **THEN** the target has the stored message once for that slot

#### Scenario: The target's run is retried

- **WHEN** a scheduler with a retry policy of two attempts delivers, and the target's handler fails on the first run
- **THEN** the target handles the same message again after the backoff

### Requirement: The target learns its slot

A delivery of a `Run` with a slot SHALL carry the header `cicada-slot`, with the slot in UTC to the second, in the form
`2026-11-13T02:00:00Z`. A delivery of a `Run` without a slot SHALL carry no such header.

#### Scenario: A late tick

- **WHEN** a scheduler with the schedule `daily(2, 0)` is paused over 02:00 and resumed at 03:00
- **THEN** the target's message has the header `cicada-slot` with 02:00 of that day, in the form above

#### Scenario: A run sent by an operator

- **WHEN** a client sends a scheduler `Run` with no slot
- **THEN** the target's message has no header `cicada-slot`

### Requirement: A scheduler skips the slots it missed

A tick SHALL arm the next one at the next slot after the later of the store clock's time and its own slot. A late tick
SHALL deliver once and the slots it missed SHALL be skipped; a tick that runs before its slot SHALL NOT arm that slot
again. A paused scheduler SHALL deliver nothing; at its resume a tick that came due SHALL deliver once. A deleted
scheduler SHALL deliver nothing more.

#### Scenario: A pause over three slots

- **WHEN** a scheduler with the schedule `every(60)` is paused for three minutes and then resumed
- **THEN** the target gets the stored message once at the resume, and then once at each following slot

#### Scenario: An early tick

- **WHEN** an operator makes the armed tick of a scheduler with the schedule `every(60)` due now, 20 seconds before
  its slot
- **THEN** the target gets the stored message once for that slot, and the next tick is armed at the slot after it

#### Scenario: A delete

- **WHEN** a started scheduler is deleted
- **THEN** its armed tick is purged and the target gets nothing more

### Requirement: An operator runs a schedule now

A `Run` that a client sends SHALL deliver at once, also on a scheduler that was never started, and SHALL keep the
armed tick.

#### Scenario: Run now

- **WHEN** a started scheduler gets `Run` from a client between two slots
- **THEN** the target has the stored message at once, and again at the next slot

#### Scenario: Run before the start

- **WHEN** a scheduler that was never started gets `Run` from a client
- **THEN** the target has the stored message once, and no tick is armed

### Requirement: An operator edits a schedule

`Update` SHALL replace the schedule, the target, the message and the retry policy that it gives, and keep the counts.
A new schedule on a started scheduler SHALL cancel the armed tick and arm one at the new schedule's next slot. A new
target, message or retry policy SHALL count from the next `Run`, also one that is already waiting. An `Update` with no
field SHALL change nothing.

#### Scenario: A new schedule

- **WHEN** a started scheduler with the schedule `daily(2, 0)` gets an `Update` with the schedule `every(60)`
- **THEN** the next delivery comes at the next minute, and none comes at 02:00 from the old tick

#### Scenario: A new message

- **WHEN** a started scheduler gets an `Update` with another message, and the clock reaches the next slot
- **THEN** the target gets the new message, and `runs` counts on from where it was

#### Scenario: A new target

- **WHEN** a started scheduler gets an `Update` with another target, and the clock reaches the next slot
- **THEN** the new target gets the stored message and the old one does not

#### Scenario: A new retry policy

- **WHEN** a scheduler without a retry policy gets an `Update` with a policy of two attempts, and then delivers to a
  target whose handler fails on the first run
- **THEN** the target handles the same message again after the backoff

#### Scenario: An empty update

- **WHEN** a started scheduler gets an `Update` with no field
- **THEN** its state and its armed tick stay as they were

#### Scenario: A waiting run sees an update

- **WHEN** a `Run` of a scheduler waits in its mailbox behind an `Update` with another message
- **THEN** the `Run` sends the new message

#### Scenario: A new schedule before the start

- **WHEN** a scheduler that was never started gets an `Update` with a schedule
- **THEN** the schedule is kept and nothing is delivered

### Requirement: The ticks of a scheduler are its own

A `Tick` that the scheduler did not send itself, or that is not its armed tick, SHALL end `done` and change nothing.

#### Scenario: A tick sent by a client

- **WHEN** a client sends a scheduler a `Tick`
- **THEN** the message ends `done`, nothing is delivered, and the armed tick stays as it was

### Requirement: A failed delivery does not end the chain

A `Run` to a target that does not exist SHALL fail with the cause `commit`, and a `Run` whose send is refused at the
call SHALL fail with the cause `refused`; in both cases the next slot SHALL deliver as usual. A `Run` whose read of the
store fails SHALL be retried by the retry policy that the scheduler gives each `Tick` and each `Run` it sends. A
failed `Run` that an operator retries, or that the dead letters redrive, SHALL deliver again.

#### Scenario: A target that is gone

- **WHEN** the target of a started scheduler is deleted and the clock passes two slots
- **THEN** the `Run` of each slot ends `failed` with the cause `commit`, and the scheduler's next tick stays armed

#### Scenario: The target is back

- **WHEN** a target that was gone is created again before a slot
- **THEN** it gets the stored message at that slot

#### Scenario: A message the target does not take

- **WHEN** a scheduler's stored message has a type name that its target's family does not have
- **THEN** the `Run` of each slot ends `failed` with the cause `refused`, and the chain goes on

#### Scenario: A failed run is retried

- **WHEN** a `Run` failed because its target was gone, the target is created again, and an operator retries the `Run`
- **THEN** the target gets the stored message

### Requirement: A start or a new schedule arms a chain again

A chain SHALL end when its armed tick goes without running: it is purged, cancelled, or fails for good. A retry or a
redrive of a failed tick SHALL NOT arm it again. A `Start`, or an `Update` with a schedule, SHALL arm it again.

#### Scenario: A start after a tick that failed for good

- **WHEN** the armed tick of a scheduler ended `failed` and the scheduler gets `Start`
- **THEN** a new tick is armed and the next slot delivers

#### Scenario: An update after a tick that failed for good

- **WHEN** the armed tick of a scheduler ended `failed` and the scheduler gets an `Update` with a schedule
- **THEN** a tick is armed at the new schedule's next slot

#### Scenario: A purge

- **WHEN** the mailbox of a started scheduler is purged and the clock passes the next slot
- **THEN** nothing is delivered, until a `Start` arms a tick again

#### Scenario: A retried tick

- **WHEN** the armed tick of a scheduler ended `failed` and an operator retries it
- **THEN** the retried tick ends `done`, nothing is delivered, and no tick is armed
