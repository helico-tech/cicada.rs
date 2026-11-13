# Proposal

## Why

A recurring job is the most common need that a durable store solves. The repo has one only as a worked example:
[examples/src/cron.rs](../../../examples/src/cron.rs) is a full cron chain (`Start`, `Tick`, `RunNow`), but its work
is a `CronTask` trait of the host, so every host that wants one copies the example. The sample service hosts it as
the type `cron`. ADR 0018 lists "schedule" among the ready-made actors that come later, and the `cicada-actors` design
said the schedule "is an actor, and it stays later".

Outside, recurring jobs were the most requested feature of Solid Queue, and Oban, Sidekiq and River sell cron that
is durable or that an operator edits at runtime. In cicada both come free with an actor's state. The research behind
this proposal (the scheduler, rate limiter, debouncer and batcher, their precedent and their traps) was done on
2026-11-12.

## What Changes

- A new ready-made actor `SchedulerActor`, type name `cicada.scheduler`, one instance per schedule (the key is its
  name). On each slot of its `Schedule` it sends a `StoredMessage` to a target address.
- Its state holds the schedule, the target, the message, a retry policy for the delivered message, the id of the
  armed tick, the next slot, a count of runs and the time of the last one. A failed delivery is a failed message of
  the scheduler, shown to its watchers.
- Its messages: `Start` (safe to repeat), `Run { slot }` (with no slot: runs now and keeps the slots), `Update
  { schedule, target, message, retry }` with each field optional; and its own `Tick`, which counts only when it comes
  from itself.
- A tick arms the next tick and sends itself a `Run` with its slot and the crate's shared retry policy; the `Run`
  sends to the target, with the header `cicada-slot`. A target that is absent fails that run alone, and the chain goes
  on, as in the topic.
- Slots missed while no node ran, or while the scheduler was paused, are skipped; a late tick fires once. That is the
  rule `Schedule` already has; the guide states it.
- Pause, resume and removal are the core's own `pause`, `resume` and `delete`, with no messages of their own.
- A slow target never runs two deliveries at once (G1); they wait in its mailbox.
- Its contract is pinned in `docs/reference/contracts`; the actors guide gets a section with doctests.

The simplest version: `Schedule` as it is (`every` and `daily`, in UTC), and the cron example's chain with a target
and a stored message in place of the host's trait.

## Capabilities

### New Capabilities

- `scheduler`: a ready-made actor that sends a stored message to a target on each slot of a schedule, survives an
  absent target, skips missed slots, and is paused, resumed and removed by the core's control.

### Modified Capabilities

None.

## Out of scope

- Cron expressions and time zones. Nobody has asked for them, and they need a cron parser and a time zone database;
  the first request means `cicada.scheduler.v2`.
- A rule that skips a run while the last one still runs. It needs a notice of success that the core does not give.
- Catching up missed slots, a backfill, an end date or a limit on runs, and jitter.
- Changes to the core.

## Questions for the design

- Does a new ready-made actor need an ADR? The project rule says a new wire format needs one, and a new contract is
  one. The answer holds for the actors after this one.
- A node refuses to start when a contract changes, even by a field with a default. Which fields must the first
  contract hold, so that a later version needs no new type name?
- Does `examples/src/cron.rs` stay as the worked example of a timer chain, and does the sample service move to the
  scheduler?

## Order

After `value-depth-limit`, before `rate-limiter-actor`. The first of four small actors that only decide when a message
moves on: scheduler, rate limiter, debouncer, batcher.

## Impact

- `crates/cicada-actors` (a module `scheduler`, its tests on virtual time, `tests/wire_formats.rs`, the table in
  `src/lib.rs`), `docs/reference/contracts` (a new file), [docs/guide/actors.md](../../../docs/guide/actors.md) (its
  tables, its count of the types, the section "Schedules") and its doctests, `docs/architecture.md` (the row of
  `cicada-actors`), `AGENTS.md` (the map), `README.md` (the table of actors), `CHANGELOG.md`, and `sample-service`
  (the catalog, `tests/contracts/` and `tests/contract-texts.json`).
- Hosts: no host has asked for it. Chorus and Karakuri record no request; the sample service hosts the scheduler next
  to its `cron` type.
