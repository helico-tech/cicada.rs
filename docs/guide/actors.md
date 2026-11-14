# Ready-made actors: choose a pattern

Host these types from `cicada-actors`, create their instances, then send their public messages.
They use ordinary actor guarantees, watches and control effects from the [core](core.md).

| Need | Actor / type | Complete example |
| --- | --- | --- |
| Restart a child at fault | `SupervisorActor` / `cicada.supervisor` | [supervision](../../examples/src/supervision.rs) |
| Keep and redrive failed messages | `DeadLettersActor` / `cicada.dead-letters` | [supervision](../../examples/src/supervision.rs) |
| Send to existing subscribers | `TopicActor` / `cicada.topic` | [topic tests](../../crates/cicada-actors/tests/) |
| Send at recurring UTC slots | `SchedulerActor` / `cicada.scheduler` | [cron](../../examples/src/cron.rs) |
| Space deliveries at a shared rate | `RateLimiterActor` / `cicada.rate-limiter` | [rate_limit](../../examples/src/rate_limit.rs) |
| Keep the latest value of a burst | `DebouncerActor` / `cicada.debouncer` | [debounce](../../examples/src/debounce.rs) |
| Collect items into one delivery | `BatcherActor` / `cicada.batcher` | [batching](../../examples/src/batching.rs) |
| Run a durable multi-step job | `FlowActor<F>` / host-chosen name | [flow_onboarding](../../examples/src/flow_onboarding.rs) |
| Start flows from a publication | `EventActor` / `cicada.event` | [flow_events](../../examples/src/flow_events.rs) |

## Shared conventions

Use named refs such as `Scheduler::named("nightly")`; dead letters use `_` by convention.
Creating an existing instance leaves its settings and progress untouched. Pause/resume/delete use the core controls.
A delete drops stored work; already forwarded messages stay in their targets’ mailboxes.

`StoredMessage` holds a type name and JSON payload, without an address. Build it with
`StoredMessage::for_ref(&target, message)` for compile-time message-family checking.
[Contract references](../reference/contracts/) contain the exact state and message formats.

Forwarded deliveries get new ids and the forwarding actor as sender. Put reply addresses in payloads.
A submission’s retry policy governs the forwarding turn; the target’s policy is a separate setting.
Target retries do not pass through a limiter, debouncer or batcher again.

**Due time is a lower bound.** Pauses, queueing, outages and slow commits can make delivery late.
Each turn rewrites the state it changes: keep stored values small or store references to external data.

## Message processing

Choose what should reach the target: accepted messages at reserved due times, the latest value after quiet, or a list.
These are three alternatives, not stages of one pipeline.

```mermaid
flowchart LR
    a["Signup"] -->|"Submit (Send)"| limiter["Rate limiter"] -->|"Send (due at slot)"| sms["SMS actor"]
    b["Document"] -->|"Submit (Reindex)"| debounce["Debouncer"] -->|"Reindex (latest)"| indexer["Indexer"]
    debounce -->|"Flush (when due)"| debounce
    c["Account"] -->|Add| batcher["Batcher"] -->|"Write { items }"| audit["Audit log"]
    batcher -->|"Flush (when due)"| batcher
```

Labels are the message variants from [rate_limit.rs](../../examples/src/rate_limit.rs),
[debounce.rs](../../examples/src/debounce.rs) and [batching.rs](../../examples/src/batching.rs):
`RateLimiterMsg::Submit` wraps `SmsMsg::Send`; `DebouncerMsg::Submit` wraps `IndexerMsg::Reindex`;
`BatcherMsg::Add` becomes `AuditLogMsg::Write`. A full batch sends during Add without waiting for Flush.
Due times do not bound execution time; a paused or busy target can run later.

## Supervisor

Use a supervisor only when a child can restart from its configured initial state.
**A restart deletes the old state, descendants and mailbox**; purged messages do not become dead letters.
Do not supervise dead letters or a topic whose subscriptions you must keep.

```mermaid
sequenceDiagram
    participant C as Mailer child
    participant S as SQL store
    participant P as Supervisor
    participant D as Dead letters
    Note over C: MailerMsg::SendMail fails
    C->>S: Commit terminal failure
    S->>P: SupervisorMsg::Failed (watch)
    S->>D: DeadLettersMsg::Failed (watch)
    Note over S: Failure + notifications<br/>commit together
    P->>S: Commit child restart + Restarted
    S->>P: SupervisorMsg::Restarted
    D->>S: Commit retained letter
```

