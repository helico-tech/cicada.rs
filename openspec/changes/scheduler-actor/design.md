# Design

Described on 2026-11-12 and 2026-11-13 (change loop, Describe), with the owner. Nothing is being built. The owner
agreed to decisions 1, 3, 6 and 9 as asked (S1A, S2B, S3A, S4A) and to the calls in 3, 4 and 7 that the main session
made; after the first review to a retry policy for the delivered message (S5A), the slot for the target (S6A), one
public `Run` in place of `RunNow` and `Deliver` (S7A), and a `Run` that delivers before `Start` (S8); and after the
second review to leaving cron expressions and time zones out (S9B) and warning about a scheduler that runs itself
(S10A). "Changed by the reviews" at the end lists what the reviews changed.

## Context

What the code does today (release 0.16.0):

- [examples/src/cron.rs](../../../examples/src/cron.rs) is a full cron chain: `Start`, `Tick { scheduled_for }`,
  `RunNow` and `StopJob`. The work is the host's `CronTask`, run inside the `Tick`, which gets the slot. A tick arms
  the next one at `schedule.next(current_time().max(slot))`, with a retry policy of its own (3 runs, 1 minute apart).
  The state keeps the id of the armed tick, so `RunNow` cancels it and arms a new one. The sample service hosts it as
  the type `cron`.
- `cicada_actors::Schedule` is `every(seconds)`, aligned to the epoch, or `daily(hour, minute)`, in UTC. `next(after)`
  is the first slot strictly after a time, in whole seconds. A decoded schedule is checked like a constructed one. Its
  doc comment arms the next tick at `next(max(now, slot))`.
- The topic delivers with a `Deliver` that it sends itself, with the crate's shared retry policy (7 runs, from 1 s up
  to 32 s, 63 s in all). A read of the store that fails is retried; a send refused at the call (no registered type,
  or a type that does not take the message) is refused for good. It watches itself to drop a subscriber that is gone.
- A send to an actor that does not exist rejects the whole commit (cause `commit`), and it is not retried.
- Only the sender of a message gives it a retry policy (`SendOptions::retry`); its receiver cannot. A policy of one
  attempt means no retry. `RetryPolicy` is a type of the core, with serde and a JSON Schema.
- `SendOptions` carries headers, passed through unchanged.
- `turn.cancel(id)` ignores an id that is not a pending message of the actor by then.
- An operator's retry of a failed message sends it again with a new id and no sender; a redrive of the dead letters
  sends it from the dead letters. A check on the sender or the id ignores both.
- An operator's `deliver-now` makes a pending message due at once and keeps its id and its sender. A purge of a
  mailbox and an operator's cancel of a message tell nobody.
- A paused actor is not claimed; its pending messages wait, and a timer that came due meanwhile runs at the resume.
- The ready-made actors (ADR 0028) have a type name `cicada.<x>`, a ref `{ name }` with `named`, and own messages that
  count only when the actor sent them itself. A `StoredMessage` holds a type name and a payload, no address.
- A contract changes with every field, also one with a default. A changed contract means a versioned type name, or a
  stop of the world that deletes every actor of the type (the operations guide, "Changing a contract").
- The dead letters keep 1,000 letters by default and drop the oldest first.
- The sample service hosts the other three ready-made actors, and the `cron` type of the example.
- Issue 0602 left cron expressions out until their wire form was agreed with the Kotlin implementation; ADR 0008
  ended that tie.

## Goals / Non-Goals

**Goals**

- A recurring send with no host code: the schedule, the target, the message and its retry policy are all in the
  state, and an operator creates and edits a schedule at runtime, over HTTP or the command line.
- A chain that does not end by itself: not by a target that is gone, nor by a message the target refuses, nor by a
  failure of the target.
- A first contract that holds every edit an operator makes, so the next request needs no new type name.

**Non-Goals**

- Cron expressions and time zones. Nobody has asked for them, and they need a cron parser and a time zone database,
  which the crate does not have. The first request for either means `cicada.scheduler.v2` (Risks).
- Skipping a slot while the target still works on the last one. It needs a notice of success that the core does not
  give.
- Catching up missed slots, a backfill, an end date, a limit on runs, and jitter.
- A record of failed deliveries in the state, and pausing by itself when the target is gone (decision 6).
- Refusing a scheduler that runs itself (decision 7).
- Changes to the core, and an ADR (decision 1).

## Decisions

### 1. No ADR

- **No ADR (chosen).** The repo asks for an ADR when a change alters a guarantee or a wire format. The contract of a
  ready-made actor is a wire format (ADR 0028), but this change adds one and alters none, and it follows the rules of
  ADR 0028. Its decisions live here and in the spec, and `docs/reference/contracts` pins its contract. The answer
  holds for the rate limiter, the debouncer and the batcher, unless one of them changes the core.
