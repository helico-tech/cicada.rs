# Proposal

## Why

Factory wants one business work id (for example `jira:ABC-123`) on every message of a ticket's life: through actors,
flows, timers and the ready-made actors, over days. Cicada does not carry one by itself: a send has only the
correlation id and headers its sender sets. Observers can add headers (`TurnStart::add_header_for`), but each
consumer must write the same copying observer. The scheduler and the container actor drop the correlation id that
topic, permits, job graph and flows keep. The owner agreed on 2026-11-28 (F2).

## What Changes

- A ready-made observer that copies chosen headers (for example `work-id`) from a turn's message to every send of the
  turn, timers included. It runs on nodes and on HTTP workers, as every observer does.
- The scheduler and the container actor keep the correlation id like the other ready-made actors: the scheduler sends
  each slot with the correlation id of the message that set the schedule.

To decide in the design: where the observer lives (`cicada::observer`, `cicada-actors`, or `cicada-otel`); whether the
scheduler also keeps the copied headers of the message that set the schedule (a state field, so a contract change);
what the docs say about fan-in actors (permits, placement, batcher, debouncer, a shared scheduler), which send in a
turn caused by another work item, so a copied header names that item: there the work id belongs in the payload.

## Out of scope

- Keeping OpenTelemetry baggage through timers.
- Any header Cicada adds or keeps without an observer the host chose.
- Labels or environment for containers from headers: they come from the job spec.

## Order

After `flow-request-target`. Before `topic-delivery-retry`.

## Impact

Crates: cicada, cicada-actors, cicada-containers.

The observer and its tests; `crates/cicada-actors/src/scheduler.rs`; `crates/cicada-containers/src/actor.rs`; their
contracts in `docs/reference/contracts` if a state changes. Docs handoff: the core guide's observer section, the
design guide (where a work id belongs) and the actors guide's notes on correlation.
