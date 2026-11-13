# Ready-made actors: `cicada-actors`

Layer 3 of Cicada. [The core guide](core.md) explains actors, watches and control effects, which these actors use;
the worked example [examples/src/supervision.rs](../../examples/src/supervision.rs) puts a supervisor and dead letters
to work.

| Crate | Contents |
| --- | --- |
| `cicada-actors` | Ready-made actors on the public API, each under a fixed type name: a supervisor (`SupervisorActor`), dead letters (`DeadLettersActor`), a topic (`TopicActor`) and a scheduler (`SchedulerActor`); `StoredMessage` and `Schedule`. |

The Rust blocks of this guide are doctests of the `examples` crate (`cargo test -p cicada-examples --doc`).

`cicada-actors` holds actors that use only the public API of the core (ADR 0018, ADR 0028). Each has a fixed type name;
a node hosts it like any other type, and the host creates its instances.

| Actor | Type name | Key | What it does |
| --- | --- | --- | --- |
| `SupervisorActor` | `cicada.supervisor` | any name | Starts its children, watches them, and restarts one that is at fault. |
| `DeadLettersActor` | `cicada.dead-letters` | any name, `_` by convention | Keeps failed messages, told of them by a watch; an operator redrives or discards one. |
| `TopicActor` | `cicada.topic` | the name of the topic | Sends each published message to all of its subscribers. |
| `SchedulerActor` | `cicada.scheduler` | the name of the schedule | Sends a stored message to a target on each slot of a `Schedule`. |

A message that one of them keeps to send later is a `StoredMessage`: a type name and a JSON payload,
`{"type": …, "payload": …}`. The actor knows where it goes, so it holds no address. `StoredMessage::for_ref(&worker,
WorkerMsg::Begin { batch: 7 })` builds one from a typed ref and a message, so a message of the wrong family does not
compile. The contracts of the four types are wire formats; their references in
[docs/reference/contracts](../reference/contracts/) pin them, doc comments included.