- One ADR for the four small actors. It would repeat ADR 0028.
- One ADR for each actor. Four documents that say little.

### 2. One instance per schedule

`Scheduler { name }`, built with `Scheduler::named("nightly")`, under the type name `cicada.scheduler`; the key is the
name of the schedule. `SchedulerActor` is a unit struct: every setting is in the state, so an operator edits it
without a deploy.

```rust
#[serde(rename_all = "camelCase")]
pub struct SchedulerState {
    pub schedule: Schedule,
    pub target: ActorAddress,
    pub message: StoredMessage,
    #[serde(default)] pub retry: Option<RetryPolicy>,    // for the delivered message; none: one attempt
    #[serde(default)] pub next_run: Option<Timestamp>,   // the slot of the armed tick
    #[serde(default)] pub armed_tick: Option<MessageId>, // the armed tick, which Start and Update cancel
    #[serde(default)] pub runs: u64,                     // deliveries that reached the target's mailbox
    #[serde(default)] pub last_run: Option<Timestamp>,   // the store's time of the run that sent the last of them
}
```

The state of a create over HTTP:

```json
{"schedule": {"type": "daily", "hour": 2, "minute": 0}, "target": "reports/_",
 "message": {"type": "Build", "payload": {"full": true}},
 "retry": {"maxAttempts": 3, "backoff": {"kind": "fixed", "delay": "PT5M"}}}
```

`SchedulerState::new(schedule, &target, message)` builds a state from a typed ref and a message of its family, so the
compiler checks that the target takes the message, as `StoredMessage::for_ref` does. It returns `EncodeError` for a
value that is not a message family. The fields are public, as in the other states of the crate, so a host sets the
retry policy with `state.retry = Some(policy)`; there is no builder method.

A scheduler is **started** when `armedTick` is set in its state. The field stays set when its tick is purged,
cancelled or fails for good (decision 6). A state that a client creates with an `armedTick` is started by that
definition; nothing worse follows from it.

Not chosen: settings in the actor value. A host would need code for each schedule.

### 3. The messages: `Start`, `Run`, `Update`, and its own `Tick`

```rust
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum SchedulerMsg {
    Start,
    Run { #[serde(default)] slot: Option<Timestamp> },
    Update {
        schedule: Option<Schedule>,
        target: Option<ActorAddress>,
        message: Option<StoredMessage>,
        retry: Option<RetryPolicy>,
    },
    Tick { slot: Timestamp }, // its own
}
```

- `Start` cancels the armed tick, if there is one, and arms a `Tick` at `schedule.next(now)`. It is safe to repeat:
  one tick stays armed. It also arms a chain again that ended (decision 6).
- `Run` sends the stored message to the target (decision 4). A tick sends it with its slot; an operator sends
  `{"type": "Run"}` with none, to run now, also on a scheduler that was never started. The slot belongs to the
  scheduler: the guide says that a client sends `Run` without one. The armed tick stays: the slots are aligned to the
  clock, so arming again from now would give the same slot. (The cron example cancels and arms again; that is not
  needed.)
- `Update` replaces each field it gives and keeps the counts. A new schedule on a started scheduler cancels the armed
  tick and arms one at the new schedule's next slot, also when the chain had ended; on a scheduler that was never
  started it is only kept. A new target, message or retry policy counts from the next `Run`, also one that is already
  waiting. A retry policy of one attempt turns retries off. An `Update` with no field changes nothing.
- `Tick` counts only when the scheduler sent it itself and it is the armed one (its id is `armedTick`). Otherwise it
  ends `done` and changes nothing.

An `Update` example: `{"type": "Update", "payload": {"message": {"type": "Build", "payload": {"full": false}}}}`.

- **`Update` with optional fields (chosen).** One message covers every edit at runtime and keeps the counts.
  Changing the payload, the target or the retry is the obvious next request, and adding a message later changes the
  contract.
- `Reschedule { schedule }`, as the proposal had it. A new target or message would need a contract change later, or
  a delete and a create.
- `Start` alone, every edit a delete, a create and a `Start`: three calls, and the counts are lost.

**`Run` is public (chosen).** A failed run is the only sign of a missed delivery, and an operator's retry or a redrive
of the dead letters must run it again; a check on the sender would make both end `done` and send nothing. A `Run`
holds no address, so a client that sends one can do no more than run the schedule now. Not chosen: an own `Deliver`
next to a public `RunNow` (a retry or a redrive of a failed `Deliver` does nothing), or the same with no check on
`Deliver` (two messages that do one thing).

### 4. Two turns for each slot

