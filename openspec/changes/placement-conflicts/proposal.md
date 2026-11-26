# Proposal

## Why

From the design review of `placement-member` (2026-11-26): two machines offering under one node id would replace each
other's entry at every offer, and pins of granted children would land on whichever claims first. The pool should
notice a second instance under a live node's id and tell its operator, once.

## What Changes

- An offer whose instance differs from the node's recorded one while that node is `live` is accepted as a turn but
  ignored: the entry stays, and the pool marks the node `conflict` and queues one alert of the new kind `conflict` (in
  its own turn, as other alerts), so the alert and its once-only mark survive. Once the node is `silent`, an offer with
  a new instance takes over the entry (a replaced machine) and clears the mark.
- The spec rule "each offer SHALL replace the node's last one" is modified for this case. A node field for the mark and
  the alert kind change the unreleased pool contract (ADR 0067: the line ships as one release).

To decide in the design: how long the mark lasts, whether the ignored offers are counted, what the limit for cloned
identity files says (two machines under one id also share leases and the node row).

## Order

After `placement-drain-and-gone`, before `placement-member`.

## Impact

`crates/cicada-actors/src/placement.rs`, its contract reference and samples. Additive inside the unreleased line.
