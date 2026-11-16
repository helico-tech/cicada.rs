## ADDED Requirements

### Requirement: Actor design guidance explains principles through Cicada

A guide at docs/guide/design.md SHALL explain design principles through concrete stories, followed by their Cicada
form and links to checked examples and API guides. It SHALL cover the nine-step method's questions and written
results, with actor maps, protocol/state tables, lifecycle diagrams and success/failure sequences in Mermaid.

#### Scenario: A consumer starts with a problem rather than actors

- **WHEN** a consumer uses the guide to design an order or session
- **THEN** the guide works through promise, domain events, invariant ownership, identity/lifecycle, responsibility
  cards, protocols, coordination/time, outside effects, and checking/simplifying
- **AND** it considers a simpler non-actor design where appropriate, rather than turning every class into an actor
- **AND** each stage identifies its written result without implementing the later interactive skill workflow

#### Scenario: Choose and correct a design

- **WHEN** a consumer chooses boundaries or coordination
- **THEN** a compact pattern chooser links to existing plain actors, flows, request/reply, saga, timer, supervision
  and outside-service examples, naming when each fits and its costs
- **AND** mistakes include split invariants, an actor carrying all traffic, unbounded state, assumed message order,
  unclear acknowledgement meaning, wait cycles and outside effects that are unsafe to repeat
- **AND** each mistake has a symptom and correction, without introducing new runtime features

### Requirement: Guidance preserves current runtime limits and source authority

The guide SHALL agree with docs/architecture.md and the resolved source, not adopt the research's broad statements
as guarantees. It SHALL distinguish atomic actor effects, outside effects, host memory and durable actor state.
The guide SHALL own reusable knowledge; the consumer skill SHALL route to it rather than duplicate it.

#### Scenario: Smaller research findings become guidance

- **WHEN** a design uses an application database, internal messages, replies, deferred work or state machines
- **THEN** the guide explains separate application-database writes as outside effects, sender checks as protocol
  checks rather than authentication, typed reply refs or message.from, bounded deferred state, and state/message tables
- **AND** it links existing capabilities for hand-written circuit breakers and group recovery
- **AND** it does not claim G4 deduplicates repeated business submissions or arbitrary outside effects

#### Scenario: A live handle meets durable state

- **WHEN** a design needs a socket, process or device handle
- **THEN** the guide compares a turn using a host-held handle with a host task following durable actor state
- **AND** it bounds the former's machine-local use to one node hosting the type today, and teaches the latter with
  the websocket example, without implying unshipped pinning exists
- **AND** diagrams distinguish ordinary messages, committed state, host reads and outside socket I/O

### Requirement: The session accepts bounded output or explicitly refuses it

The example SHALL use one precreated session per logical client, with a unique key never reused after deletion.
It SHALL retain at most 100 unacknowledged frames, each at most 1024 UTF-8 bytes and containing no NUL, with sequence
numbers from 1 through 9007199254740991. Acceptance and a producer reply SHALL commit together. Full buffers SHALL
refuse new output without eviction or automatic retry.

#### Scenario: Accept output and correlate its result

- **WHEN** Enqueue names a valid body and existing typed producer reply actor, and capacity/sequence remain
- **THEN** one frame is appended with the next sequence and Accepted names that sequence and Enqueue's MessageId
- **AND** the frame, reply and input completion commit together
- **AND** the demo producer holds at most one pending request/result, and transport enqueue success is not presented
  as session acceptance or client processing

#### Scenario: Refuse without losing accepted output

- **WHEN** an otherwise storable body exceeds 1024 UTF-8 bytes, the buffer is full, or the sequence range is exhausted
- **THEN** Rejected names the request and InvalidBody, BufferFull or SequenceExhausted, in that precedence
- **AND** sequence and retained frames remain unchanged
- **AND** a producer may submit a new message after explicit refusal; an unknown transport result instead uses the
  original id within Cicada's dedup window, with no permanent business-deduplication promise

#### Scenario: NUL is refused before the session can handle the request

- **WHEN** a host submits an Enqueue whose body contains NUL through ActorSystem
- **THEN** submission returns OpError::Invalid before enqueueing the message
- **WHEN** a producer turn tries to send that Enqueue
- **THEN** the send returns TurnError::Invalid before buffering it
- **AND** neither path queues the Enqueue or produces a session Accepted/Rejected reply
- **AND** tests and guide distinguish these submission errors from the actor's oversized-body InvalidBody refusal

#### Scenario: A reply destination disappeared

- **WHEN** the reply actor no longer exists at commit
- **THEN** the normal failed commit leaves no accepted frame or advanced sequence
- **AND** the example does not suppress the reply error or claim application acceptance

#### Scenario: Disconnect and end of life

- **WHEN** a client disconnects or a host restarts
- **THEN** retained output remains in actor state with no automatic expiry
- **WHEN** an operator deletes the actor or purges pending work
- **THEN** the guide explains that the replay/work promise ends for what was removed
- **AND** a new stream uses a new key, never reset sequence numbers at an old key
- **AND** the buffer bound is not described as a mailbox-admission or total-session-count limit

### Requirement: Client acknowledgements trim only a contiguous processed prefix