The two watchers act independently. Restarting does not replay the failed message; an operator redrives its letter.
The diagram assumes a handler fault below the restart limit and both watches already configured.

Build `SupervisorState::new().child(Child::new(&ref, &state)?.start(message)?)`, create the supervisor, then send `Start`.
Each child starts independently in its own `StartChild` turn: create as a child, watch, send its start message.
An actor with another parent or no parent cannot be adopted.

| Child failure cause | Response |
| --- | --- |
| `handler`, `timeout` | Restart that child |
| `state` | Give up without deleting its state; escalate permanently |
| `refused`, `message`, `commit`, `node`, `claim-limit` | Leave it running |

A restart suppresses further failures of the same incident until `Restarted` is handled.
The default limit is 3 restarts in 60 seconds; `restart_limit` changes it. Exceeding it fails a supervisor message
as a handler failure, allowing a parent supervisor to restart its tree. A child that cannot start/read fails a
supervisor message permanently instead. Start attempts use 7 attempts with waits doubling from 1 s (63 s total waits).

Send `Start` again to recover a child that was given up or an interrupted restart. A child that stops/is deleted is
forgotten until a later restart of its supervisor. Only real store notifications drive failure handling; internal
messages count only when self-sent. Host the supervisor with its child types to validate entries before creation.
See [the implementation](../../crates/cicada-actors/src/supervisor.rs) for incident and restart details.

## Dead letters

Create the actor and set its watch explicitly. This is the complete setup for a handler-free example host:

<!-- doctest
@@2
# async fn example(store: cicada::Store) -> Result<(), Box<dyn std::error::Error>> {
@@6
# Ok(()) }
-->
```rust,no_run
use cicada::{ActorRef, ActorSystem, WatchTarget};
use cicada_actors::{DeadLetters, DeadLettersActor, DeadLettersState};

let system = ActorSystem::builder(store).host(DeadLettersActor::with_capacity(1_000)?).build();
system.start().await?;
let letters = DeadLetters::default();
system.create(&letters, &DeadLettersState::default()).await?;
system.watch(&WatchTarget::Every, &letters.address()?).await?;
```

Only store `Failed` notifications create letters. Capacity defaults to 1000, with the oldest dropped first.
Letters keep the message, error, cause, attempts, recorded time and notification sequence.

| Message | Result |
| --- | --- |
| `Redrive { message_id }` (`messageId` on the wire) | Send again with a new id, keeping payload/correlation; record `redrivenAs` |
| `Discard { message_id }` | Forget the letter |

A successful redrive happens once per letter. It does not restore the original sender, retry policy or headers.
An invalid message/type or unreadable payload records `redriveError`; a missing target rejects the commit and leaves
the letter unchanged. A redriven failure becomes a new letter.

Payloads up to 61 levels deep stay in `payload`; deeper accepted payloads use compact JSON in `payloadJson`.
Exactly one field is present. In Rust, `LetterPayload::value()` decodes it on demand with exact numeric rounding.
Redrive applies current send checks; retained text does not make invalid/deep data sendable.
Legacy deep letters need conversion or discard: [upgrade procedure](../how-to/upgrading.md).

## Topic

```mermaid
sequenceDiagram
    participant U as Publisher
    participant T as Topic
    participant A as Subscriber A
    participant B as Subscriber B
    U->>T: TopicMsg::Publish (Note)
    Note over T: Commit one Deliver<br/>per subscriber
    T->>T: TopicMsg::Deliver { subscriber: A, message }
    T->>A: WorkerMsg::Note { text }
    T->>T: TopicMsg::Deliver { subscriber: B, message }
    T->>B: WorkerMsg::Note { text }
```

Here the published message is `WorkerMsg::Note`, as in [the topic tests](../../crates/cicada-actors/tests/topic.rs).
`Publish` and `Deliver` wrap that message; subscribers receive `Note`, not `Deliver`.
Each Deliver is a separate topic turn; neither subscriber is created by the topic.

Create `TopicState::default()`, send `Subscribe { subscriber }`, then `TopicMsg::publish(message)`.
`Publish` creates one self-sent `Deliver` per subscriber, so one broken subscriber does not block the others.
Deliveries carry the publication’s correlation id. Per-subscriber publication order holds unless a delivery is retried.