A `Tick` arms the next tick and sends the scheduler a `Run { slot }`, in one commit. The `Run` reads the target, the
message and the retry policy from the state and sends the message with `send_raw`; when that commits, it adds 1 to
`runs` and sets `lastRun` to the store's time of its turn.

- **Two turns (chosen).** A send to a target that is gone rejects only the commit of the `Run`; the next tick is
  already armed.
- The `Tick` sends to the target itself. A target that is gone would reject the commit that arms the next tick, and
  the chain would end.
- The `Run` carries the message. It would send a stale copy after an `Update`, and hold the payload one level deeper.

### 5. Missed slots, pauses and an early tick

A `Tick` arms the next one at `schedule.next(max(turn.now(), slot))`, on the store clock (S9), as `Schedule`'s doc
comment says. A late tick delivers once and the slots it missed are skipped. A paused scheduler is not claimed: its
tick waits, and at the resume it delivers once and arms the next slot from then. An operator's `deliver-now` of the
armed tick runs it early, with its id and sender; it delivers that slot once, and the next tick goes to the slot after
it, not to the same slot again. Start and Update cancel the armed tick before they arm another, so one tick is armed at
a time. A `Run` from an operator just before a slot gives two deliveries close together; the guide says so.

Not chosen: `schedule.next(turn.now())`. A tick that runs before its slot would arm the same slot again and deliver it
twice.

### 6. Retries and failures of the scheduler's own turns

- A `Tick` and a `Run` that a tick sends carry the crate's shared retry policy, so a failure of the store does not end
  the chain or lose a run.
- A `Run` whose read of the store fails is retried. A send refused at the call is refused for good (cause `refused`).
  A target that is gone fails the run at the commit (cause `commit`), which is not retried. The next slot delivers as
  usual.
- **No record in the state (chosen).** A failed run is a failed message of the scheduler: its watchers are told, the
  dead letters keep it where they watch (`cicada.scheduler/<name>`, the prefix `cicada`, or `*`), and the outcomes
  show it. The scheduler watches nothing, so its family has no `Failed` and `Stopped`.
- `lastError` and `failedRuns` in the state. The `Run` cannot write them, as a rejected commit writes nothing, so the
  scheduler would watch itself, as the topic does: a watch, and one more turn for each failure.

**A chain ends** when its armed tick goes without running: a purge of the scheduler's mailbox, an operator's cancel of
the tick, or a tick that fails for good (the claim limit, which asks no retry policy). Only the last tells the
scheduler's watchers. A retry or a redrive of a failed tick has a new id or another sender, so it ends `done` and
arms nothing. In each case a `Start`, or an `Update` with a schedule, arms the chain again; the guide says so.

A scheduler whose target is gone fails every slot until an operator pauses it, deletes it, or creates the target
again; it does not stop by itself, since that too needs a watch on itself (Risks).

### 7. What the target sees

- The delivered message has the scheduler as its sender, no correlation id, and a new id for each run.
- It carries the scheduler's retry policy for it (`retry` in the state), so a failed run of the target is tried again
  after its backoff instead of waiting for the next slot. Without one, a failed run ends `failed` at the target.
- A run with a slot adds the header `cicada-slot`: the slot in UTC to the second, as `2026-11-13T02:00:00Z`. The
  target knows which slot it handles also when the tick was late. A run without a slot has no such header. The name
  and the form of the header are stable, as a target keys its work by them; the contract does not hold headers, so
  the spec and the guide pin them.
- A `Run` that commits puts exactly one message into the target's mailbox (G4); the target handles it at least once,
  and uses `turn.message().id`, or the slot, as idempotency key. A target slower than the schedule never runs two
  deliveries at once (G1): they wait in its mailbox, and none is skipped.
- A scheduler whose target is itself, with the message `Run`, runs itself without end until it is paused. The guide
  warns about it. It is not refused: nobody sets it up by accident, and a loop through two schedulers would get past
  any check.

- **Retry policy and slot header now (chosen).** Both are the next requests a scheduler gets (the cron example retried
  its work and passed the slot), and the retry policy changes the contract when it is added later.
- Neither, named in Risks. A daily job whose run fails would wait a day.

### 8. How deep

The state holds the payload of its message two levels down (`message.payload`), and so does an `Update` that sets a
message. A payload deeper than 62 levels is refused: at the create of the scheduler (`400` over HTTP,
`OpError::Invalid` from code), and at the send of the `Update`.

### 9. The cron example and the sample service

- **Keep `examples/src/cron.rs`, and host the scheduler in the sample service (chosen).** The example shows a chain
  that does host work in its own turn, which the scheduler does not. The sample service already hosts the other
  three ready-made actors; it creates no scheduler at start, since an operator creates a schedule.
