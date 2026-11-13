# turn-observer Specification

## Purpose
Code that a host adds to a node hears when each turn starts and how it ended, and can add headers to the
messages the turn sends. It has no say in the turn: it cannot refuse it or change what it commits.

## Requirements

### Requirement: A node tells its observers when a turn starts and ends

A node SHALL call `turn_started` of each observer once for every turn it runs, after the claim and before it decodes
or runs anything, with the actor's address, the message as claimed and the store time of the claim. When the commit
of the turn succeeds, the node SHALL call `turn_ended` on the value that call returned, with the outcome that was
committed and the messages the commit sent. A claim that the claim limit fails is the exception: the spec
`claim-limit` says when its observers are told.

#### Scenario: A handler returns

- **WHEN** a node with an observer runs a turn whose handler returns and sends two messages
- **THEN** the observer is told the start, with the message and an `attempt` of 1, and then the end, with the outcome
  `done` and the two messages

#### Scenario: A handler fails and is retried

- **WHEN** a handler fails for a message whose retry policy has attempts left
- **THEN** the observer is told the end of that turn with a reschedule that holds the error, and the next attempt is
  a new start with an `attempt` of 2

#### Scenario: A handler fails for good

- **WHEN** a handler fails for a message without a retry policy
- **THEN** the observer is told the end with the outcome `failed` and the error

#### Scenario: A message that cannot be decoded

- **WHEN** a node claims a message whose payload does not fit its type
- **THEN** the observer is told the start and the end, with the outcome `failed`

#### Scenario: State that cannot be decoded

- **WHEN** a node claims a message for an actor whose state it cannot decode
- **THEN** the observer is told the start and the end, with the outcome `failed`

#### Scenario: A turn is cancelled

- **WHEN** an operator cancels a turn while its handler runs
- **THEN** the observer is told the end with the outcome `cancelled`

#### Scenario: A commit is rejected

- **WHEN** a handler returns after a send to an actor that does not exist
- **THEN** the observer is told the end once, with the outcome `failed` that the node committed in its place, and no
  message sent

### Requirement: A turn without a commit that its node saw succeed is dropped, not ended

A node SHALL NOT call `turn_ended` for a turn of which it saw no commit succeed. It SHALL drop the value that
`turn_started` returned, which is how an observer learns of it. A later claim of the same message SHALL be a new
start.

#### Scenario: A lease is lost

- **WHEN** a turn loses its lease while its handler runs, and another claim later handles the message
- **THEN** the value of the first turn is dropped without `turn_ended`, and the observer is told a second start, with
  an `attempt` of 2, and its end

#### Scenario: A node stops in the middle of a turn

- **WHEN** a node stops and gives back the lease of a turn whose handler has not ended
- **THEN** the value of that turn is dropped without `turn_ended`

#### Scenario: The actor is deleted while its turn runs

- **WHEN** an actor is deleted from outside while its handler runs
- **THEN** the value of that turn is dropped without `turn_ended`

#### Scenario: A commit whose answer is lost

- **WHEN** the store applies the commit of a turn, the node gets no answer, and its next try is answered that the
  lease is no longer held
- **THEN** the value of that turn is dropped without `turn_ended`, although the message has its outcome

### Requirement: An observer has no say in a turn

An observer SHALL NOT be able to refuse a turn or to change what it commits, other than by the headers of its
sends. A panic in an observer SHALL be caught and SHALL change nothing else: the turn runs and commits as it would
without that observer, the headers that observer added in that call are not used, and the other observers are
called.

#### Scenario: An observer panics at the start

- **WHEN** a node has two observers, the first panics in `turn_started` after it added a header, and the handler
  sends a message
- **THEN** the handler runs, the message ends `done`, the sent message has no header of the first observer, and the
  second observer is told the start and the end

#### Scenario: An observer panics at the end

- **WHEN** the first of two observers panics in `turn_ended`
- **THEN** the outcome of the message stands and the second observer is told the end

#### Scenario: An observer panics when its value is dropped

