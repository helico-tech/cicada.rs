Verdict: not yet

Round 1 fixes hold: the lock lives in an `Arc` kept by the member, no host name is stored in the file, and the error paths are decided. The signal works. Both machines upsert the same row by id every 10 s (`heartbeat_node`, `ON CONFLICT (id) ... hostname = excluded.hostname`), and the row's id is the identity's id by default. The row always holds the name of whichever machine wrote last, so the other machine sees the clash in most rounds, and the pool gets reports.

## Blocking

1. **The core test has no way to run.** Design, Tests: "a node row written with another host name". `NodeDetails` is `pub(crate)` and `heartbeat_node` is on the private `Backend`. A second system on the same machine writes the same host name. Decide how a test gets a row with another host name: a `testing` hook in `cicada`, or a host-name override in the node's config.
2. **Which host name is compared.** Decision 5: `node_details` looks up the name once, at start. Calling `local_hostname()` every round reports a false duplicate, for good, after the machine renames itself (macOS does this when the network changes). Compare against the value the system writes instead, through a new getter such as `ActorSystem::hostname()`. Then `usable_hostname` can stay private.
3. **ADR 0069 is contradicted.** Its Consequences say "A cloned disk ... looks like one machine; the pool only reports it". Its mark ends when the node goes silent, or when the recorded instance offers. The design adds a message and a mark field (a contract change) and its own end rule. Write a new ADR that extends 0069, and say how both end rules combine.

## Notes

- `ActorSystem::nodes()` pages through every running turn in the store, in every member, every round. Read only the member's own row, or name the cost.
- A copied disk keeps its host name (`/etc/hostname`), so the Why's own case goes undetected. Say so plainly in the handoff.
- On one host with a shared volume, a rolling deploy's new container fails with `Locked` until the old one stops. Name this in the handoff.
- `round` returns early when the probe gives `None`. Say whether the check runs first.

## Questions

- Q1: Is "a copied disk with the same host name goes undetected" acceptable for Factory?

## Not checked

Windows locks, and the later proposals beyond their order.
