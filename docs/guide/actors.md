# Ready-made actors: `cicada-actors`

Layer 3 of Cicada. [The core guide](core.md) explains actors, watches and control effects, which these actors use;
the worked example [examples/src/supervision.rs](../../examples/src/supervision.rs) puts a supervisor and dead letters
to work.

| Crate | Contents |
| --- | --- |
| `cicada-actors` | Ready-made actors on the public API, each under a fixed type name: a supervisor (`SupervisorActor`), dead letters (`DeadLettersActor`) and a topic (`TopicActor`); `StoredMessage` and `Schedule`. |

The Rust blocks of this guide are doctests of the `examples` crate (`cargo test -p cicada-examples --doc`).

`cicada-actors` holds actors that use only the public API of the core (ADR 0018, ADR 0028). Each has a fixed type name;
a node hosts it like any other type, and the host creates its instances.

| Actor | Type name | Key | What it does |
| --- | --- | --- | --- |
| `SupervisorActor` | `cicada.supervisor` | any name | Starts its children, watches them, and restarts one that is at fault. |
| `DeadLettersActor` | `cicada.dead-letters` | any name, `_` by convention | Keeps failed messages, told of them by a watch; an operator redrives or discards one. |
| `TopicActor` | `cicada.topic` | the name of the topic | Sends each published message to all of its subscribers. |

A message that one of them keeps to send later is a `StoredMessage`: a type name and a JSON payload,
`{"type": …, "payload": …}`. The actor knows where it goes, so it holds no address. `StoredMessage::for_ref(&worker,
WorkerMsg::Begin { batch: 7 })` builds one from a typed ref and a message, so a message of the wrong family does not
compile. The contracts of the three types are wire formats; their references in
[docs/reference/contracts](../reference/contracts/) pin them, doc comments included.

