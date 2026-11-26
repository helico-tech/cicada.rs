# 0068. Asks in the placement pool

Date: 2026-11-26

## Status

Accepted for placement-asks. Builds on ADR 0067 and ADR 0058.

## Context

Work asks the placement pool (ADR 0067) for a place with a set of needs (labels). The permit pool (ADR 0058) grants its
waiting requests strictly in order, which is right for one interchangeable capacity. With needs per ask, a strict order
lets one ask for a rare label block every ask behind it, even when their places are free.

## Decision

- `Ask`, `Release` and `Cancel` follow the permit pool's protocol: grant numbers that only go up, a release that must
  name its grant, a repeated ask answered again, an answer in the ask's commit, a grant from the line sent in its own
  turn and freed when it was never received. Answers are `PlacementAnswer { pool, holder, kind, grant?, node?,
  reason? }`; refusals name `noNode`, `lineFull` or `poolFull`.
- A grant goes to a live node with every need and a free place, the most free places first, then the smaller node id.
- An ask no node could ever meet is refused at once; one only silent or full nodes could meet waits in a bounded line.
- **The line is served in order, skipping asks that cannot be placed now.** A waiting ask whose needs no node has any
  more stays until `Cancel`.
- Holders are capped per pool (at most 1000), as permits are.

## Consequences

A common ask is not held up by a rare one, at the cost of strict fairness: an ask for a rare label can wait while later
asks pass it. Hosts that need a deadline cancel a waiting ask themselves.
