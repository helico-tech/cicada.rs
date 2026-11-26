# 0069. A second machine under one node id

Date: 2026-11-26

## Status

Accepted for placement-conflicts. Extends ADR 0067 (its offer rule and notice kinds), which is not edited.

## Context

ADR 0067 lets each offer replace the node's last one. Two machines offering under one node id would then replace each
other's entry at every offer, and the pins of granted children would land on whichever claims first. Offers carry an
`instance`, kept across restarts by the member (`placement-member`), so the pool can tell two machines apart.

## Decision

- An offer whose instance differs from the recorded one while the node is `live` changes no entry field. Its turn
  succeeds: it marks the node `conflict: {instance, at}` and, for a new mark, sends one alert of the new kind
  `conflict` in a turn of its own. Later conflicting offers, from any other instance, only update the mark.
- The mark ends when the node goes `silent`, or when the recorded instance offers after no conflicting offer for one
  silence time. A silent node's entry is taken over by the next offer, whatever its instance.

## Consequences

An operator learns of a duplicated node id once, and the first machine keeps its places. A cloned disk with the
identity file copied looks like one machine. Two machines under one id also share leases and the node row in Cicada's
store: the pool only reports it.
