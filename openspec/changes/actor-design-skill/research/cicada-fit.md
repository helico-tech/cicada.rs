# Can a designer express these designs in Cicada?

Checked on 2026-11-15 against Cicada 0.18.11 (`docs/architecture.md`, the guides in `docs/guide/`, `examples/src/`).
The design idioms come from [actor-design.md](actor-design.md). The question for each one is "can a designer express
this in Cicada?", not "how does the runtime do it?".

## Answer

Yes, for almost everything. Of 86 idioms: 48 are built in (two of them only for flows, three with a limit), 16
have a tested worked example, 19 can be written by hand with the public API, and 3 are a gap. The three are one
case: a turn that acts on a thing that lives on one machine. It has a workaround today, and the open proposal
`pinned-actors` is its fix.

| Verdict | Meaning |
| --- | --- |
| **Built in** | The core or a ready-made actor does it. |
| **Example** | A tested worked example in `examples/src/` shows it; the designer writes it. |
| **By hand** | The public API allows it; there is no example or helper. |
| **Gap** | Not expressible well today. |

## What Cicada gives the designer to build on

A design rule often depends on what the runtime promises. These are Cicada's promises, as a designer needs them
(`docs/architecture.md#guarantees`). They decide which idioms are needed at all.

| Cicada promises | What it means for a design |
| --- | --- |
| One message at a time per actor. State, sends, creates and the message's completion commit together or not at all (G1, G2). | Each actor is a consistency boundary. "Change my state and tell others" is one atomic step: no outbox to design. |
| Effects inside the system happen once. Only the handler's own code and calls to the outside can run again (G4). | Messages between actors need no duplicate check. Only outside calls need an idempotency key. |
| Sends are one-way. A turn cannot wait for a reply. | Request and reply is always two messages. Ask chains and call-cycle deadlocks cannot be written. |
| An actor exists only after an explicit create. A send to a missing actor fails the sender's turn. | Every design must say who creates each actor. Create and first send can share one commit. |
| An actor keeps nothing in memory between turns. Only its stored state survives. | A live handle (socket, child process, device session) cannot live in the actor. It lives in the host. See [the gap](#the-one-real-gap-a-turn-that-acts-on-a-thing-on-one-machine). |
| A delayed message is durable. | A timeout, deadline or schedule is a message that survives restarts. |
| State can be read from outside without a message. A turn cannot read another actor's state. | Screens and reports read snapshots. Actors ask each other by message. |
| Per actor, order follows due time, then arrival. A retried message goes to the back. The sender picks the retry policy. | Do not design on order across senders or across retries. A "cancel" waits its turn like any message. |
| Every handled message is a commit in the store. | Chatty protocols cost more here than in an in-memory runtime. Batch high-rate data before it reaches an actor. |
| All actors share one store, and each actor's record has one writer: its own turn. | The store is not a side channel. The application's own tables are outside it: a handler's write to them is an outside effect. |

## A. Finding the actors

| Idiom | Verdict | How |
| --- | --- | --- |
| One actor per entity or consistency boundary | Built in | A type plus a key; the turn is the transaction. `docs/guide/core.md#design-an-actor` |
| Identity by a stable business key | Built in | The key is one string; a composite key converts to one string (`examples/src/fanout.rs`). |
| Explicit lifecycle: create, stop, delete | Built in | `turn.create`, `turn.stop()`, `turn.delete`; a `Stopped` tells watchers. |
| Parent owns children; ending the parent ends them | Built in | `turn.create_child`; a delete or stop removes the whole tree in one commit. `docs/guide/core.md#child-actors` |
| One actor per business process or conversation | Built in | A flow run, or an actor keyed by the process id (`examples/src/saga/`). |
| Short-lived child per job (per-session child) | Example | `examples/src/fanout.rs`: the child replies and stops in one commit. |
| Singleton or coordinator | Example | A unit ref at `type/_` (`examples/src/singleton.rs`). It serializes all its work: the bottleneck risk stays the designer's. |
| Registry or index of actors | Example | `examples/src/registry.rs`; from outside, list by address prefix without any actor. |
| Split a hot actor by a natural key | By hand | The sender picks the key; there is no router. |
| What a function can do stays a function | Built in | A handler is plain Rust; nothing forces an actor. |
| Containment, creation and recovery as separate relations | Built in | A parent owns cleanup (`create_child`). Recovery is a watch (`turn.watch`). The two are independent. |
| Business life apart from a running instance | Built in | An actor is its stored state. There is no instance in memory to confuse with it. |
| Key reuse and late messages after the end | Built in, with a limit | A delete purges the mailbox, a new create is a new life, and an old handler commits nothing. Outside work from the old life can still land: give a reused identity a new key when that matters. |

## B. State

| Idiom | Verdict | How |
| --- | --- | --- |
| The actor alone changes its state | Built in | No effect writes another actor's state. |
| Actor as a state machine | By hand | An enum in the state and a `match`. The compiler checks that every message is handled, not every message in every state. |
| Put a message aside until the actor is ready (stash) | By hand | Keep the request in state with a bound (`examples/src/flow_approval.rs` does), or send it to self with a delay. |
| Bounded state | By hand | The designer's rule; dead letters show a capacity that drops the oldest. |
| Read model for screens and reports | Built in | `list_actors_with_state` and the change hints (`docs/guide/core.md#follow-changes`). |
| Refer to other actors by identity only | Built in | A ref is an address; it is what state and messages hold. |
| Decide once: the new state and the intended actions together | Built in | A turn buffers its effects and commits them with the state. |
| Ask the owner for a decision, not for data | By hand | Design the command and its reply (`TryReserve`, not a read and a write). |
| Messages are immutable values | Built in | A message is JSON. |
| State as a list of facts (event sourcing) | By hand | Cicada stores the current state. Keep a bounded history in state, or publish events to a topic. |

## C. Protocols

| Idiom | Verdict | How |
| --- | --- | --- |
| Messages are the API; the receiver owns the vocabulary | Built in | One message enum per ref; a published contract with JSON Schemas. `docs/guide/core.md#the-contract-of-a-type` |
| Request and reply as two messages, with correlation | Example | `examples/src/reply/`: reply to `message().from`, echo `correlation_id`. |
| Typed reply route | Built in for flows, by hand elsewhere | Flows have `ReplyTo<T>`. A plain actor replies with `send_to`: the reply's type name is checked when sent, not when compiled. |
| Reply address inside the message when forwarding | Example | `examples/src/saga/` carries `reply_to: Order`. |
| Ask with a timeout | Built in for flows, example elsewhere | `step.wait_timeout`; in a plain actor, a reply message plus a timer (`examples/src/timer.rs`). |
| Internal messages kept out of the public protocol | By hand | Every variant is public and sendable over HTTP. The ready-made actors check that the sender is the actor itself. |
| Idempotent message by a stable id | Built in | The sender may set the message id; a repeat within the retention window (7 days by default) is refused as a duplicate. |
| Three identities: operation, conversation, attempt | Built in | The message id, `correlation_id` and `attempt`. `caused_by` links a message to the one that caused it. |
| An acknowledgement that says what it proves | Built in | A send is acknowledged once it is stored. Its outcome (`done`, `failed`, `cancelled`, `purged`) is read later by id, for 7 days by default. |
| Self-contained message; large content by reference | By hand | A value holds at most 64 nested levels; keep a reference to large content. |
| Unknown or unreadable message is visible, not dropped | Built in | It ends `failed` with cause `message` and reaches watchers and dead letters. |
| Changing a protocol over time | By hand | Any change to a message or state shape is a new contract. Put the version in the type name (`billing.v2.invoices`). `docs/operations.md#changing-a-contract` |

## D. Coordination

| Idiom | Verdict | How |
| --- | --- | --- |
| Agreement across actors by messages, no shared transaction | Built in | The atomic commit of state and sends is exactly this. |
| Saga with compensation, run by one actor (orchestration) | Example | `examples/src/saga/`; flows add `Flow::on_cancel` for cleanup (`examples/src/flow_resources.rs`). |
| Saga by events (choreography) | Built in | `cicada.topic` to existing actors; `cicada.event` to start flows. |
| Process manager | Built in | A flow: an ordered journal, one new step per turn. `docs/guide/actors.md#flows` |
| Reserve, then confirm or release | Example | The saga reserves stock and releases it on failure. Expiry of a reservation is a timer, by hand. |
| Local knowledge of a partner: requested, confirmed, unknown | Example | The order of `examples/src/saga/` keeps its step machine in state. |
| One atomic step that touches several actors | Built in, with a limit | One turn creates, deletes, pauses and sends to several actors in one commit. It cannot change another actor's state. |
| Shares of one shared limit (allocated rights) | By hand | Each actor holds its share in state; transfers are messages. |
| Bound the work an actor accepts | Example | `examples/src/flow_approval.rs` refuses beyond 100 pending requests. |
| Fan-out and fan-in | Example | `examples/src/fanout.rs`. |
| Scatter and gather: all replies, or the first | Built in | `step.wait_all`, `step.wait_first`, with or without a deadline. `docs/guide/actors.md#joins-waiting-for-several-replies` |
| Pipeline or routing slip | By hand | Each stage sends to the next; carry the final reply address in the message. |
| Publish and subscribe | Built in | `cicada.topic`: one delivery turn per subscriber, so one broken subscriber blocks nobody. |
| Share work over many workers | Built in | One actor per job; any node that hosts the type takes it. No router or pool to design. |
| No deadlock from call cycles | Built in | There is no blocking ask. Two actors that each wait for a message the other never sends is still possible: give waits a deadline. |

## E. Time

| Idiom | Verdict | How |
| --- | --- | --- |
| Timeout or deadline as a message, with old timers ignored | Example | A delayed self-send; keep its id in state and cancel it to replace it; a generation number refuses an old timeout (`examples/src/timer.rs`). |
| Recurring work | Built in | `cicada.scheduler`; `examples/src/cron.rs` for work inside the actor's own turn. |
| Debounce, rate-shape, batch | Built in | `cicada.debouncer`, `cicada.rate-limiter`, `cicada.batcher`. |
| Notice that something did not happen | Built in | `cicada.heartbeat-watch`: one alarm per silence. The alarm is a suspicion, not proof. |
| Long wait without holding anything open | Built in | A delayed message or `step.sleep`. |
| Human in the loop | Example | `examples/src/flow_approval.rs`: ask, wait a week, time out. Refusing an approval of an older version is by hand. |
| Time and randomness that replay the same | Built in | `turn.now()` is fixed per turn; in a flow, clocks and randomness go inside steps. |

## F. Failure

| Idiom | Verdict | How |
| --- | --- | --- |
| Business failure as data, fault as failure | Built in | A reply variant such as `QuoteRejected` is data. `Err` is a fault; `turn.fail(..)` refuses a message for good. |
| Let it crash, back to a known state | Built in | A failed turn commits nothing: the state is always the last committed one. |
| Watch another actor (monitor) | Built in | `turn.watch` gives `Failed` and `Stopped` as ordinary messages. `docs/guide/core.md#watches` |
| Supervisor that restarts a child | Built in | `cicada.supervisor`: restart limit, then escalation by failing a message of its own. |
| Restart a group together (one-for-all, rest-for-one) | By hand | The ready-made supervisor restarts one child. A turn can delete and create several actors in one commit. |
| Error kernel: valuable state away from risky work | By hand | A design choice. Note the difference from Erlang: a crash never loses state here; a supervisor restart deletes it on purpose. |
| One retry owner, with a budget | Built in | The sender's `RetryPolicy`, with a number of attempts and a backoff. The runtime adds no retry of its own (G6). `docs/guide/core.md#retry` |
| A restart with a stated meaning | Built in | A failure loses nothing. A supervisor restart resets the child to its configured first state. |
| Parking lot with repair and replay | Built in | `cicada.dead-letters` keeps the message, the error and the cause, and redrives on request. It holds 1,000 letters by default and drops the oldest. |
| Poison message | Built in | The claim limit ends it `failed` after 5 lost claims. |
| Circuit breaker | By hand | Breaker state and a timer in the gateway actor, or a second actor that pauses and resumes it. ADR 0018 lists a ready-made one as "later". |

## G. The outside world

| Idiom | Verdict | How |
| --- | --- | --- |
| Gateway or adapter actor in front of a service | Example | The client is a field of the actor struct; the handler awaits the call (`examples/src/sync.rs`, the payments of the saga). |
| Idempotency key for an outside call | Built in | `turn.message().id` is the same on every re-run; a flow step gets `<run address>/<step id>`. |
| The effect happened but was not recorded | Built in | The same message runs again with the same id, and `attempt > 1` says an earlier run may have got part-way. Put separate turns between effects (`examples/src/steps.rs`). |
| Outbox and inbox | Built in | Commit the state and a message to the gateway actor together; the gateway makes the call at least once. Completing a message and the change it caused are one commit too. |
| Effect record with an "unknown" state | By hand | States in the actor's state: planned, sent, unknown, confirmed. |
| A write to the application's own database | By hand | It is not part of the turn's commit: treat it as an outside effect, for example an upsert keyed by the message id. |
| Reconcile with the outside system | By hand | On a re-run, ask the outside system by key before acting. |
| Inbound webhook | Built in | A send over HTTP with the provider's event id as message id: a repeat is refused. For events out of order, carry the source's version and compare it in the handler, by hand. |
| Poll an outside system | Example | `examples/src/sync.rs`: one page per turn. The cursor and the next step commit together, so no page is skipped. |
| Consume a change feed or a broker | By hand | A task in the host reads the feed and sends messages with stable ids. |
| Quota or rate limit toward a service | Built in, with a limit | One named rate limiter per real allowance. It spaces due times and does not cap calls that run at once: enforce a strict quota at the call site. |
| Slow or unavailable service | Built in | A turn time limit and the sender's retry. One slow call holds only that actor's queue. |
| Device twin that observes | Example | One actor per device (`examples/src/registry.rs`) and a heartbeat watch for silence. |
| Human task | Example | `examples/src/flow_approval.rs`. |
| A turn that acts on an OS process or a container | **Gap** | Works with one hosting node; many machines need `pinned-actors`. Way 1 below. |
| A turn that acts on a hardware device on one machine | **Gap** | The same. |
| Risky foreign code as a watched worker | **Gap** | The same: the worker is a child program. |
| A live connection (socket, session) | By hand | The node that holds the socket follows the session actor's state and pushes; inbound frames are messages. Way 2 below. |

## The one real gap: a turn that acts on a thing on one machine

Erlang's rule is "one process owns each port, and everything goes through the owner". A Cicada actor cannot be
that owner in the same way, for two reasons that are both by design:

1. An actor has no memory between turns, so it cannot hold the live handle.
2. Any node that hosts the type may run the next turn, so the turn may run on a machine where the thing is not.

So the live handle always lives in the host, and the design chooses how the actor reaches it. There are two ways.
If the actor only needs to observe the machine, neither is needed: a task on the machine sends messages (`Beat`,
readings) to an actor that runs anywhere.

**Way 1: the turn uses the handle.** The actor struct holds a table of live handles, keyed by the actor's key.
The state holds what should be true and what was last seen: "running", the command line, the process id, an
incarnation number, the last exit code. A turn looks its handle up and acts on it. A task in the host waits on the
handle and sends what happens (`Exited`, a line of output) to the actor as messages.

- It works today when one node hosts the type.
- With many machines it does not: a type name is a compile-time constant, so "the monitor of host A" and "the
  monitor of host B" cannot be two types, and a turn may run on the wrong machine. This is the gap. `pinned-actors`
  closes it: a create names a node, and only that node runs the actor's turns.
- The design must cover three cases. After a restart of the node the table is empty while the state says
  "running": adopt the process or start it again. A turn that started a process and then did not commit runs
  again: it must find that process, not start a second one. An `Exited` of an old process must not end the new
  one: carry the incarnation number.

**Way 2: the machine follows the actor.** The actor holds only what should be true: "running, with this command
line", or the frames to push, each with a number. A task on the machine that has the handle follows the actor's
state through the change hints, with a full read now and then, acts on the handle, and reports what it sees back
as messages. No turn touches the handle.

- It works today, on any number of machines, without a pin.
- It fits a live connection such as a websocket. The session actor is the lasting identity; the socket is one
  incarnation of it. A reconnect may land on another node, which picks up from the last number the client
  confirmed. A pin fits this badly, because a pin does not move.
- It fits "keep this running". It fits "run this now and give me the output" poorly: that becomes a numbered
  command in the state and a reply as a message.

Both ways belong in the design guidance. `pinned-actors` now asks for a worked example of way 1, an actor that
owns a child process. It is the Cicada form of the Erlang port owner.

## Smaller findings

None of these blocks a design. Each is a place where the skill must tell the designer what to do by hand.

| Finding | Suggestion |
| --- | --- |
| Internal messages cannot be marked private; every variant is in the public contract. | Teach the sender check in the skill. A core change is not needed. |
| A plain actor has no typed reply route; flows have one. | Teach the two forms: a typed ref in the request, or `message().from` with `send_to`. |
| No ready-made circuit breaker. | Teach the two hand-written forms. Build the ready-made one when a consumer asks (ADR 0018). |
| The supervisor restarts one child, not a group. | Teach "delete and create several in one commit" for a group that must restart together. |
| No stash. | Teach "keep it in state, bounded". |
| A state machine is checked per message, not per state and message. | Teach the state table as a review step. |
| A handler cannot write to the application's own tables inside the turn's commit. | Teach "it is an outside effect; key it by the message id". The guides do not say this today. |

## What Cicada rules out, and what stays the designer's job

Cannot be written in Cicada:

- Ask chains and deadlock from call cycles (sends are one-way).
- State touched from a callback or another thread (a handler holds its turn alone).
- Mutable state leaking through a message (messages are JSON values).
- A state change without its outgoing messages, or the reverse (one commit).
- A message that vanishes when its handler crashes (every message reaches exactly one outcome).

Still the designer's job:

- God actors, anaemic actors, and a singleton that all traffic passes through.
- An invariant split over several actors.
- State that grows without bound.
- A database that several actors share behind each other's back.
- Reliance on order across senders, on order after a retry, or on a due time being exact.
- Chatty protocols.
- Outside calls that are not safe to repeat.