Socket JSON SHALL use tagged Hello with stream/acked, Frame with stream/seq/body, and Ack with stream/through.
The session SHALL maintain a monotone cumulative ack, initially 0, and trim only frames at or below a valid ack.
One authenticated logical client SHALL own the processed cursor; independent clients SHALL use independent sessions.

#### Scenario: Valid, duplicate and invalid acknowledgements

- **WHEN** Ack names this stream and a positive exact integer above the current ack but no higher than the assigned sequence
- **THEN** one turn advances the ack and removes frames through it, freeing capacity
- **WHEN** an ack repeats, goes backwards, names another stream or lies outside that range
- **THEN** no retained output or ack is changed
- **AND** the server trusts the authorized client's assertion of processing; a socket write alone never produces Ack

#### Scenario: Replay after an acknowledgement was lost

- **WHEN** a client processed a frame but its acknowledgement did not commit
- **THEN** the host may replay that frame on refresh or reconnect
- **AND** the client compares against its persisted per-stream contiguous cursor, ignores the duplicate and re-acks it
- **AND** only the next contiguous new sequence is applied; a gap causes reconnect, never skipping ahead
- **AND** no exactly-once client-side processing guarantee is claimed

#### Scenario: Overlapping reconnects and lost client state

- **WHEN** a logical client reconnects while the old socket can still deliver bytes
- **THEN** its local connection generation ignores old-socket callbacks, and old monotone acknowledgements remain safe
- **AND** the host uses no durable socket registry or socket-fencing promise
- **WHEN** the client loses its processed cursor and it is behind Hello.acked
- **THEN** it needs application recovery or a new session because acknowledged frames are no longer available

### Requirement: The host reconciles durable state and owns bounded socket I/O

The bridge SHALL use public ActorSystem operations, subscribe before reading, and reread on matching change hints,
Resync and a one-second timer. It SHALL send retained frames in sequence above the snapshot's ack. It SHALL bound
socket writes and store reads/sends to five seconds and stop all connection work on error, closure or shutdown.

#### Scenario: Initial read, lost hints and another host

- **WHEN** a socket opens for an existing session
- **THEN** the host subscribes first, reads without waiting indefinitely for the first hint, sends Hello and replays
  retained frames; each later snapshot may repeat them
- **WHEN** hints are missed, resync occurs, or a reconnect uses another host sharing the store
- **THEN** a fresh read or periodic reconciliation recovers retained output without a pin or access to the old socket
- **AND** the bridge filters exact addresses after the literal-prefix subscription
- **AND** it describes memory/SQLite clone coverage and PostgreSQL peer coverage honestly

#### Scenario: The connection cannot continue

- **WHEN** the session is missing, a store read/send fails, the feed ends, socket I/O fails or times out, or shutdown is requested
- **THEN** the bridge closes the connection and stops/joins its reader and writer without detached work
- **AND** uncertain ack submission does not trim state locally; reconnect reads the durable ack
- **AND** reading client acks proceeds concurrently with socket writes, without an unbounded queue

#### Scenario: Actual websocket adapter

- **WHEN** the example runs through its axum adapter
- **THEN** inbound application messages/frames are limited to 4 KiB and write buffering to 16 KiB with eager writes
- **AND** malformed or binary application input closes the connection; the websocket library handles control frames
- **AND** a loopback demo precreates a fresh stream and producer, emits sample output and prints connection details
- **AND** the guide includes a minimal browser client for cursor/reconnect handling and names host authentication
  and authorization as prerequisites for integration, not properties of knowing a session key

### Requirement: The guide and example are checked and delivered together

The builder SHALL supply the websocket example, tests, guide doctest wiring and existing-skill routes. The docs
session SHALL prepare the guide and navigation before landing. Tests SHALL cover the protocol and bridge on paused
virtual time with TestActors, and one actual websocket exchange SHALL check the adapter.

#### Scenario: Recovery and boundary coverage

- **WHEN** the example is checked
- **THEN** tests cover UTF-8/count/numeric boundaries, actor-level oversized-body refusal, separate NUL submission
  errors with no Enqueue/session reply, atomic reply rollback, ack validation/freeing space,
  duplicate replay, disconnect before ack, lost ack submission, reconnect through a second system, overlaps,
  resync/missed hints, polling, stalled I/O, deletion, store errors/feed end and shutdown
- **AND** deterministic bridge tests exercise the same loop/protocol as the websocket adapter without production fault hooks
- **AND** the adapter test performs a websocket handshake and JSON exchange without real-time sleeps

#### Scenario: Documentation and routes cannot be left broken

- **WHEN** the guide is added
- **THEN** DesignGuideDoctests compiles its Rust blocks, docs link checks resolve its navigation, and the existing
  consumer skill routes to its nine headings and worked example with REQUIRED_ROUTES coverage
- **AND** the docs owner prepares docs/guide/design.md and links in root README.md, docs/README.md, docs/agents.md and core.md
- **AND** the handoff explains submission errors separately from application refusals
- **AND** the handoff names G2 for the acceptance/rollback proving tests and preserves existing guide anchors
- **AND** missing owner-prepared prose blocks landing; no checks are disabled and no new consumer skill is introduced
