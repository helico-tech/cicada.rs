# 0072. One live holder per resource

Date: 2026-11-26

## Status

Accepted for placement-resource-key. Extends ADR 0069 and ADR 0071, which are not edited.

## Context

Two nodes on one machine, each with its own identity, can offer the same resource, such as one Docker daemon, so its
places count twice. Node ids and instances differ, so ADR 0069 and ADR 0071 do not see it.

## Decision

- An offer may name a resource key (Docker's daemon id from `RuntimeInfo::id`). One live node holds a key: an offer
  whose key a different live node holds changes nothing for the offering node; the pool records the contest in a
  pool-level list (key, node, time of the last such offer) and sends one `conflict` alert when it is recorded; the
  contest ends one silence time after its last offer. The node entries and ADR 0069's mark do not change.
- When the holder is silent or gone, the next offer with the key takes it.

## Consequences

A daemon shared by two nodes counts once. Hosts that want this pass a key; offers without one behave as before.