Their states and messages are checked like any other, against the depth limit of 64 levels too
([How deep a value can be](core.md#how-deep-a-value-can-be)). As they hold values two to four levels down, they keep
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

## Scheduler

`SchedulerActor` sends a stored message to a target on each slot of a `Schedule`, one instance for each schedule. The
type has no settings: the schedule, the target, the message and a retry policy for the delivered message are in the
state, so an operator creates and edits a schedule at runtime, over HTTP or on the command line, with no deploy.

<!-- doctest
# use cicada::prelude::*;
# use serde::{Deserialize, Serialize};
# #[derive(Debug, Clone, Serialize, Deserialize)]
# struct Reports;
# impl ActorRef for Reports { const TYPE: &'static str = "reports"; type Message = ReportsMsg; }
# #[derive(Debug, Serialize, Deserialize, JsonSchema)]
# #[serde(tag = "type")]
# enum ReportsMsg { Build { full: bool } }
@@4
# async fn schedule(system: ActorSystem) -> Result<(), Box<dyn std::error::Error>> {
@@5
# Ok(())
# }
-->
```rust
use std::time::Duration;
use cicada::retry::{Backoff, RetryPolicy};
use cicada_actors::{Schedule, Scheduler, SchedulerMsg, SchedulerState};

let nightly = Scheduler::named("nightly-report"); // hosted as SchedulerActor
let mut state = SchedulerState::new(Schedule::daily(2, 0)?, &Reports, ReportsMsg::Build { full: true })?;
state.retry = Some(RetryPolicy::new(3, Backoff::fixed(Duration::from_secs(300))?)?);
system.create(&nightly, &state).await?;
system.send(&nightly, SchedulerMsg::Start).await?;
```

`SchedulerState::new` takes a typed ref and a message of its family, so a message the target does not take does not
compile. Over HTTP a schedule takes two calls, a create and a `Start` (a turn does both in one commit, as its creates
come before its sends):

```sh
curl -X PUT localhost:8080/actors/cicada.scheduler/nightly-report -H 'content-type: application/json' -d '{"state":
  {"schedule": {"type": "daily", "hour": 2, "minute": 0}, "target": "reports/_",
   "message": {"type": "Build", "payload": {"full": true}},
   "retry": {"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "PT5M"}}}}'
curl -X POST localhost:8080/actors/cicada.scheduler/nightly-report/messages -H 'content-type: application/json' \
  -d '{"type": "Start"}'
```

The state also holds `nextRun` (the slot of the armed tick), `armedTick` (its id), `runs` (the deliveries that reached
the target's mailbox) and `lastRun` (the store's time of the run that sent the last of them). A scheduler is
**started** when `armedTick` is set; no slot delivers before a `Start`.

- **Its messages.** `Start` arms a tick at the next slot; it is safe to repeat, as one tick stays armed. `Run` sends the
  stored message to the target now and keeps the armed tick, also on a scheduler that was never started. `Update`
  replaces each of `schedule`, `target`, `message` and `retry` that it gives and keeps the counts, as
  `{"type": "Update", "payload": {"message": {"type": "Build", "payload": {"full": false}}}}`. A new schedule on a
  started scheduler cancels the armed tick and arms the new schedule's next slot; a new target, message or retry policy
  counts from the next `Run`, also one that is already waiting. Pause, resume and delete a scheduler with the core's
  control: there are no messages for them.
- **Each slot takes two turns.** The `Tick` arms the next tick and sends the scheduler a `Run` with its slot; the
  `Run` reads the target, the message and the retry policy from the state and sends the message. The `Tick` is the
  scheduler's own: from anybody else, or when it is not the armed tick, it ends `done` and changes nothing. A schedule
  every second is two turns a second.
- **What the target gets.** The stored message, with the scheduler as its sender, no correlation id and a new id for
  each run. It carries the scheduler's retry policy (`retry`), so a failed run of the target is tried again after its
  backoff instead of waiting for the next slot; with none, or a policy of one attempt, a failed run ends `failed` at
  the target. A `Run` that commits puts exactly one message into the target's mailbox; the target handles it at least
  once. A target slower than its schedule never runs two deliveries at once: they wait in its mailbox, and none is
  skipped.
- **The slot.** A run for a slot carries the header `cicada-slot`: the slot in UTC to the second, as
  `2026-11-13T02:00:00Z`. The target knows which slot it handles also when the tick was late. The name and the form of
  the header are stable. The payload is the same on every slot, so a target keys its work by the slot, or by the
  message id:

<!-- doctest
# use cicada::prelude::*;
# fn key<A: Actor>(turn: &Turn<A>) -> String {
@@4
# key
# }
-->
```rust
let key = match turn.message().headers.get("cicada-slot") {
    Some(slot) => slot.clone(), // "2026-11-13T02:00:00Z": a run for a slot
    None => turn.message().id.as_str().to_owned(), // a run now, which has no slot
};
```

- **A run now.** A client sends `{"type": "Run"}`, without a slot: the slot belongs to the scheduler, and a target that
  keys its work by the slot would drop the real run of a slot that a client made up. Such a run has no `cicada-slot`
  header. A `Run` just before a slot gives two deliveries close together.
- **Missed slots are skipped.** A tick arms the next one at the first slot after the later of the store's time and
  its own slot. Slots missed while no node ran, or while the scheduler was paused, are skipped: at the resume a late
  tick delivers once, and the next tick goes to the next slot from then. A tick that an operator makes due early
  (`deliver-now`) delivers its slot once, and the next tick goes to the slot after it.
- **A failed delivery does not end the chain.** A target that is gone fails that `Run` with the cause `commit`; a
  message its type does not take fails it with `refused`; the next slot delivers as usual. A `Run` whose read of the
  store fails is tried again (7 attempts, the waits doubling from one second). A failed `Run` is a failed message of
  the scheduler: its watchers are told, dead letters that watch it keep it, and an operator's retry or a redrive of
  the dead letters delivers it again. The scheduler records no failure in its state and watches nothing.
- **A target that is gone fails every slot** until an operator pauses the scheduler, deletes it, or creates the target
  again; the scheduler does not stop by itself. With `every(60)` that is 1,440 failed messages a day, and dead letters
  that watch the scheduler keep 1,000 by default and drop the oldest first: one forgotten scheduler pushes other
  failures out within a day. Pause or delete such a scheduler.
- **How a chain ends.** A chain ends when its armed tick goes without running: a purge of the scheduler's mailbox, an
  operator's cancel of the tick, or a tick that fails for good (the claim limit). Only the last tells the scheduler's
  watchers, and `armedTick` stays set in each case. A retry or a redrive of a failed tick ends `done` and arms
  nothing. `Start`, or an `Update` with a schedule, arms the chain again.
- **A scheduler that runs itself**, with itself as its target and `Run` as its message, runs without end until it is
  paused. It is not refused; do not set one up.
- **How deep.** The state holds the payload of its message two levels down, and so does an `Update` that sets a
  message. A payload deeper than 62 levels is refused: at the create of the scheduler (`400` over HTTP,
  `OpError::Invalid` from code), and at the send of the `Update`.

### Schedule

`cicada_actors::Schedule` (`Schedule::every(seconds)`, aligned to the epoch, and `Schedule::daily(hour, minute)`,
UTC) gives the scheduler, or a cron actor of the host, its next slot; it is serializable, so it can live in state. A
cron actor that does host work in its own turn arms its next tick as a timer ([Timers](core.md#timers) in the core
guide); [examples/src/cron.rs](../../examples/src/cron.rs) is that `Tick` chain. Cron expressions and time zones are
not supported:

```rust
use cicada::time::Timestamp;
use cicada_actors::Schedule;

let t: Timestamp = "2026-10-02T10:15:30Z".parse().unwrap();
assert_eq!(Schedule::every(3600).unwrap().next(t), "2026-10-02T11:00:00Z".parse::<Timestamp>().unwrap());
assert_eq!(Schedule::daily(2, 30).unwrap().next(t), "2026-10-03T02:30:00Z".parse::<Timestamp>().unwrap());
```
