# Ready-made actors: `cicada-actors`

Layer 3 of Cicada. [The core guide](core.md) explains actors, watches and control effects, which these actors use;
the worked example [examples/src/supervision.rs](../../examples/src/supervision.rs) puts a supervisor and dead letters
to work.

| Crate | Contents |
| --- | --- |
| `cicada-actors` | Ready-made actors on the public API, each under a fixed type name: a supervisor (`SupervisorActor`), dead letters (`DeadLettersActor`), a topic (`TopicActor`), a scheduler (`SchedulerActor`), a rate limiter (`RateLimiterActor`), a debouncer (`DebouncerActor`) and a batcher (`BatcherActor`); `StoredMessage` and `Schedule`. |

The Rust blocks of this guide are doctests of the `examples` crate (`cargo test -p cicada-examples --doc`).

`cicada-actors` holds actors that use only the public API of the core (ADR 0018, ADR 0028). Each has a fixed type name;
a node hosts it like any other type, and the host creates its instances.

| Actor | Type name | Key | What it does |
| --- | --- | --- | --- |
| `SupervisorActor` | `cicada.supervisor` | any name | Starts its children, watches them, and restarts one that is at fault. |
| `DeadLettersActor` | `cicada.dead-letters` | any name, `_` by convention | Keeps failed messages, told of them by a watch; an operator redrives or discards one. |
| `TopicActor` | `cicada.topic` | the name of the topic | Sends each published message to all of its subscribers. |
| `SchedulerActor` | `cicada.scheduler` | the name of the schedule | Sends a stored message to a target on each slot of a `Schedule`. |
| `RateLimiterActor` | `cicada.rate-limiter` | the name of the limit | Sends each submitted message to its target at the next free slot of a shared rate. |
| `DebouncerActor` | `cicada.debouncer` | the key whose bursts it debounces | Keeps the latest submission of a burst and sends it to its target once the burst is quiet. |
| `BatcherActor` | `cicada.batcher` | the stream or group whose items it batches | Collects items and sends them to its target as one message when the batch is full or has waited long enough. |

A message that one of them keeps to send later is a `StoredMessage`: a type name and a JSON payload,
`{"type": …, "payload": …}`. The actor knows where it goes, so it holds no address. `StoredMessage::for_ref(&worker,
WorkerMsg::Begin { batch: 7 })` builds one from a typed ref and a message, so a message of the wrong family does not
compile. The contracts of the seven types are wire formats; their references in
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

## Rate limiter

`RateLimiterActor` gives many senders one shared rate: it sends each submitted message to its target at the next free
slot, one instance for each limit. It shapes when messages become due; it does not enforce an outside API's quota at
the time of the call, and nothing asks it "may I?". The rate, the burst and the longest delay are in the state, fixed
at the create. A host creates its shared limiters at startup:

<!-- doctest
# use cicada::prelude::*;
# async fn start(system: ActorSystem) -> Result<(), Box<dyn std::error::Error>> {
@@6
# Ok(())
# }
-->
```rust
use std::time::Duration;
use cicada_actors::{RateLimiter, RateLimiterState};

let sms = RateLimiter::named("sms"); // hosted as RateLimiterActor
// One text a second, five at once after idle time, and none that waits longer than a minute.
system.create(&sms, &RateLimiterState::new(1, Duration::from_secs(1), 5, Duration::from_secs(60))?).await?;
```

A producer can also create the limiter and submit to it in one turn, as a turn's creates come before its sends. The
limiter is the sender of what it forwards, so a reply address belongs in the payload; the retry policy of the message
at the target is chosen in the `Submit`. [examples/src/rate_limit.rs](../../examples/src/rate_limit.rs) is this
signup:

<!-- doctest
# use cicada::prelude::*;
# use serde::{Deserialize, Serialize};
# #[derive(Debug, Clone, Serialize, Deserialize)]
# struct Sms;
# impl ActorRef for Sms { const TYPE: &'static str = "sms"; type Message = SmsMsg; }
# #[derive(Debug, Serialize, Deserialize, JsonSchema)]
# #[serde(tag = "type")]
# enum SmsMsg { Send { phone: String, text: String, reply_to: ActorAddress } }
@@3
# fn register<A: Actor>(turn: &mut Turn<A>, phone: String, rate: RateLimiterState) -> HandlerResult {
@@5
# Ok(())
# }
-->
```rust
use std::time::Duration;
use cicada_actors::{RateLimiter, RateLimiterMsg, RateLimiterState};

let sms = RateLimiter::named("sms");
turn.create(&sms, &rate)?; // a limiter that exists keeps its settings and reservations
let text = SmsMsg::Send { phone, text: "Welcome!".into(), reply_to: turn.self_address().clone() };
let retry = RetryPolicy::new(3, Backoff::fixed(Duration::from_secs(5))?)?; // at the target
turn.send(&sms, RateLimiterMsg::submit(&Sms, text, Some(retry))?)?;
```