A missing instance rejects its delivery and the topic drops that subscriber after the failure notification.
An unregistered type or unsupported message permanently refuses delivery but keeps the subscription: unsubscribe it
explicitly. The topic watches itself; forged notifications and non-self-sent deliveries do nothing.
One turn per subscriber per publication makes large fan-outs expensive.
[Source](../../crates/cicada-actors/src/topic.rs) and [wire contract](../reference/contracts/cicada.topic.json).

## Scheduler

Build `SchedulerState::new(schedule, &target, message)`, create the actor, then send `SchedulerMsg::Start`.
A slot takes two turns: `Tick` arms the next tick and queues `Run`; `Run` forwards the currently configured message.

| Message | Use |
| --- | --- |
| `Start` | Start or repair the tick chain; safe to repeat |
| `Run` | Deliver now, even before Start, without moving the tick |
| `Update` | Replace supplied schedule/target/message/retry fields; a changed schedule rearms the tick |

```mermaid
sequenceDiagram
    participant U as Caller
    participant Q as Scheduler
    participant T as Worker
    U->>Q: SchedulerMsg::Start
    Q->>Q: SchedulerMsg::Tick { slot } (when due)
    Note over Q: Tick commits next delayed Tick<br/>+ Run
    Q->>Q: SchedulerMsg::Run { slot }
    Q->>T: WorkerMsg::Note { text }
    Note over T: cicada-slot header<br/>identifies the UTC slot
    Q->>Q: SchedulerMsg::Tick { slot: next } (when due)
```

Here the configured target message is `WorkerMsg::Note`, as in [the scheduler tests](../../crates/cicada-actors/tests/scheduler.rs).
Set `state.retry` for failures at the target. Scheduled deliveries carry `cicada-slot` in UTC to the second;
a manual `Run` has no slot. Correlation ids are not forwarded. Internal ticks must be self-sent and armed.

A late tick delivers once, then skips missed slots. A failed Run does not stop future ticks; pause/delete an orphaned
schedule to stop repeated failures. Cancelling/purging the armed tick can stall the chain; send Start to repair it.
A retry/redrive of the old tick does not repair it. Never target the scheduler’s own Run: it creates an endless chain.
[Source](../../crates/cicada-actors/src/scheduler.rs).

### Schedule

`Schedule::every(seconds)` aligns intervals to the Unix epoch. `Schedule::daily(hour, minute)` uses UTC.
`next(time)` returns the first slot strictly after the given time. There are no cron expressions or time zones.

<!-- doctest
@@5
# Ok::<(), Box<dyn std::error::Error>>(())
-->
```rust
use cicada::time::Timestamp;
use cicada_actors::Schedule;

let now: Timestamp = "2026-10-02T10:15:30Z".parse()?;
assert_eq!(Schedule::every(3600)?.next(now), "2026-10-02T11:00:00Z".parse::<Timestamp>()?);
```

For work inside your own actor’s turn, use the [cron example](../../examples/src/cron.rs).

## Rate limiter

Create `RateLimiterState::new(count, period, burst, max_delay)` and send
`RateLimiterMsg::submit(&target, message, target_retry)`.
It reserves a GCRA slot and forwards atomically. Count/burst/period must be positive; max delay may be zero.
The interval rounds up to microseconds, and idle time restores the burst.

A slot exactly at `maxDelay` is allowed; a later slot is refused permanently. The limit counts from handling time,
excluding mailbox wait/commit. A failed forward reserves nothing; cancelling a forwarded message returns no capacity.
Forwarded messages keep the submission’s correlation id but not its headers or retention.

**This shapes due times, not external API calls.** A paused or slow target can run several overdue deliveries close
together. Enforce strict external quotas at the call site. Replace the actor to change settings; existing forwards
keep their slots. [Example](../../examples/src/rate_limit.rs).

## Debouncer

Create `DebouncerState::new(quiet, max_wait)` with `0 < quiet <= maxWait`, then submit through
`DebouncerMsg::submit(&target, message, target_retry)`.
The latest target, message, retry and correlation id replace the previous submission together.
It keeps the trailing value only and checks the target at flush time.

Each submission rearms a timer at `min(now + quiet, firstAt + maxWait)`.
Public `Flush` sends the kept value, cancels the timer and clears the burst in one commit; an empty flush does nothing.
Headers/retention are not copied. A failed flush leaves everything intact; send Flush to recover a lost timer, or
submit a new value to replace the old one. A retried old Flush can flush a newer burst early.
Delete/recreate to change settings. [Example](../../examples/src/debounce.rs).

