# 0072. One live holder per resource

Date: 2026-11-26

## Status

Accepted for placement-resource-key. Extends ADR 0069 and ADR 0071, which are not edited.

## Context

Two nodes on one machine, each with its own identity, can offer the same resource, such as one Docker daemon, so its
places count twice. Node ids and instances differ, so ADR 0069 and ADR 0071 do not see it.

## Decision

- An offer may name a resource key (Docker's daemon id from `RuntimeInfo::id`). One live node holds a key: an offer
  whose key a different live node holds changes nothing for the offering node, marks it in conflict with the
  `resource`, and sends one `conflict` alert per mark.
- When the holder is silent or gone, the next offer with the key takes it. A mark with `resource` ends one silence time
  after the last conflicting offer.

## Consequences

A daemon shared by two nodes counts once. Hosts that want this pass a key; offers without one behave as before.