`RateLimiterMsg::submit` takes a typed ref and a message of its family, so a message the target does not take does not
compile. Over HTTP (the example's sender writes its fields in camelCase):

```sh
curl -X PUT localhost:8080/actors/cicada.rate-limiter/sms -H 'content-type: application/json' -d '{"state":
  {"count": 1, "period": "PT1S", "burst": 5, "maxDelay": "PT1M"}}'
curl -X POST localhost:8080/actors/cicada.rate-limiter/sms/messages -H 'content-type: application/json' -d '{"type":
  "Submit", "payload": {"to": "sms/_", "message": {"type": "Send", "payload": {"phone": "+31 6 1", "text": "Hi",
  "replyTo": "signups/1"}}}}'
```

- **Its settings.** `count` messages per `period`; `burst`, the messages that may be due at once after enough idle
  time, the first included (1 spaces every message); `maxDelay`, the longest time from handling a submission to its
  slot. Count and burst are positive, the period is positive, and the durations, and the burst's span of
  `(burst - 1)` intervals, are at most `MAX_DELAY`; the constructor and the decoder refuse anything else. No message
  changes them: replace the limiter (below).
- **Its slots.** The limiter uses the generic cell rate algorithm (GCRA): the interval between slots is the period
  divided by the count, rounded up to a whole microsecond (3 a second are 333,334 µs apart), and the state keeps one
  timestamp, `idleAt`, when the full burst is free again. Idle time gives the burst back. At 2 a second with a burst
  of 2 and a longest delay of one second, five submissions at once get now, now, +0.5 s and +1 s, and the fifth is
  refused.
- **The longest delay.** A slot exactly `maxDelay` away is taken; one further is refused for good (the cause
  `refused`), so it reaches dead letters that watch the limiter. A `maxDelay` of zero takes only capacity that is free
  now. The delay counts from the store's time when the turn that handles the submission was claimed: the time the
  submission waited in the limiter's mailbox, and the time its commit takes, are not in it.
- **What the target gets.** The stored message, due at its slot, with a new id, the limiter as its sender, the
  correlation id of the submission and the default retention. Headers of the submission are not copied; an observer
  still adds its own. Its retry policy is the `retry` of the `Submit`, none by default; the retry policy of the
  submission's own envelope governs only the limiter's turn. A retry at the target goes after its own backoff,
  without passing the limiter again.
- **A failed forward reserves nothing.** The slot and the forwarded message commit in one turn. A slot beyond the
  longest delay or past `MAX_INSTANT`, or a message the target does not take, is refused for good; a target that is
  gone rejects the commit (`commit`); a read of the store that fails is tried by the submission's own retry policy.
  An operator's retry or a redrive of the dead letters handles the `Submit` again, at a slot of that turn's store
  time. A repeated message id follows the core's dedup rules.
- **A forwarded message keeps its slot.** A failure at the target, or a cancel of the forwarded message, gives no slot
  back.
- **Slots are due times, not runs.** Messages that wait past their slots, behind a paused or slow target, or after an
  outage, can run close together. A slot is clamped to the store's time at the commit, so a slow commit of the limiter
  can narrow the spacing between the stored due times too. The limiter does not enforce an outside API's quota; a
  worker that must never exceed one checks it at the call.
- **Replacing a limiter.** Deleting a limiter and creating it with new settings starts its reservations fresh;
  messages it forwarded keep their due times in their targets' mailboxes. Producers that share a key must agree on the
  settings, as a create of a limiter that exists changes nothing.
- **How deep.** A `Submit` holds the payload of its message two levels down: a payload deeper than 62 levels makes the
  send of the submission too deep, and it is refused at the call.

## Debouncer

`DebouncerActor` turns a burst of submissions into one delivery: it keeps only the latest submission, one instance
for each key, and sends it to its target once the burst was quiet for a while, at the latest a fixed time after the
burst began, or at once on a `Flush`. Use it for "re-index after edits" or "one notice after a run of changes". It
delivers the trailing value only: it merges no payloads and has no leading edge. The quiet period and the longest
wait are in the state, fixed at the create. A host creates a debouncer at startup, or a producer creates it and
submits to it in one turn, as a turn's creates come before its sends:

<!-- doctest
# use cicada::prelude::*;
# use serde::{Deserialize, Serialize};
# #[derive(Debug, Clone, Serialize, Deserialize)]
# struct Indexer;
# impl ActorRef for Indexer { const TYPE: &'static str = "indexer"; type Message = IndexerMsg; }
# #[derive(Debug, Serialize, Deserialize, JsonSchema)]
# #[serde(tag = "type")]
# enum IndexerMsg { Reindex { doc: String, version: u32, text: String, reply_to: ActorAddress } }
@@3
# fn edit<A: Actor>(turn: &mut Turn<A>, doc: String, version: u32, text: String) -> HandlerResult {
@@7
# Ok(())
# }
-->
```rust
use std::time::Duration;
use cicada_actors::{Debouncer, DebouncerMsg, DebouncerState};

let debouncer = Debouncer::named(format!("reindex-{doc}")); // hosted as DebouncerActor
// Two seconds quiet, at most ten seconds from the first submission of a burst.
let settings = DebouncerState::new(Duration::from_secs(2), Duration::from_secs(10))?;
turn.create(&debouncer, &settings)?; // a debouncer that exists keeps its settings and its burst
let reindex = IndexerMsg::Reindex { doc, version, text, reply_to: turn.self_address().clone() };
let retry = RetryPolicy::new(3, Backoff::fixed(Duration::from_secs(5))?)?; // at the target
turn.send(&debouncer, DebouncerMsg::submit(&Indexer, reindex, Some(retry))?)?;
```

[examples/src/debounce.rs](../../examples/src/debounce.rs) is this document. `DebouncerMsg::submit` takes a typed ref
and a message of its family, so a message the target does not take does not compile. Over HTTP (the example's indexer
writes its fields in camelCase):

```sh
curl -X PUT localhost:8080/actors/cicada.debouncer/reindex-d1 -H 'content-type: application/json' -d '{"state":
  {"quiet": "PT2S", "maxWait": "PT10S"}}'
curl -X POST localhost:8080/actors/cicada.debouncer/reindex-d1/messages -H 'content-type: application/json' -d '{"type":
  "Submit", "payload": {"to": "indexer/_", "message": {"type": "Reindex", "payload": {"doc": "d1", "version": 3,
  "text": "abc", "replyTo": "documents/d1"}}}}'
curl -X POST localhost:8080/actors/cicada.debouncer/reindex-d1/messages -H 'content-type: application/json' \
  -d '{"type": "Flush"}'
```

- **Its settings.** `quiet`, how long a burst must be quiet before its submission is sent, and `maxWait`, the longest
  time from the burst's first submission to its scheduled flush: `0 < quiet <= maxWait <= MAX_DELAY`, which the
  constructor and the decoder check. With `quiet` equal to `maxWait`, a burst is a fixed window from its first
  submission; that is not a rate limit. No message changes the settings: delete the debouncer and create it again.
- **The latest submission wins, whole.** A `Submit` replaces the kept target, message, target retry policy and
  correlation id together. The target is not looked up until the flush, so it need not exist yet. A superseded
  submission ends `done`, and nobody is told that it was dropped.
- **Its timer.** Each `Submit` cancels the armed `Flush` and arms one at `min(now + quiet, firstAt + maxWait)`, where
  `now` is the store's time of the turn and `firstAt` that of the burst's first submission, with the crate's shared
  retry policy (7 attempts, the waits doubling from one second). With `quiet` 2 s and `maxWait` 5 s, submissions
  handled at 0, 1, 2, 3 and 4 s deliver only the last, at 5 s. A due time past `MAX_INSTANT` fails the `Submit` for
  good (`refused`) and changes nothing. A `Submit` and the timer due at the same instant run in the core's order: the
  `Submit` first replaces the kept value; the flush first delivers the old one, and the `Submit` starts a new burst.
- **Flush is public.** A `Flush` sends the kept submission, cancels the armed timer and clears the burst in one commit;
  with nothing kept it ends `done` and does nothing. Anyone may send one, so an operator delivers early, or recovers a
  flush that failed. It sends what is kept when it runs: an old `Flush` that is retried or redriven while a new burst
  waits flushes the new burst early.
- **What the target gets.** The stored message, due now, with a new id, the debouncer as its sender, the kept
  correlation id and retry policy, and the default retention. Headers of the submission are not kept; an observer
  still adds its own to the flush's send. A reply address belongs in the payload. A retry at the target goes after its
  own backoff, without passing the debouncer again; the timer's own retry policy governs only the flush.
- **A failed flush clears nothing.** A message the target does not take, or a target type that is not registered,
  fails the flush for good (`refused`); a target that is gone rejects the commit (`commit`); a read of the store that
  fails is tried again by the flush's retry policy, and a `Submit` in between cancels that retry and arms its own
  timer. Terminal failures reach the debouncer's watchers, and dead letters that watch it keep them; the debouncer
  watches nothing. A failure at the target after the flush committed does not bring the value back.
- **A lost timer.** A flush that failed for good, or that an operator cancelled or purged (which tells no watcher),
  leaves the kept submission without a live timer. A `Flush` sends it; a new `Submit` replaces it and arms a timer from
  the old `firstAt`, due at once when `maxWait` has passed.
- **Due times are not run times.** `maxWait` bounds the scheduled flush from the store's time at which the burst's
  first submission was handled. Time a submission waited in the mailbox, a pause of the debouncer, an outage or a slow
  commit can all make the flush later; due times are rounded up to a microsecond and clamped to the store's time at
  the commit. A paused debouncer flushes when it is resumed and its flush is claimed.
- **It never stops itself.** Submissions may wait in its mailbox; delete an idle debouncer with the core's control.
  Pause, resume and delete work as for any actor; a create of a debouncer that exists changes nothing.
- **How deep.** The state holds the kept payload three levels down, so a payload may be at most 61 levels deep. A
  `Submit` holds it two levels down and is accepted with 62 levels, but keeping it would make the state too deep: that
  turn fails (the cause `handler`) and the earlier burst and its timer stay as they were.

## Batcher

`BatcherActor` collects items, one instance for each stream or group, and sends them to a fixed target as one
message: when the batch is full, at the latest a fixed time after its first item, or at once on a `Flush`. Use it for
targets that work on lists: a bulk API, a digest email, one write for many rows. It keeps items in handling order and
does not merge them. The target, the message type, the count limit, the longest wait and the target's retry policy
are in the state, fixed at the create. A host creates a batcher at startup, or a producer creates it and adds to it in
one turn, as a turn's creates come before its sends:

<!-- doctest
# use cicada::prelude::*;
# use serde::{Deserialize, Serialize};
# #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
# struct AuditEvent { account: String, action: String }
# #[derive(Debug, Clone, Serialize, Deserialize)]
# struct AuditLog;
# impl ActorRef for AuditLog { const TYPE: &'static str = "audit-log"; type Message = AuditLogMsg; }
# #[derive(Debug, Serialize, Deserialize, JsonSchema)]
# #[serde(tag = "type")]
# enum AuditLogMsg { Write { items: Vec<AuditEvent> } }
@@3
# fn rename<A: Actor>(turn: &mut Turn<A>, account: String, action: String) -> HandlerResult {
@@6
# Ok(())
# }
-->
```rust
use std::time::Duration;
use cicada_actors::{Batcher, BatcherMsg, BatcherState};

let batcher = Batcher::named("audit"); // hosted as BatcherActor
// A `Write { items }` to the audit log of at most 50 events, at most five seconds from the first.
let mut settings = BatcherState::new(&AuditLog, "Write", 50, Duration::from_secs(5))?;
settings.retry = Some(RetryPolicy::new(3, Backoff::fixed(Duration::from_secs(5))?)?); // at the target
turn.create(&batcher, &settings)?; // a batcher that exists keeps its settings and its batch
turn.send(&batcher, BatcherMsg::add(AuditEvent { account, action })?)?;
```

[examples/src/batching.rs](../../examples/src/batching.rs) works this out. The target's message holds the batch in a
field `items`, here `Vec<AuditEvent>`, so the target decodes the batch like any message. Over HTTP:

```sh
curl -X PUT localhost:8080/actors/cicada.batcher/audit -H 'content-type: application/json' -d '{"state":
  {"target": "audit-log/_", "messageType": "Write", "maxItems": 50, "maxWait": "PT5S"}}'
curl -X POST localhost:8080/actors/cicada.batcher/audit/messages -H 'content-type: application/json' -d '{"type":
  "Add", "payload": {"item": {"account": "a1", "action": "renamed to x"}}}'
curl -X POST localhost:8080/actors/cicada.batcher/audit/messages -H 'content-type: application/json' \
  -d '{"type": "Flush"}'
```

- **Its settings.** `target`, `messageType`, `maxItems`, from 1 through 1,000, and `maxWait`, the longest time from a
  batch's first item to its scheduled flush, `0 < maxWait <= MAX_DELAY`; with `retry`, the retry policy of each batch
  at the target, none by default. The constructor and the decoder check them, and the message type's length. Whether
  the target type is registered and takes the message type is checked only when a batch is sent. There is no default
  `maxItems`, and no message changes the settings: delete the batcher and create it again.
- **Items, in handling order.** An `Add` holds one item, any JSON value, `null` included; `BatcherMsg::add` builds one
  from any `Serialize` value. The batcher does not check items against the target's schema: the target decodes the
  whole batch. Headers and the correlation id of an `Add` are not kept, so data to reply to or correlate belongs in
  the item.
- **A full batch goes at once.** The `Add` that brings the batch to `maxItems` sends it, with its own item, in its own
  turn, and clears it. So a stored batch always holds fewer than `maxItems` items, even when `Add`s queue up. With
  `maxItems` 1, each `Add` is sent on its own, with no timer.
- **Its timer.** The first item of a batch sets `firstAt`, the store's time of that turn, and arms a `Flush` at
  `firstAt + maxWait`, with the crate's shared retry policy (7 attempts, the waits doubling from one second); later
  items keep that timer. An `Add` handled at or after that deadline sends the batch with its own item. With
  `maxItems` 3 and `maxWait` 10 s, items handled at 0, 1 and 2 s go as one batch at 2 s, and an item at 4 s is due
  alone at 14 s. A due time past `MAX_INSTANT` fails the `Add` for good (`refused`) and changes nothing. An `Add` and
  the timer due at the same instant run in the core's order: the `Add` first joins the batch; the flush first sends the
  batch, and the `Add` starts the next one.
- **Flush is public.** A `Flush` sends the items held, cancels the armed timer and clears the batch in one commit; with
  nothing held it ends `done` and does nothing. Anyone may send one, so an operator delivers early, or recovers a
  flush that failed. It sends what is held when it runs: an old `Flush` that is retried or redriven while a new batch
  waits flushes the new batch early.
- **What the target gets.** The message type with the payload `{"items": [...]}`, due now, with a new id, the batcher
  as its sender, the settings' retry policy, no correlation id, and the default retention. An observer still adds its
  own headers to the send. A retry at the target goes after its own backoff, without passing the batcher again; the
  policy of an `Add` or of the timer governs only that turn.
- **A failed flush clears nothing.** A target type that is not registered or does not take the message, or a batch out
  of bounds, fails the turn for good (`refused`); a target that is gone rejects the commit (`commit`); a read of the
  store that fails is tried again by the incoming message's own retry policy. The batch and its timer stay as they
  were. Terminal failures reach the batcher's watchers, and dead letters that watch it keep them; the batcher watches
  nothing. A failure at the target after the flush committed fails the target's message, which holds the whole batch,
  and brings nothing back to the batcher.
- **A failed final `Add` keeps no item.** When the `Add` that fills the batch, or that comes after the deadline, cannot
  send it, its item is not kept: the batch stays below `maxItems` however long the target is gone. That `Add`'s failure
  holds the item, and dead letters that watch the batcher keep it. A retry or a redrive of it adds the item to the batch
  held then, so it may come after later items.
- **A lost timer.** A flush that failed for good, or that an operator cancelled or purged (which tells no watcher),
  leaves the items without a live timer, and so does a batch created with items and no `armed`. An `Add` before the
  old deadline does not arm a new timer; the batch goes when it is full, on an `Add` at or after the deadline, or on a
  `Flush`. Send a `Flush` to recover it.
- **Groups.** To collect a known number of parts, use one batcher for each group, with `maxItems` the number of parts
  and `maxWait` the timeout: the batch goes when the last part comes, or at the timeout with the parts that came. Put
  the group in each item, as the target is the same for all groups, and delete the batcher when the group is done.
- **Count, not bytes.** `maxItems` bounds how many items are held, not their size, and each `Add` rewrites the whole
  state. Keep items small, or batch references to data kept elsewhere.
- **Due times are not run times.** `maxWait` bounds the scheduled flush from the store's time at which the batch's
  first item was handled. Time an item waited in the mailbox, a pause of the batcher, an outage, a retry or a slow
  commit can all make the flush later; due times are rounded up to a microsecond and clamped to the store's time at
  the commit. A paused batcher flushes when it is resumed and its flush is claimed.
- **It never stops itself.** A create of a batcher that exists changes nothing. Pause, resume and delete work as for
  any actor; a delete discards the items held, and batches already sent stay with their target.
- **How deep.** The state and the batch hold each item two levels down, so an item may be at most 62 levels deep. A
  deeper item fails its `Add`, with the cause `handler` when it would be held and `refused` when it would be sent at
  once; the earlier batch and its timer stay as they were. The batcher is newer than 0.16.0, so no stored batch holds
  a deeper item.