## Batcher

Create `BatcherState::new(&target, message_type, max_items, max_wait)`; `maxItems` is 1–1000 and max wait positive.
Send `BatcherMsg::add(item)` for each item. The target receives `{ "items": [...] }` in handling order.
Set the state’s `retry` for failures at the target. Item headers/correlation ids are not retained.

The first item arms a Flush timer. Filling the batch, an Add at/after the deadline, or a public Flush sends it and
clears it atomically. A stored batch therefore has fewer than `maxItems` items. A failed flush keeps the batch.
If the final Add cannot send, **that new item is not kept**; recover its failed message separately.

A cancelled/purged/failed timer can leave items stranded. Send Flush to recover; an Add before the old deadline does
not rearm it. A retried old Flush can send a newer batch early. Count limits do not bound bytes; keep items small.
Delete/recreate to change settings. [Example](../../examples/src/batching.rs).

## Flows

Use a `Flow` for a job whose durable progress is an ordered sequence of results.
Host it with `FlowActor::new(flow)` under the type name chosen by the flow.
Create a `FlowRun<F>` with `FlowState::new(input)`, then send `FlowMsg::Continue`.

The central part of [flow_onboarding.rs](../../examples/src/flow_onboarding.rs) is ordinary async code:

<!-- doctest
# use cicada::HandlerResult;
# use cicada_actors::Step;
# use std::time::Duration;
# use cicada_examples::steps::OnboardingServices;
# async fn run<S: OnboardingServices>(step: &mut Step, services: &S, email: &str) -> HandlerResult {
@@7
# Ok(()) }
-->
```rust
let account_id: String = step.run("create-account", |key| async move {
    Ok(services.create_account(email, &key).await?)
}).await?;
step.run("send-welcome", |key| async move {
    Ok(services.send_welcome(email, &key).await?)
}).await?;
step.sleep("cool-off", Duration::from_secs(60)).await?;
```