- Remove the example and move the sample service to the scheduler. The pattern of a timer chain with host work would
  lose its worked example.
- Keep both as they are. The scheduler would be hosted nowhere it can be tried.

The section "Schedules" of the actors guide becomes "Scheduler": the actor, its messages and its rules, with doctests;
`Schedule` stays in it, and the link to the cron example names it as the chain for host work. The guide's tables and
its count of the types name the scheduler too.

## Risks / Trade-offs

- **A later feature changes the contract.** Cron expressions or time zones change the wire form of `Schedule`, and so
  the contract of every type that keeps one. A record of failures needs `Failed` and `Stopped` in the family. A skip
  while the target still works needs state of its own. Each then needs `cicada.scheduler.v2`, or a stop of the world
  that deletes every schedule. `Update`, the retry policy and the slot cover the next requests; the rest are named
  here.
- **The contract holds a type of the core.** It is the first contract of a ready-made actor with `RetryPolicy` in it,
  so a change to `RetryPolicy` (a new kind of backoff, jitter) changes this contract too.
- **A target that is gone fails every slot.** With `every(60)` that is 1,440 failed messages a day; dead letters that
  watch the scheduler keep 1,000 by default and drop the oldest first, so one forgotten scheduler pushes other
  failures out within a day. The guide says to pause or delete such a scheduler.
- **A chain can end without a word.** A purge of the scheduler's mailbox or a cancel of its tick tells nobody, and a
  retried tick arms nothing. The guide names them, with `Start` as the fix.
- **A scheduler that runs itself** loops until it is paused. The guide warns about it.
- **Two turns for each slot.** A schedule every second is two turns a second. Throughput is not a goal.
- **A slow target.** Deliveries pile up in its mailbox; none is skipped.
- **A refused message** fails every slot until an `Update` sets one the target takes.
- **Two calls over HTTP.** A schedule needs a create and a `Start`. A turn does both in one commit, as creates come
  before sends.
- **The same payload on every slot.** A target that takes an id from the payload sees the same id each time; it keys
  its work by the slot header or the message id. The `durable-flows` proposal names this for runs started by an event.
- **A slot that a client makes up.** A client can send `Run` with any slot, also one in the future, and a target that
  keys its work by the slot would then drop the real run. The guide says that a client sends `Run` without a slot.

## Migration Plan

None: a new type. A node that hosts it registers its contract at its first start; nothing changes for the others.

## Changed by the reviews

The first review (2026-11-12) found:

- The delivered message had no retry policy, and the target never learned its slot; both would change the contract
  when added later. Now `retry` in the state and in `Update` (S5A), and `slot` on `Run` with the header `cicada-slot`
  (S6A).
- A retry or a redrive of a failed `Deliver` did nothing, as `Deliver` counted only from the scheduler itself. Now one
  public `Run` replaces `RunNow` and `Deliver` (S7A).
- `RunNow` before `Start` contradicted the spec, and "started" had no definition. Now a `Run` delivers at any time
  (S8), only the slots wait for `Start`, and "started" means an armed tick, which an `Update` with a schedule also
  arms again after a tick that failed for good.
- A target that is gone fails every slot and can push other failures out of the dead letters: named in Risks and the
  guide.
- The Impact list missed the architecture, the map of `AGENTS.md`, the guide's tables and the contract snapshots of
  the sample service.
- The reason for no ADR contradicted ADR 0028: reworded.
- The same payload on every slot clashes with runs keyed by an event id in `durable-flows`: a line there.

The second review (2026-11-13) found:

- An operator's `deliver-now` of the armed tick delivered its slot twice, as the next tick was armed from `now`. Now
  from `max(now, slot)`, as `Schedule` says.
- A purge, a cancel of the tick, and a retry or a redrive of a failed tick end the chain without a word. Now named in
  decision 6, the Risks and the guide, with `Start` as the fix; "started" now reads "`armedTick` is set".
- The reason for leaving out cron expressions was stale (issue 0602 was about the Kotlin implementation). Now: nobody
  asked, two missing dependencies, and a `v2` named in Risks (S9B).
- The first ready-made contract with a type of the core (`RetryPolicy`): named in Risks.
- The header `cicada-slot` is a wire format: now stable, with its form pinned. A slot that a client makes up is named.
- The retry setter was left to the build: now the public field.
- The spec lacked scenarios for an `Update` of the target or the retry, an empty `Update`, and a waiting `Run` that
  sees an `Update`, and `lastRun` had no stated value. Added.
- A scheduler that runs itself loops: a warning in the guide (S10A).
- Stale text in the proposal (the sample service would replace `cron`) and in the `debouncer-actor` proposal (the
  scheduler's `Deliver`): fixed.

## Open Questions

None.
