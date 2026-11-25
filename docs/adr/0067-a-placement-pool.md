# 0067. A placement pool

Date: 2026-11-26

## Status

Accepted for placement-offers. Builds on ADR 0052 (pinned actors) and ADR 0058 (the permit pool). Later changes of
the placement-pool line add asks, holders, drain and the member.

## Context

Work that must run on a particular kind of machine (Factory's containers on hosts with Docker and a processor kind, a
git mirror per machine) needs a place on a node that offers it, and a node id to pin a child to. The node table is for
display and is not a basis for placement (ADR 0054). The permit pool counts one capacity and keeps its logic private.

## Decision

- A ready-made actor `cicada.placement/<pool>` in `cicada-actors`, created by the host with its settings (silence time,
  node limit up to 1000) in its state. Nodes offer `instance`, `labels` and `places` with `Offer`, a renewal that
  replaces the node's last offer and records when it was heard.
- The pool's own timer (a self-only `Check`, re-armed by `Rearm`) marks a node not heard from for its silence time as
  silent: no new places, and one `silent` notice to the pool's alert target per silence, sent in a turn of its own;
  its next offer makes it live with a `back` notice. Bounds: 32 labels each, 1000 places per node.
- The line (offers, asks, holders, drain and gone) is released as one minor version.

## Consequences

Placement rests on what members say, renewed, not on the node table. A member must offer at an interval shorter than
the silence time, under a node id its node pins under. Asks, grants and holders come with `placement-asks` and
`placement-holders`; draining and declaring a node gone with `placement-drain-and-gone`.