[The flow replay diagram](patterns.md#durable-flows) shows the commit boundaries.

The code runs from the top on each execution. Saved results replay without rerunning their closures; only one new
step, sleep, send, request or wait runs per turn. **Code outside steps must be deterministic and free of external effects.**
Put service calls, clocks and randomness inside steps.

Each closure receives `<run address>/<step id>` as an idempotency key. It can run again after an uncommitted turn;
external effects remain at least once. Never reuse a run key for a different job, even after deletion.

| Concern | Rule |
| --- | --- |
| Errors | `?` propagates the first new step’s suspension/error; catching it cannot continue the turn |
| Business decisions | Save a successful value such as `Result<T, String>` and branch on it |
| Timeouts | External call: timeout/select inside its step closure, never around `step.run`. Reply: `step.wait_timeout` |
| Replay compatibility | Keep entry order, ids, kinds and result types stable for in-progress runs |
| Ids and size | Unique ids shared by steps, sleeps, sends, requests and waits; 1–100 chars of `A-Za-z0-9._~-:`, at most 1000 entries |
| Recovery | Public Continue rearms a run whose armed message was cancelled/purged/exhausted; never restarts a cancelled run |
| Completion | Done and cancelled runs ignore Continue, Reply and Cancel; progress stays until deletion |
| Send delivery | One enqueue per committed entry; the target's execution is at least once, with no retry policy |
| Waiting | Single waits or joins of up to 100 requests, for all replies or the first; optional saved deadlines, stable committed outcomes |
| Available in 0.18.4 | Steps, sleeps, sends, requests, waits, joins and cancellation; cancellation runs no cleanup |

Only the armed Continue executes the flow; a public poke first replaces/arms it. Default retry is 7 attempts with
waits doubling from 1 s. A replay mismatch is permanently refused. A journal supplied at creation is trusted progress.
Input schema changes affect the contract; step-code changes can break replay **without changing the contract**.
Drain old runs or use a new flow type. The [0.18.0 upgrade](../how-to/upgrading.md#upgrade-flows-to-0180) changes every
flow contract. See [the flow source](../../crates/cicada-actors/src/flow.rs) for the journal API.

### Durable sleep

`step.sleep(id, duration)` saves a store-time deadline and schedules Continue without retaining a running handler.
Replay uses the saved time, even if the code’s duration changes. Early pokes/deliver-now cannot pass the sleep;
late wakes proceed immediately. Zero sleep still creates a commit boundary.

A lost timer is recovered with public Continue; deletion removes the run and timer. Deploy sleep-capable code to every
host before starting sleeping runs. Sleep ids share the step namespace/1000-entry limit; durations are bounded by
`MAX_DELAY` and year 9999. Timer continuations start linked traces with [OpenTelemetry](core.md#opentelemetry).

### Sending to actors

`step.send(id, to, message)` commits an optional target create, one message, its journal entry and the next Continue
together. Use `&actor` for an existing target, `Target::create(&actor, &state)` for an independent actor, or
`Target::child(&actor, &state)` for a child removed when the run is deleted. Existing actors keep their state.

This sends a report using the types from [sync.rs](../../examples/src/sync.rs):

<!-- doctest
# use cicada::HandlerResult;
# use cicada_actors::{Step, Target};
# use cicada_examples::sync::{Reports, ReportsMsg, ReportsState};
# async fn report(step: &mut Step) -> HandlerResult {
@@6
# Ok(()) }
-->
```rust
let reports = Reports { tenant: "acme".into() };
step.send(
    "report",
    Target::create(&reports, &ReportsState::default()),
    ReportsMsg::SyncFinished { imported: 25 },
).await?;
```

The entry saves the destination, generated message id and create kind, not the payload or initial state. Replay
matches only entry id and kind; it converts/serializes none of the proposed arguments, even if the target changed.
Argument construction still runs, so keep it deterministic and free of effects.

Enqueueing is not target success. The message is due now, uses the executing Continue's correlation id and has **no
retry policy**. Target failure leaves flow progress intact; design business-failure reporting and recovery explicitly.

| Problem | Outcome |
| --- | --- |
| Self-target, conversion/encoding error, or a caught conversion panic | Permanently refused |
| Unregistered type, unsupported message, or initial state that is null, contains NUL or exceeds depth limits—even for an existing target | Permanently refused by raw-operation checks |
| Registry read fails | Cause `node`; retried by the flow's policy |
| Missing target, or child create collides with an actor having another parent or none | Commit rejected, cause `commit`; fix the target and send a public Continue |

A failed turn commits no create, message or entry. After a rejected commit, public Continue attempts the send afresh
with a new message id. [Flow tests](../../crates/cicada-actors/tests/flow.rs) cover atomicity, replay and recovery.

### Requests and reply waits

`step.request(id, to, build)` sends work carrying a typed `ReplyTo<T>` route; `step.wait(id, &request)` returns its
reply as `T`. Use `step.wait_timeout` to branch on a reply or timeout. Targets use the same existing/create/child
choices as `step.send`. This purchase sets a one-week decision deadline, from
[flow_approval.rs](../../examples/src/flow_approval.rs):

<!-- doctest
# use cicada::HandlerResult;
# use cicada_actors::{Step, Target, Waited};
# use cicada_examples::flow_approval::{Approvals, ApprovalsMsg, ApprovalsState, DECISION_TIMEOUT, Decision, PurchaseInput};
# async fn ask(step: &mut Step, input: PurchaseInput) -> HandlerResult {
# let approvals = Approvals { team: input.team.clone() };
# let empty = ApprovalsState::default();
@@6
# let _: Option<Decision> = decision;
# Ok(()) }
-->
```rust
let ask = |reply| ApprovalsMsg::Ask { item: input.item.clone(), cents: input.cents, reply };
let request = step.request("ask-approval", Target::create(&approvals, &empty), ask).await?;
let decision: Option<Decision> = match step.wait_timeout("decision", &request, DECISION_TIMEOUT).await? {
    Waited::Received(decision) => Some(decision),
    Waited::TimedOut => None,
};
```

The target stores the route until it can reply from one of its own turns:

<!-- doctest
# use cicada::{HandlerResult, Turn};
# use cicada_examples::flow_approval::{ApprovalsActor, Decision, Pending};
# async fn decide(turn: &mut Turn<ApprovalsActor>, decided: Pending, approved: bool, by: String) -> HandlerResult {
@@1
# Ok(()) }
-->
```rust
decided.reply.send(turn, &Decision { approved, by }).await?;
```

The route is `{run, request}`: run address and request message id, with the same wire form for every `T`.
The request/create/entry/Continue commit together; replay returns the saved request without calling its builder.
The reply commits with the target's turn, is due now, carries that turn's correlation id and has no retry policy.
`reply.send` rejects encoding errors, NUL and values deeper than 61 levels with `TurnError::Invalid`.

Neither wait keeps a handler open or polls. An untimed wait **parks** with `armed: null`; a timed wait saves a
deadline and arms one Continue due then. The deadline is the wait-creating turn's claim store time plus the timeout
(rounded up to microseconds), not the request time. Replay uses the saved deadline even if the timeout changes.
A timeout beyond `MAX_DELAY` or year 9999 is permanently refused, even when a reply arrived early.

The reply turn runs no flow code: it saves the value, closes the wait, cancels any deadline timer and arms Continue
with the reply's correlation id, atomically. With no kept reply, a Continue at or after the deadline commits
`TimedOut` and arms another Continue with its own correlation id. **Only the next execution returns the committed
outcome**. A new wait with zero timeout still commits open first unless an early reply is already kept.
Timeout is `Waited::TimedOut` data, not a retryable error.
See [the message diagram](patterns.md#flow-requests-and-replies).

Mailbox order `(deliverAt, seq)` decides the race. A reply handled first can win **after** the deadline; pausing,
outages or a slow commit can delay the decision. Once committed, the outcome survives later retries and ignores
late replies. A timeout neither stops the target's work nor cleans it up; send an application cancellation message
if needed.

Only the **first eligible reply from the saved target address** counts. Unknown, duplicate, closed or wrong-sender
replies write nothing. Direct HTTP sends have no sender; delegates, children and dead-letter redrives have their own
addresses. Delegates must return results through the target. Trusted workers executing the target's leased turn send
as that target. This checks actor addresses, not human identity; control who can make the target reply.

| Situation | Recovery or limit |
| --- | --- |
| Target fails, disappears or never replies | Untimed wait stays parked; use a timed wait to give up, or reply with business failure data |
| Public Continue before deadline / while parked | Rearms the saved deadline / parks again; no new entry or request |
| Deadline Continue cancelled, purged or failed | Wait stays open, even past the deadline. Send public Continue to decide timeout; a reply handled first can still win |
| Saved reply does not decode as `T` | Permanently fails execution; fix code and poke with Continue. The saved outcome stays |
| Run deleted before target replies | The target's whole commit is rejected |
| Run paused / continuation lost after closure | Resume to handle replies / send public Continue to replay the saved outcome |

Keep each running wait's helper stable: switching between `wait` and `wait_timeout` permanently fails replay.
Upgrade every host and worker before enabling [requests](../how-to/upgrading.md#enable-flow-replies-in-0181) or
[deadlines](../how-to/upgrading.md#enable-flow-wait-deadlines-in-0182), and again before
[joins](../how-to/upgrading.md#enable-flow-joins-in-0183) or sending
[Cancel](../how-to/upgrading.md#enable-flow-cancellation-in-0184); equal contract hashes do not prove support.

### Joins: waiting for several replies

Start independent jobs with `step.request`, then join their replies with one wait entry. Choose the result you need:

| Helper | Returns |
| --- | --- |
| `step.wait_all(id, &requests)` | `Vec<T>` in request-list order, after every reply |
| `step.wait_all_timeout(id, &requests, timeout)` | `Vec<Waited<T>>` in request-list order; missing replies become `TimedOut` when the timeout decision commits |
| `step.wait_first(id, &requests)` | `(index, value)` for the listed request whose reply the run **accepted first** |
| `step.wait_first_timeout(id, &requests, timeout)` | `Waited::Received((index, value))`, or `Waited::TimedOut` with no winner |

The sign-off in [flow_approval.rs](../../examples/src/flow_approval.rs) asks every team and sets a one-week deadline:

<!-- doctest
# use cicada::HandlerResult;
# use cicada_actors::{Step, Target, Waited};
# use cicada_examples::flow_approval::{Approvals, ApprovalsMsg, ApprovalsState, DECISION_TIMEOUT, Decision, SignOffInput};
# async fn sign_off(step: &mut Step, input: SignOffInput) -> HandlerResult {
# let empty = ApprovalsState::default();
@@8
# let _: Vec<Waited<Decision>> = slots;
# Ok(()) }
-->
```rust
let mut requests = Vec::new();
for team in &input.teams {
    let approvals = Approvals { team: team.clone() };
    let ask = |reply| ApprovalsMsg::Ask { item: input.item.clone(), cents: input.cents, reply };
    requests.push(step.request(&format!("ask-{team}"), Target::create(&approvals, &empty), ask).await?);
}
// One slot per team, in the order of `input.teams`.
let slots = step.wait_all_timeout("sign-offs", &requests, DECISION_TIMEOUT).await?;
```

A timed-out all join keeps partial results: each slot is `Received(reply)` or `TimedOut`. It times out when the
decision commits at or after the deadline, with the replies kept then; the [single-wait ordering rules](#requests-and-reply-waits)
apply. The same example's quote buys from the first supplier to offer:

<!-- doctest
# use cicada::HandlerResult;
# use cicada_actors::{Request, Step};
# use cicada_examples::flow_approval::{Offer, QuoteInput};
# async fn first(step: &mut Step, input: QuoteInput, quotes: Vec<Request<Offer>>) -> HandlerResult {
@@2
# let _: (&str, Offer) = (supplier, offer);
# Ok(()) }
-->
```rust
let (winner, offer) = step.wait_first("first-quote", &quotes).await?;
let supplier = input.suppliers[winner].as_str();
```

Build the borrowed slice (`Vec` or array) deterministically from input and earlier results; replay compares its exact
order and mode. All handles share one reply type `T`; use an enum for several shapes. A join takes 1–100 distinct
earlier requests that no other wait names. With 100 requests, their join uses 101 of the 1000 journal entries.

Replies are kept as accepted, even before the join exists. A partial reply leaves the run parked or its deadline
Continue unchanged. Only the completing reply closes the join, cancels any timer and arms Continue with its correlation
id; no reply turn runs flow code. The next execution returns the saved selection and decodes only those replies.
For a first join, **acceptance order decides the winner**, including early replies. Preserve the `replies` order when
restoring progress. Once committed, every slot and winner stays stable on replay.

Closing a join closes **all** its requests. Losing or late targets keep working; their later replies are ignored without
a write. Cleanup needs new requests, since no later wait can reuse any of the closed join's handles.

| Situation | Recovery or limit |
| --- | --- |
| Empty list, more than 100 handles, duplicates, unknown or already-used requests | Permanently fails execution before checking kept replies |
| Changed list/order, all versus first, or deadline presence | Permanently fails replay; change joins only for new runs or drain old ones |
| Selected reply does not decode as `T` | Permanent failure names its position; fix code and send public Continue. The saved outcome stays |

A one-request all join replays the matching single wait; a first join stays distinct even with one request.
See [0.18.3 rollout](../how-to/upgrading.md#enable-flow-joins-in-0183) before enabling joins.

### Cancelling a run

Send `FlowMsg::Cancel` to end a running flow and keep its progress. Sending only queues it; **read the run's status**
to confirm cancellation. This test cancels the sleeping flow from the [FlowMsg example](../../crates/cicada-actors/src/flow.rs):

<!-- doctest
# use std::time::Duration;
# use cicada::prelude::*;
# use cicada_actors::{Flow, FlowActor, FlowMsg, FlowRun, FlowState, FlowStatus, Step};
# use cicada_test::TestActors;
# struct Nightly;
# impl Flow for Nightly {
#     const TYPE: &'static str = "nightly";
#     type Input = String;
#     async fn run(&self, step: &mut Step, _input: String) -> HandlerResult {
#         step.sleep("wait", Duration::from_secs(3600)).await?;
#         step.run("report", |_key| async { Ok(()) }).await
#     }
# }
# #[tokio::main(flavor = "current_thread", start_paused = true)]
# async fn main() {
# let test = TestActors::builder().with(|node| node.host(FlowActor::new(Nightly))).build().await.unwrap();
# let run = FlowRun::<Nightly>::named("n1");
# test.create(&run, &FlowState::new("n1".to_owned())).await;
# test.send(&run, FlowMsg::Continue).await;
# test.run_until_idle().await;
@@4
# assert_eq!(state.journal.len(), 1, "the saved sleep stays");
# assert!(test.pending(&run).await.is_empty(), "the timer is cancelled");
# }
-->
```rust
test.send(&run, FlowMsg::Cancel).await; // queued; not cancelled yet
test.run_until_idle().await;
let state: FlowState<String> = test.state(&run).await;
assert_eq!((state.status, state.armed), (FlowStatus::Cancelled, None));
```

The Cancel turn runs no flow code. Its one commit cancels the armed Continue, sets status
`cancelled` and clears `armed`, whether the run is sleeping, waiting, in retry backoff, never started or unable to replay. Input, journals, replies and any open wait stay
saved. A known rejected commit changes nothing and is not retried by the sender's policy. A lost commit answer is
uncertain: reread the status or resend Cancel, which a cancelled run ignores.

**Cancellation waits its turn and runs no cleanup.** Mailbox order `(deliverAt, seq)` applies; an execution already
running may still commit effects before Cancel is handled. Earlier effects and reply/timeout decisions stay saved.
Requests, children and external resources keep running, and later replies are ignored. Release resources in your
application; a future cleanup release will not clean up runs cancelled by 0.18.4.

A cancelled run is terminal and readable until deletion; use a new key for a new job. A done run ignores Cancel.
Any sender allowed to message the run can cancel it. Success is an ordinary done turn, with no watcher notification;
the cancelled Continue has no turn. Change hints are best effort: reread status, even when using hints to refresh it.

| Control | Effect on a flow |
| --- | --- |
| `FlowMsg::Cancel` | Ends the run when its turn commits; keeps progress, runs no cleanup |
| Cancel a pending Continue / cancel the running turn | Stalls the run, still `running`; the running turn stops at a heartbeat only if cancellation wins before it ends |
| Pause | Holds pending messages, including Cancel, until resume |
| Purge | Discards pending messages, including a queued Cancel; does not end the run |
| Delete | Removes the run, descendants and mailbox; a queued Cancel never runs |
| `FlowMsg::Continue` | Recovers a stalled running run; does nothing to done or cancelled runs |

A delayed Cancel is an ordinary scheduled message, not a hard deadline; the run may finish before it is handled.
[Upgrade all hosts and workers](../how-to/upgrading.md#enable-flow-cancellation-in-0184) before using it.

## Event

Create an `Event` with `EventState`, subscribe flow type names, then send `EventMsg::publish(input)`.
Each publication creates and starts one independent run per subscribed type **in one commit**.
`publish_batch(items)` supplies input `{ "items": [...] }`, so a batcher targeting PublishBatch feeds an event directly.
[flow_events.rs](../../examples/src/flow_events.rs) shows the full producer and host setup.

```mermaid
flowchart TD
    order["Order publisher"] -->|"EventMsg::Publish"| event["Order event"]
    event -->|FlowMsg::Continue| receipt["New receipt run"]
    event -->|FlowMsg::Continue| fulfil["New fulfilment run"]
    orders["Order publisher"] -->|BatcherMsg::Add| batcher["Batcher"]
    batcher -->|EventMsg::PublishBatch| digestEvent["Digest event"]
    digestEvent -->|FlowMsg::Continue| digest["New digest run"]
```

This is [flow_events.rs](../../examples/src/flow_events.rs): a topic delivers to existing actors; an event creates
independent flow runs. Each event commits its run creates and Continue messages together.
A failure after creation belongs to that run, not to the successful publication.

The run key is the publication’s message id, percent-encoded once. Do not reuse ids across jobs/events or manually
create runs whose keys could collide. `.` and `..` cannot key subscribed runs. Up to 100 unique flow types can subscribe.

An unknown/invalid subscribed target can fail the entire publication: remove stale subscriptions, then retry.
Registration alone cannot identify a flow; subscribe only flow types. On a non-hosting node, bad input may instead
create a run that fails its first message as `state`. Watch both events and flow types to see the two failure paths.

**Recover a started run with Continue, not another publication.** Operator retry/redrive gets a new publication id,
and therefore new runs. Core deduplication only lasts for the retention window; existing runs retain their first input.
Unsubscribe stops future runs; deleting an event does not delete its independent runs. No publication history is kept.

## Payload depth limits

Wrapping values consumes the core’s 64-level depth budget. Constructors/sends and state writes enforce it.

| Container | Maximum nested payload/input depth |
| --- | --- |
| Supervisor child state / start payload | 61 / 60 |
| Dead letter | 61 as a value; deeper payloads use JSON text |
| Topic, scheduler, rate-limiter payload | 62 |
| Debouncer kept payload | 61 (a 62-level submission can enqueue but fails when stored) |
| Batcher item | 62 |
| Flow input / step result | 63 / 61 |
| Event Publish input / PublishBatch item | 63 / 61 |

A batcher can accept an item the event cannot place in flow state; the publication then fails and starts no runs.
Legacy deep values require the [upgrade procedure](../how-to/upgrading.md); retaining a payload does not relax send
or state limits. Exact wire shapes remain in [contract references](../reference/contracts/).
