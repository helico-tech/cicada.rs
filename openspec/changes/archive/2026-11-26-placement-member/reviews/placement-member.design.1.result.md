Verdict: not yet

## Blocking

1. **A refused offer cannot send an alert.** Design Decision 3; spec "another machine cannot take it". A permanent error marks the turn `failed` with no effects (architecture.md, outcome table). So the turn can neither send `conflict` nor record that it was sent. Fix: accept the turn and ignore the offer, then queue the alert. Add a node field (such as `conflict: <instance>`) for "once", and say when a conflict ends. Name the state and contract change (`cicada.placement.json`). This also clashes with `openspec/specs/placement-pool` ("each offer SHALL replace…"), so it needs a MODIFIED delta.
2. **The node id can split.** Decision 2. `--node-id` / `CICADA_NODE_ID` override the host's `NodeConfig` (`cicada-app/src/settings.rs:274`). Then pins use that id while the member offers `identity.node_id()`. Fix: the member offers `system.node_id()`.
3. **Undecided error paths.** Decision 2 does not say what happens with a corrupt or unreadable identity file, or a missing folder. It must not silently make a new identity. Return an error and add a scenario. The takeover after silence and the refusal of a zero interval also have no spec scenario.
4. **Size: three behaviours** (the loop, the identity, the pool's conflict rule). Split the conflict rule into its own pool change, before this one.

## Notes

- The `Offer.instance` rustdoc says "drawn at each process start". Update it.
- A refused machine keeps sending: about 5,760 failed messages a day.
- "Below the pool's silence" cannot be checked by the member. Make it the host's duty.
- `run` should end on `stop` while a probe hangs.
- Two machines under one id also share leases and the node row. Name this in the rustdoc limit.

## Questions

- CM-Q1: Keep the conflict check as its own change? My pick: yes.

## Not checked

Handoff doctests, skill routes, the error types of `ActorSystem::send`.