- **WHEN** a turn loses its lease and the drop of an observer's value panics
- **THEN** the node goes on: the values of the other observers are dropped, and the next claim of the message runs

### Requirement: Several observers are called in the order they were added

A node SHALL accept any number of observers and SHALL call them in the order they were added, at the start of a
turn and at its end.

#### Scenario: Two observers

- **WHEN** a node was built with observer A and then observer B, and runs a turn
- **THEN** the calls are: start of A, start of B, end of A, end of B

### Requirement: An observer can add headers to the sends of a turn

In `turn_started` an observer SHALL be able to add headers for `Sends::All`, `Sends::Timers` or `Sends::Others` through
`TurnStart::add_header_for`. `add_send_header` SHALL mean All and retain its existing behaviour. A timer SHALL be a
send whose target equals the current actor's address and whose requested Delivery is At or After, including a zero
delay or past absolute time. Every other handler send SHALL be Others. This rule SHALL apply to every turn send API,
regardless of how the store later resolves delivery. Observer selection SHALL NOT alter delivery or actor effects.

All SHALL add to both kinds; later calls in one observer and then later observers SHALL win per exact name in each
kind. A header that a send sets itself SHALL keep the send's value for the same exact name. Notifications of watches
SHALL carry none of these headers. An invalid observer header SHALL be omitted with a warning without erasing an
earlier valid value; an observer header SHALL never fail a send. For the selected kind, a send SHALL take its own
headers first, then observer headers in name order until the existing 64-header limit; excess SHALL be omitted with
warnings. A panicking turn_started SHALL discard all header additions from that observer, in both kinds. Core header
storage and precedence SHALL remain case-sensitive.

#### Scenario: Headers reach every send

- **WHEN** an observer adds the header `trace` and the handler sends two messages
- **THEN** both messages have the header `trace` with the observer's value

#### Scenario: A send's own header wins

- **WHEN** an observer adds the header `trace` and the handler sends a message that sets `trace` itself
- **THEN** the message has the value the send set

#### Scenario: Two observers add the same header

- **WHEN** observer A and then observer B were added, both add the header `trace`, and the handler sends a message
- **THEN** the message has the value of B

#### Scenario: A header that is too long

- **WHEN** an observer adds a header with a value of 5,000 characters and the handler sends a message
- **THEN** the send succeeds and the message does not have that header

#### Scenario: A header that a message refuses

- **WHEN** an observer adds a header whose value holds a NUL character, and one with a name of 300 characters, and
  the handler sends a message
- **THEN** the send succeeds and the message has neither header

#### Scenario: A message that is full

- **WHEN** an observer adds two headers and the handler sends a message with 63 headers of its own
- **THEN** the send succeeds with 64 headers: its own and the observer's header whose name sorts first

#### Scenario: A notification has no observer headers

- **WHEN** an observer adds the header `trace`, and a turn of a watched actor fails
- **THEN** the `Failed` in the watcher's mailbox has no header `trace`

#### Scenario: A header selection distinguishes requested timer sends

- **WHEN** an observer adds `audit` with All, `timer` with Timers and `work` with Others, and a handler sends an
  immediate self-message, a delayed self-message, and immediate and delayed messages to another actor
- **THEN** every message has audit; only the delayed self-message has timer, and the other three have work
- **AND** add_send_header is equivalent to selecting All

#### Scenario: Every send path uses the same timer classification

- **WHEN** a handler targets its own address with At or After through send_self_with, send_with, send_to or send_raw
- **THEN** each send selects Timers and not Others, just as the corresponding address equality requires

#### Scenario: Zero delays and past times are still timer requests

- **WHEN** a handler sends to itself with After(0), At(now), At(past), or an At that becomes past before commit
- **THEN** each send selects timer headers even though the store may resolve its delivery to commit time
- **AND** a Now send to itself selects other headers

#### Scenario: Calls and observers override independently per kind

- **WHEN** observer A adds x=all for All and then x=timer for Timers, and observer B later adds x=other for Others
- **THEN** a timer has x=timer and another send has x=other
- **AND** an All call made last in an observer overwrites its earlier x value in both kinds

