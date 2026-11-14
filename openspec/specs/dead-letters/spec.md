# dead-letters Specification

## Purpose
The dead-letters actor collects failed messages through a watch, with no polling. An operator sends a letter
again once, or forgets it.

## Requirements

### Requirement: Dead letters are told of failed messages by a watch

The dead-letters actor SHALL have the type name `cicada.dead-letters`. For every `Failed` it gets, it SHALL record a
letter with the id, the actor, the type, the payload, the correlation id, the sender, the error, the cause and the
attempts of the failed message, and the time it was recorded. It SHALL ignore a `Stopped`, a `Failed` about a
message of its own, and a `Failed` that the store did not mark as a notification. What it is told of is decided by the
watches that are set for it.

#### Scenario: Every failure

- **WHEN** a watch on `*` is set for a dead-letters actor, and a message to any actor ends `failed`
- **THEN** its state holds a letter for that message, with its payload, its error and its cause

#### Scenario: A `Failed` sent by a client

- **WHEN** a client sends a dead-letters actor a `Failed`
- **THEN** the message ends `done`, and nothing changes: no letter is recorded

#### Scenario: One type

- **WHEN** the watch is on the target `billing`, and messages to `billing.invoices/a` and to `shipping/b` end `failed`
- **THEN** it holds a letter for the first and none for the second

#### Scenario: A failure of its own

- **WHEN** a dead-letters actor with a watch on `*` fails a message of its own
- **THEN** it records no letter for it

#### Scenario: No delay

- **WHEN** a message ends `failed`
- **THEN** the letter is recorded by the next turn of the dead-letters actor, with no timer in between

### Requirement: Dead letters keep at most their capacity

The dead-letters actor SHALL keep at most its capacity of letters, 1,000 unless the type is built with another, and
SHALL drop the letters that were recorded first.

#### Scenario: More letters than the capacity

- **WHEN** a dead-letters actor with a capacity of 2 records a third letter
- **THEN** it holds the second and the third

### Requirement: A letter can be sent again once, or forgotten

`Redrive` SHALL send the failed message of a letter again, to the same actor, with the same type, payload and
correlation id, and SHALL record the id of the new message in the letter. A letter that was sent again SHALL NOT be
sent a second time. When the type of the actor no longer takes the message, nothing SHALL be sent and the letter
SHALL record why. `Discard` SHALL forget a letter.

#### Scenario: A redrive

- **WHEN** an operator sends `Redrive` for a letter whose actor exists
- **THEN** the actor gets a new message with the payload of the failed one, and the letter holds its id

#### Scenario: A second redrive

- **WHEN** an operator sends `Redrive` again for that letter
- **THEN** nothing is sent

#### Scenario: A redrive that fails again

- **WHEN** the message of a redrive ends `failed` too
- **THEN** there is a second letter, for the new message

#### Scenario: A type that no longer takes the message

- **WHEN** an operator sends `Redrive` for a letter whose message type the actor's type no longer has
- **THEN** nothing is sent, and the letter records why

#### Scenario: The actor is gone

- **WHEN** an operator sends `Redrive` for a letter whose actor does not exist
- **THEN** the `Redrive` ends `failed` with the cause `commit`, and the letter is as it was

#### Scenario: A discard

- **WHEN** an operator sends `Discard` for a letter
- **THEN** the state no longer holds it

#### Scenario: A letter that is not there

- **WHEN** an operator sends `Redrive` or `Discard` for an id that is no letter
- **THEN** nothing changes and the message ends `done`

### Requirement: A letter keeps every accepted payload without adding too much depth

The dead-letters actor SHALL record an accepted failed payload of at most 61 levels in the letter's raw `payload`
field and a deeper accepted payload in `payloadJson` as compact JSON text. A letter SHALL contain exactly one of
these fields. `payload` SHALL accept any JSON value, including null, and `payloadJson` SHALL be a string. The generated
schema and typed decoding SHALL enforce the same presence rule. The ordinary state and send depth limits SHALL remain.

#### Scenario: Raw boundary and null

- **WHEN** messages with payloads of depth 61 and with a raw null payload end failed and notify dead letters
- **THEN** their letters have a `payload` field with the original value and no `payloadJson`
- **AND** a saved letter with `payload:null` decodes as a present raw null, not a missing field

#### Scenario: Encoded boundary

- **WHEN** accepted payloads of depth 62 and 64 end failed and notify dead letters
- **THEN** both notice turns end done, each letter contains only `payloadJson` for its payload, and the state is within 64 levels
- **AND** `value()` returns the captured failed payload and redrive sends the same payload, type and correlation id

#### Scenario: Old and new values that resemble an encoding

- **WHEN** a saved old letter or a new failure has a raw JSON-looking string, null, number, or an object with a
  `payloadJson` or `$cicada.json` key
- **THEN** its data is not treated as an encoding marker
- **AND** redrive sends that original value, with deep newly recorded objects using the separate `payloadJson` field

#### Scenario: Presence and schema agreement

- **WHEN** a host creates a dead-letter state over HTTP with a letter containing both fields, neither field, or `payloadJson:null`
- **THEN** the schema check refuses it with 422 and no actor is created
- **AND** typed state decoding refuses those same shapes, while accepting `payload:null` and a string `payloadJson`

