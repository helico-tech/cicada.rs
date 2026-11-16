# 0054. Node-down alerts through a monitor actor

Date: 2026-11-16

## Status

Accepted for node-down-alert after Opus 5.5 review and resolution of its notes under the owner's autonomous design delegation. Adds a ready-made actor/event contract;
extends the observability convention of ADR 0048 without changing leases, pinning or the node-table schema.

## Context

Node rows already expose heartbeat age and are pruned after a day. Polling them from every host with an independent
send can duplicate alerts or lose a send between recording and delivery. Core store election/outbox machinery would
duplicate the serialization and atomic sending already available to an actor. Topic already manages fanout.

## Decision

- Provide one optional unpinned singleton monitor per store, with an injected read-only node snapshot source and
  fixed Topic destination. ActorSystem is the initial adapter; no NodeConfig activation or new node-row writer.
- Poll every ten seconds through guarded durable self messages, with five-second reads and at most 1000 rows of
  persisted work. Read errors/timeouts/overflow retain history, report a bounded problem and rearm. Rearm repairs a
  lost timer without clearing history. A broken Topic follows ordinary failed-turn semantics and needs repair/Rearm.
- At the existing two-minute dead threshold, commit one Topic Publish and an id/startedAt/lastSeen watermark together.
  A newer heartbeat/new row can start another observed episode; absent/non-dead rows clear their watermark.
- Publish NodeDown with node identity, row timestamps, sample time and optional details. Topic owns subscriber
  delivery; no core watch notification or recovery message. These are observations, not physical-death assertions.
- Keep current clean-stop removal, daily pruning and duplicate-id semantics. Missed/pruned rows have no history;
  sampling can race a fresh heartbeat. No automatic action changes claims, pin placement or outside-effect fencing.

## Consequences

One actor commit supplies the coordination instead of a new store operation/migration. An absent/pinned/stopped
monitor cannot alert, and deployments must host it with the same source store and keep its Topic usable. Persistent
work is bounded, but the existing node API reads the full view including running turns. Consumers may watch monitor
failures and inspect lastProblem, and use the existing heartbeat-watch for workloads that send explicit Beats.
Invalid/excessive snapshots deliberately suppress the whole sample while polling continues; busy reads may time out
indefinitely. Operations must inspect lastProblem as well as watch terminal monitor failures. The docs session updates the node-view description from display-only to observability, retaining its exclusion from
claim authority, and explains alert limitations from the handoff. Existing actor contracts remain unchanged.