Their states and messages are checked like any other, against the depth limit of 64 levels too
([How deep a value can be](core.md#how-deep-a-value-can-be)). As they hold values three or four levels down, they keep
values of at most 60 to 62 levels; each section says how much, and what to do about a deeper value stored before
0.16.0.

## Supervisor

**A restart empties the mailbox of the child.** Its pending messages end `purged`, no dead letter is written for them,
and the child gets the state of its entry again. So supervise only actors whose state can be built again from the state
of their entry. Dead letters and a topic are not such actors: a restart would lose their letters, their subscribers and
the watch that was set for them from outside. A supervisor may be supervised.

<!-- doctest
# use cicada::prelude::*;
# use serde::{Deserialize, Serialize};
# #[derive(Debug, Clone, Serialize, Deserialize)]
# struct Worker { id: String }
# impl ActorRef for Worker { const TYPE: &'static str = "worker"; type Message = WorkerMsg; }
# #[derive(Debug, Serialize, Deserialize, JsonSchema)]
# #[serde(tag = "type")]
# enum WorkerMsg { Begin { batch: u32 } }
# #[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
# struct WorkerState { done: u32 }
# #[derive(Debug, Clone, Serialize, Deserialize)]
# struct Mailer;
# impl ActorRef for Mailer { const TYPE: &'static str = "mailer"; type Message = MailerMsg; }
# #[derive(Debug, Serialize, Deserialize, JsonSchema)]
# #[serde(tag = "type")]
# enum MailerMsg { Send { to: String } }
@@2
# async fn start(system: ActorSystem, worker: Worker, mailer: Mailer) -> Result<(), Box<dyn std::error::Error>> {
@@7
# Ok(())
# }
-->
```rust
use cicada::NoState;
use cicada_actors::{Child, Supervisor, SupervisorMsg, SupervisorState};

let supervisor = Supervisor::named("imports"); // hosted as SupervisorActor
let state = SupervisorState::new()
    .child(Child::new(&worker, &WorkerState::default())?.start(WorkerMsg::Begin { batch: 7 })?)
    .child(Child::new(&mailer, &NoState {})?);
system.create(&supervisor, &state).await?;
system.send(&supervisor, SupervisorMsg::Start).await?;
```

- **Start.** `Start` starts each child of the state that is not running, each in a turn of its own (`StartChild`): it
  creates the child as its own child with the state of its entry (`turn.create_child_raw`), watches it, and sends it
  its start message. A child that exists already, under another parent or with none, fails its own start with the
  cause `commit`, and the others start. A start is tried again for 63 seconds (7 attempts, the waits doubling from one
  second), so a type whose node is still starting is found; after that the supervisor gives the child up. A second
  `Start` starts what the first could not, ends a restart whose `Restarted` was purged, and takes back a child that
  was given up and still runs. The children are those of the state; a host that needs another set creates another
  supervisor.
- **What a failure of a child does**, by the cause of its `Failed`:

  | Cause | The supervisor |
  | --- | --- |
  | `handler`, `timeout` | restarts the child: the child is at fault |
  | `state` | gives the child up and keeps its state, and escalates without a restart above |
  | `refused`, `message` | does nothing: the message is at fault |
  | `commit` | does nothing: another actor is missing, or somebody made a mistake |
  | `node` | does nothing: the store or the node failed, not the child |
  | `claim-limit` | does nothing: the message that killed a node may be another one |

  A child whose state its type cannot read is given up, also when a restart would have helped: mostly it comes from a
  deploy that reads states more strictly, and a restart would reset every supervised actor of that type. The rule is
  fixed, and there is one strategy: restart the one child.
- **One incident is one restart.** A restart is one commit: it deletes the child, creates it again, sends it its start
  message and sends the supervisor `Restarted`. Until that `Restarted` is handled, the supervisor ignores failures of
  the child, so a child with five failed messages in its backlog is restarted once. This needs no clock, but on
  PostgreSQL a step back of the database's clock can turn one incident into two restarts. A short outage of the store
  while the supervisor handles a failure loses that restart; the child keeps running until it fails again.
- **The restart limit.** At most `maxRestarts` restarts of one child within `withinSeconds`: 3 in 60 seconds, or
  `SupervisorState::restart_limit`. One failure more gives the child up (it keeps running, without a supervisor) and
  fails a message of the supervisor as a handler fails, with the error `restart limit: <child> failed <n> times in
  <within>: <error>`: a supervisor above restarts this one, with its whole tree. A child that cannot be started, or
  whose state cannot be read, fails a message with `cannot start <child>: …` or `cannot read <child>: …` that is
  refused for good: nobody restarts for it, and dead letters and the watchers of the supervisor see it. A `Start`
  takes such a child back.
- **A child that goes away.** A child that stops itself, or that somebody else deletes, is forgotten, also while a
  restart of it is under way; it may handle its start message once more before the supervisor deletes that new life.
  A child that cannot exist (its type was removed, its entry is wrong) cannot be taken away while its supervisor
  lives: each `Start`, and each restart from above, tries it for 63 seconds and escalates again. A child that stopped
  itself comes back when a supervisor above restarts its supervisor.
- **Its own messages.** `StartChild`, `Restarted` and `Escalate` count only when the supervisor sent them itself: from a
  client, or redriven from dead letters, they end `done` and change nothing. A `Failed` or a `Stopped` counts only
  when the store wrote it as a notification, so a client cannot make a supervisor restart or forget a child.
- **Where to host it.** Host a supervisor on nodes that host the types of its children. Such a node checks an entry
  when the child starts: an address or a state the type cannot read is refused, and the child is given up at once. On
  another node a wrong entry is created, its child fails with the cause `state`, and the supervisor gives it up then.
  The state of a child and its start message are stored as JSON: when the type of the child changes so that they no
  longer fit, a restart deletes the child and cannot create it again, and the supervisor gives it up.

- **How deep.** An entry holds the state of its child three levels into the state of the supervisor, and the payload of
  its start message four. So a child state deeper than 61 levels, or a start payload deeper than 60, makes the state too
  deep, and the create of the supervisor is refused. A supervisor whose state, stored before 0.16.0, holds such an
  entry fails every turn that writes its state (`State cannot be stored: The state is nested deeper than 64 levels`).
  Delete it, which deletes its children too, and create it again with a shallower state: its `Start` starts them
  again.

A supervisor is one actor: it handles the failures of its children one after the other.

## Dead letters

`DeadLettersActor` keeps failed messages as letters. The host creates the actor and sets its watch: on every type, or
on the types it cares for. It is told of each failure by a `Failed` in its mailbox, in the transaction of the failure,
with no timer:

<!-- doctest
@@3
# async fn node(store: cicada::Store) -> Result<(), Box<dyn std::error::Error>> {
@@4
# Ok(())
# }
-->
```rust
use cicada::{ActorRef, ActorSystem, WatchTarget};
use cicada_actors::{DeadLetters, DeadLettersActor, DeadLettersState};

let system = ActorSystem::builder(store).host(DeadLettersActor::with_capacity(1_000)? /* , … */).build();
system.start().await?;
system.create(&DeadLetters::default(), &DeadLettersState::default()).await?;
system.watch(&WatchTarget::Every, &DeadLetters::default().address()?).await?;
```

Over HTTP and on the command line the watch is `PUT /watches` or `cicada create watch --target '*' --watcher
cicada.dead-letters/_`. A letter holds the failed message (its actor, type, payload, correlation id and sender), its
error, its cause, its attempts, when it was recorded, and the `seq` of the notification that told of it, which orders
letters recorded at the same store time. The actor keeps at most its capacity of letters, 1,000 unless
built with another, and drops the oldest first. Its own failures are no letters, and neither is a `Failed` that a client
sent: only a notification counts.

- `Redrive { messageId }` sends the failed message again, once: to the same actor, with the same type, payload and
  correlation id, from the dead-letters actor. The letter holds the id of the new message (`redrivenAs`), and a message
  that fails again is a letter of its own. When the type of the actor no longer takes the message, nothing is sent and
  the letter says why (`redriveError`). When the actor is gone, the `Redrive` itself ends `failed` with the cause
  `commit`, and the letter stays as it was.
- `Discard { messageId }` forgets a letter. Over HTTP: `POST /actors/cicada.dead-letters/_/messages` with
  `{"type": "Redrive", "payload": {"messageId": "…"}}`.
- **How deep.** A letter holds the payload three levels into the state, so a failed message whose payload is deeper
  than 61 levels cannot be kept: the turn that would record it fails with the cause `handler` (`State cannot be stored:
  The state is nested deeper than 64 levels`), and no letter is recorded. That failure is a notification, which tells
  nobody: the outcome of the notification (`GET /outcomes?status=failed&prefix=cicada.dead-letters/`) holds the error
  and the id of the failed message, for as long as outcomes are kept. The next failure is recorded as before.
- **A deep letter from before 0.16.0.** Dead letters that hold a letter whose payload is deeper than 61 levels fail
  every turn that writes their state: no later failure is recorded, and a `Redrive` or a `Discard` of another letter
  fails. `Discard` of the deep letter itself works, as the state is within the limit without it; deleting and creating
  the actor works too, and drops all its letters.

## Topic

`TopicActor` keeps subscribers and sends each published message to all of them, each in a turn of its own:

<!-- doctest
# use cicada::prelude::*;
# use serde::{Deserialize, Serialize};
# #[derive(Debug, Serialize, Deserialize, JsonSchema)]
# #[serde(tag = "type")]
# enum OrderMsg { Shipped { id: u32 } }
@@2
# async fn publish(system: ActorSystem, shipping: ActorAddress) -> Result<(), Box<dyn std::error::Error>> {
@@6
# Ok(())
# }
-->
```rust
use cicada::SendOptions;
use cicada_actors::{Topic, TopicMsg, TopicState};

let orders = Topic::named("orders"); // hosted as TopicActor
system.create(&orders, &TopicState::default()).await?;
system.send(&orders, TopicMsg::Subscribe { subscriber: shipping }).await?;
let shipped = TopicMsg::publish(OrderMsg::Shipped { id: 7 })?; // any message of a family
system.send_with(&orders, shipped, SendOptions::new().correlation_id("c1")).await?;
```

- `Publish` makes one `Deliver` for each subscriber, which the topic sends itself and which carries the correlation id
  of the publish. A subscriber gets the messages in the order they were published, unless a delivery was tried again.
  A `Deliver` from anybody else, or for an address that is no longer a subscriber, sends nothing.
- A subscriber that is gone fails its own delivery and nobody else's (the cause `commit`). The topic watches itself,
  not its subscribers: it is told of that failure and drops the subscriber. A `Failed` that a client sent drops
  nobody.
- A delivery whose send is refused at the call (no registered type for the address, or a type that does not take the
  message) is refused for good, sends nothing, and keeps the subscriber; an address that never gets a type costs one
  failed `Deliver` for each publish until it is unsubscribed. A delivery whose read of the store fails is tried again,
  as a start of a supervisor is.
- One turn for each subscriber and each publish: a topic with many subscribers is slow.
- **How deep.** A `Publish` and a `Deliver` hold the published message two levels down, so a payload deeper than 62
  levels makes the `Publish` too deep: the send of the publisher is refused at the call. A `Publish` stored before
  0.16.0 with such a payload fails, as its `Deliver` is refused at the call; the topic goes on with the next message.

## Schedules

`cicada_actors::Schedule` (`Schedule::every(seconds)`, aligned to the epoch, and `Schedule::daily(hour, minute)`,
UTC) gives a cron actor its next slot; it is serializable, so it can live in state. A cron actor arms its next tick
as a timer ([Timers](core.md#timers) in the core guide). See [examples/src/cron.rs](../../examples/src/cron.rs) for
the `Tick` chain:

```rust
use cicada::time::Timestamp;
use cicada_actors::Schedule;

let t: Timestamp = "2026-10-02T10:15:30Z".parse().unwrap();
assert_eq!(Schedule::every(3600).unwrap().next(t), "2026-10-02T11:00:00Z".parse::<Timestamp>().unwrap());
assert_eq!(Schedule::daily(2, 30).unwrap().next(t), "2026-10-03T02:30:00Z".parse::<Timestamp>().unwrap());
```