### Requirement: The payload API decodes without changing captured numbers

`DeadLetter.payload` SHALL be public `LetterPayload::{Value(Value), Json(String)}` with
`value() -> Result<Value, serde_json::Error>`. Reading state SHALL NOT parse encoded text; writing SHALL preserve its
selected form. Text conversion SHALL preserve captured Value semantics, exact integers and finite f64 bits. Original
JSON spelling/order and values already normalized by a database are outside this conversion guarantee.

#### Scenario: On-demand decoding

- **WHEN** a caller reads a letter with valid or invalid encoded text
- **THEN** reading succeeds without parsing the text, `value()` returns its decoded value or parse error on demand,
  and serialization retains the chosen raw or encoded form even for a shallow value

#### Scenario: Exact numeric conversion

- **WHEN** a payload containing `1.0715660391465826e-75`, its negative, the smallest positive subnormal, the largest
  finite f64, negative zero, and i64/u64 boundary integers is converted to the text form and back
- **THEN** integers remain exact and every f64 has the same bits
- **AND** the same conversion after a backend read is compared to that captured value, without claiming to undo JSONB normalization

### Requirement: An unreadable encoded payload does not block the actor

Redrive SHALL decode text on demand. A parse error SHALL record `redriveError` beginning `The kept payload cannot be
read:`, keep the original letter and send nothing. A decoded payload SHALL pass through the ordinary send validation;
depth and NUL errors SHALL be recorded through the existing redrive error path. Store errors and rejected commits
SHALL retain their existing behavior. No parse failure SHALL prevent a subsequent Discard or another letter's redrive.

#### Scenario: Invalid and excessively nested JSON text

- **WHEN** a letter contains invalid JSON text or text exceeding serde_json's normal parser nesting limit
- **THEN** state decoding succeeds, redrive sends nothing and records the parse error without replacing the text
- **AND** Discard and redrive of a different valid letter still work

#### Scenario: Parsed data is invalid to send

- **WHEN** a letter's text decodes to a 65-level payload or to a string or key containing NUL
- **THEN** redrive records the ordinary send validation error, sends nothing and retains the letter

#### Scenario: A refused batch publication

- **WHEN** a batch publication with an accepted payload fails because its flow input would make the flow state too deep
- **THEN** its notification can be recorded as a letter and redriven with the same payload
- **AND** the redriven publication can fail for the same reason and produce a new letter; encoding does not relax flow state limits

### Requirement: A breaking dead-letter upgrade preserves exported letters explicitly

The changed state contract SHALL have a new hash. The guide SHALL provide a tested backup/restore procedure for
independent, childless actors with no incoming watches, covering all names, states, pause flags and outgoing watches.
It SHALL require quiescence, verified backups and exports, stopping old hosts, and checks before deletion that each
restore state decodes as the new typed state, is at most 64 levels deep and contains no NUL in strings or keys, and
that no address repeats in the export. These example checks SHALL NOT be described as JSON Schema validation.
It SHALL require verification before resuming and disclose unsupported topology, new instance identity and notification gaps.

#### Scenario: Maintenance safeguards

- **WHEN** an operator prepares the upgrade
- **THEN** the guide requires quiescing watched work and senders, draining turns and notices, and stopping every old
  local/remote host and bootstrapper before replacing the registration
- **AND** it explains that hash checks do not evict already-running local nodes and requires checking for parents,
  children and incoming watches before deleting any instance
- **AND** those topologies require a host-specific migration because deletion removes descendants and emits Stopped

#### Scenario: Incomplete export or unquiesced work

- **WHEN** an export, backup or restore validation is incomplete
- **THEN** the documented procedure stops before destructive operations
- **WHEN** work cannot be quiesced
- **THEN** the guide discloses the notification gap, outcome retention limits and that business retries may repeat
  external effects rather than reconstruct historical letters

#### Scenario: Old raw letters survive restore

- **WHEN** an operator follows the maintenance recipe for multiple eligible actors holding raw letters and outgoing watches
- **THEN** restored letters keep their message ids, payload meaning, metadata and prior `redrivenAs` values
- **AND** pause settings and watches are restored before work resumes, and a letter already redriven is not sent again
- **AND** the guide states that versions, epochs and creation times belong to new instances and cached leases cannot be reused

#### Scenario: Readable legacy deep export

- **WHEN** a readable export contains a raw payload too deep to restore inside a letter
- **THEN** the documented exact-parsing conversion can move it to `payloadJson` without discarding the letter
- **AND** the check before deletion accepts the converted state, redrive still refuses payload depth greater than 64, and Discard works

#### Scenario: Raw legacy state is not automatically repaired

- **WHEN** a legacy raw letter of depth 70 remains in a readable store state and a turn tries to write that state
- **THEN** the ordinary state-depth check still refuses it; this change adds no automatic live repair
- **AND** the guide directs the operator to export/convert/restore or deliberate Discard, retaining a backup for unreadable data

#### Scenario: Old hosts and readers

- **WHEN** old code starts against the new registration, or an old remote worker tries to lease the changed type
- **THEN** the existing contract mismatch rules refuse it
- **AND** an old typed state reader cannot decode a `payloadJson`-only letter, so readers must upgrade too