#### Scenario: The send's own headers and limits apply independently to each kind

- **WHEN** a timer and an ordinary send each supply an exact-name header also selected by observers, plus enough
  own headers that only one extra fits
- **THEN** each keeps its own value, gains only the first remaining selected observer header in name order, and succeeds
- **AND** each has at most 64 headers and contains no header solely from the unselected kind

#### Scenario: Invalid selected headers do not erase previous values

- **WHEN** an observer adds a valid selected header, then attempts to replace it with an invalid value
- **THEN** the invalid addition is omitted with a warning and the earlier valid value remains for that selection

#### Scenario: A panicking observer loses both selections

- **WHEN** an observer adds headers for Timers and Others and then panics in turn_started
- **THEN** neither set reaches any send, the handler runs normally, and other observers retain their behaviour

### Requirement: The telemetry of `cicada-otel` is an observer

`ActorTelemetry` SHALL be a `TurnObserver`. It SHALL give every turn one span, from its claim to its commit, with
the outcome as `cicada.outcome`. It SHALL inject its new span context through the configured propagator once, adding
nonempty emitted headers to Others under their original names and to Timers under names prefixed `cicada-link-`.
The prefix SHALL be a stable cicada-otel convention (ADR 0032); core SHALL treat these as ordinary headers.

The receiving span SHALL extract its parent from plain headers and independently extract link context through a
prefix-stripping reader, using the configured propagator and a fresh empty Context for each. It SHALL add one link
before span start if the extracted link SpanContext is valid, otherwise none. The reader's keys SHALL expose only
prefixed names with the prefix removed; prefix matching SHALL be case-insensitive. Lookup SHALL prefer the exact
full key, then the first case-insensitive full-key match in BTreeMap order, matching existing HeaderReader behaviour.
The link SHALL never become the parent. Without valid plain parent context the span SHALL be a new root, regardless
of the ambient context. A valid plain parent and a valid link SHALL both be honoured, including on external sends.
The receiver SHALL NOT infer timer status from due time, sender, actor type or retry attempt.

Missing, malformed, unsupported or omitted link headers SHALL NOT fail a turn. The link SHALL depend only on the
validity of context the configured propagator can extract from retained fields. Explicit send headers and headers
from other observers SHALL keep the core precedence rules. Injection SHALL use the new span-only context, as before;
this change SHALL NOT forward arbitrary incoming headers or introduce baggage propagation. A turn that ends failed
or is rescheduled SHALL have an exception event with the error. A turn without a commit the node saw succeed SHALL
end its span as abandoned. Existing span attributes, status, duration and lag metrics SHALL otherwise be unchanged.

#### Scenario: A span per turn

- **WHEN** a node with `ActorTelemetry` runs a turn that sends an immediate message, and then the turn of that message
- **THEN** there are two spans, the first with `cicada.outcome` `done`, and the second is a child of the first

#### Scenario: A handler fails and is retried

- **WHEN** a handler fails and its message is retried
- **THEN** the span of that turn has an `exception` event with the error and `cicada.outcome` `rescheduled`

#### Scenario: State that cannot be decoded

- **WHEN** a node with `ActorTelemetry` claims a message for an actor whose state it cannot decode
- **THEN** the turn has a span with `cicada.outcome` `failed`, the error status and an `exception` event

#### Scenario: A turn that commits nothing

- **WHEN** a turn loses its lease
- **THEN** its span ends with `cicada.outcome` `abandoned` and no error status

#### Scenario: A recurring timer starts linked roots

- **WHEN** a traced actor rearms itself with a delayed Tick through three turns without explicit parent headers
- **THEN** the three spans have distinct trace ids; each timer receiver has no parent and exactly one link to the
  trace id and span id of the turn that sent it
- **AND** ordinary messages each tick sends are children within that tick's trace

#### Scenario: Immediate self-sends and delayed sends to others remain children

