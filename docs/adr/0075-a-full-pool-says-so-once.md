# 0075. A full pool says so once

Date: 2026-11-26

## Status

Accepted for placement-full-notice. Extends ADR 0067 (decision 5 of the change's design), which is not edited.

## Context

ADR 0067 set a pool's node limit (`maxNodes`) but not what happens past it. An offer from a new node to a pool that
knows `maxNodes` nodes was refused for good: its member's offer became a dead letter at every interval, and nobody was
told that the pool is full.

## Decision

- A full pool accepts an offer from a node it does not know and ignores it: the turn commits, the node is not added and
  no entry changes. The resource check of ADR 0072 comes first, so an offer of a key another live node holds is still a
  contest.
- The pool keeps one pool-level mark (`full`: the node of the last ignored offer and its time) and sends one `full`
  notice to its alert target when the mark is set; later ignored offers, from any node, only move it. The mark ends one
  silence time after the last ignored offer, removed by the check timer, or at once when an offer adds a node, so a pool
  that fills up again alerts again.
- One mark per pool, not per node: the operator's action is the same for one node or many (raise `maxNodes` with
  `Configure`, or declare gone nodes `Gone`), and a list per node would need its own bound past which offers are refused
  again.

## Consequences

A full pool raises one alert per full period instead of a stream of dead letters. The notice kind enum
(`PlacementNoticeKind`) gains `Full` and becomes `#[non_exhaustive]`; an alert target on an older node fails the `full`
alert during a rolling upgrade.
