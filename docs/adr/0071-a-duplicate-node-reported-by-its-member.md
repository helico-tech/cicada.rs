# 0071. A duplicate node reported by its member

Date: 2026-11-26

## Status

Accepted for placement-identity-lock. Extends ADR 0069, which is not edited.

## Context

ADR 0069 tells two machines apart by their offers' instance. Two machines that load a copied identity folder share
both node id and instance, so the pool sees one node (ADR 0069: "a cloned disk with the identity file copied looks like
one machine"). They do share one node row, upserted by id at each heartbeat, whose host name then flips between them.

## Decision

- Each round, first, a placement member reads its own node row; a host name that differs from the one its node writes
  makes it skip its offer and report `Duplicate { node, host }` to the pool.
- The pool sets the conflict mark of ADR 0069 with the reported `host`, sends one `conflict` alert per mark, and gives
  the node no new places while marked. A mark with `host` ends one silence time after the last report; ADR 0069's rules
  end a mark without `host`; a mark set both ways ends when both would.
- `NodeConfig::hostname` sets the host name a node writes (the machine's own by default), and `ActorSystem::hostname()`
  returns it.
- `NodeIdentity` locks its folder, so one machine runs one process per identity.

## Consequences

A copied identity folder on machines with different host names is reported and placed on no further. Clones that keep
their host name are still not told apart. Containers that redeploy under new host names are not affected, as only one
machine writes the row at a time.