- **WHEN** a traced turn sends an immediate message to itself and a delayed message to another actor
- **THEN** both receiving spans are children of that turn in its trace, with no automatically added link

#### Scenario: Flow sleeps and scheduler deadlines follow the same rule

- **WHEN** a durable flow commits immediate continuations, then sleeps, including for zero duration
- **THEN** continuations before the sleep share their chain's trace, the sleep wake Continue starts a root linked to
  the turn that armed it, and subsequent immediate continuations share the wake turn's trace
- **AND** a scheduler's delayed self-Tick starts a linked root and its immediate Run and target sends inherit that trace

#### Scenario: A timer may have an explicit parent and a link

- **WHEN** a handler supplies a valid plain traceparent on its delayed self-send and telemetry injects its link
- **THEN** the receiver is a child of the explicit parent and also links to the sending turn
- **AND** an All header from another observer can similarly supply a parent without losing its unrelated audit headers

#### Scenario: The all-zero workaround remains valid

- **WHEN** a timer carries the caller's invalid all-zero traceparent and a valid automatically injected link
- **THEN** its turn is a root linked to the sender, and the message succeeds

#### Scenario: Explicit link fields win by exact name

- **WHEN** a timer supplies its own cicada-link-traceparent and telemetry adds the same exact header name
- **THEN** the caller's value remains and determines the extracted link
- **AND** case variants are separate core keys; when duplicates exist the exact requested full key wins at extraction

#### Scenario: Link metadata is missing or malformed

- **WHEN** a timer has no usable link fields, malformed link fields, or only tracestate because the traceparent
  field was omitted at the header cap, and no valid plain parent
- **THEN** its span is a root without a link, even inside an ambient trace, and message handling succeeds
- **AND** if valid plain parent context is present it still supplies the parent

#### Scenario: Header limits can omit telemetry without changing actor behaviour

- **WHEN** a timer's own 64 headers leave no room for its generated link fields
- **THEN** it succeeds without those fields and starts a root without a link if no plain parent was supplied
- **AND** a propagator key whose prefixed name exceeds 200 characters is omitted with a warning rather than failing a send

#### Scenario: Case-insensitive link extraction

- **WHEN** a message carries a valid Cicada-Link-Traceparent without a conflicting exact lowercase key
- **THEN** the span links to that context as it would for cicada-link-traceparent

#### Scenario: A custom propagator determines both representations

- **WHEN** ActorTelemetry uses a propagator with a trace header other than traceparent
- **THEN** ordinary sends use its native header, timer sends use that header with cicada-link- prepended, and the
  receiver extracts the same sender SpanContext as a link using the prefix-stripping reader
- **AND** a composite propagator produces at most one extracted link; no W3C-only codec is required

#### Scenario: Policy retries and lost claims retain the sender link

- **WHEN** a timer without explicit parent context is retried by its policy or re-executed after a lost claim
- **THEN** each attempt is a fresh root linked to the original sending span; no attempt becomes the next attempt's parent
- **AND** rescheduled, failed and abandoned outcomes retain their existing span treatment

#### Scenario: Operator retry and deliver-now preserve trace headers

- **WHEN** an operator retries a failed timer or makes a pending timer due now
- **THEN** its retained headers determine the parent and link as before; retry has a new message id and root span,
  while deliver-now does not convert the existing link to a parent

#### Scenario: An older queued timer retains its parent until rearmed

- **WHEN** a new node handles a timer carrying only plain propagation headers from before the upgrade
- **THEN** that turn is a child as before; its next automatic timer send carries prefixed context and starts a linked root
- **AND** an old node reading new prefixed headers ignores them and starts a root without a link if no plain parent exists

#### Scenario: Notifications and redrive use their existing header rules

- **WHEN** the store sends a watch notification and a dead-letter actor later redrives the failed message to another actor
- **THEN** the notification has no observer headers and starts a root, and the redrive inherits the dead-letter turn's
  context instead of copying the failed message's link fields
- **AND** without ActorTelemetry, observers' own header selections still work but no automatic telemetry is added
