# Proposal

## Why

The node table shows a node as dead once its heartbeat is 2 minutes old, but nothing tells anyone. An operator has to
look. With `pinned-actors`, a dead node also means that the actors pinned to it stop: their messages wait until the
node comes back. Hosts need to hear about it, the way watches tell a watcher that an actor failed or stopped.

## What Changes

- Add the optional singleton NodeDownMonitor in cicada-actors. It reads the existing node view through an injected
  source and treats 120 seconds without a heartbeat as dead, matching the existing node status threshold.
- Publish NodeDown events through an explicitly configured Topic. Existing Topic subscriptions and delivery handle
  recipients; one monitor turn commits the publication, dedup watermark and next poll together, using normal leases.
- Include the node id, row timestamps, detection sample time and optional hostname/version/label. No recovery event
  in the first version. A newer heartbeat or recreated row permits another observed silence alert.
- Retain the existing clean-stop removal and daily node-row pruning. This is sampled observability, not a complete
  death history: absent/pruned rows and all-cluster outages cannot be reported retrospectively with certainty.
- Provide virtual-time behavior/contract tests, a checked example and consumer guidance through the docs handoff.

Decided under the owner's delegation (2026-11-16; Opus 5.5 reviews):

- Use a ready-made actor and Topic rather than new core store election, notification tables or NodeConfig recipients.
- Fixed ten-second durable polling, five-second reads and 1000-row persisted-work cap; errors retain watermarks and
  rearm with a bounded diagnostic. Missing Topic follows normal rollback/failure and operator repair/Rearm.
- Observe retained dead rows immediately after startup; do not turn missing rows into death claims. No recovery alert.
- Keep heartbeat-watch for explicit Beats from workloads/devices. NodeDownMonitor is for the native node table and
  never grants authority to claim/migrate actors or fence their outside effects.

## Out of scope

Alerts on suspect nodes. Alerts on workers, which have no node row. Paging or e-mail: the target is an actor, and the
host decides what it does. Moving the actors of a dead node.

## Order

After `pinned-process-example`. Before `sample-worker-mode`.

## Impact

crates/cicada-actors node_down module, public source/actor/message/state types, generated contract reference,
wire/behavior tests and examples/src/node_down.rs. ADR 0054 and handoffs for the actors guide, core node-view linkage,
operations, architecture observability language and both skills' routes. No store operation/migration, heartbeat
protocol, core watch kind or automatic sample-service activation. Existing node-row and Topic semantics stay intact.
