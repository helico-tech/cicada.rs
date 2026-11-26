# Proposal

## Why

Two machines that load the same `placement-identity.json` (a copied folder or disk) have the same node id and
instance; the pool sees one node, and runs placed there may start on either machine. Factory's old pool caught this,
and its owner decided Factory builds no guard of its own (request from the Factory session, 2026-11-26, its epic 0008;
blocks Factory's merge).

## What Changes

- `NodeIdentity` holds an exclusive file lock on its folder while the node runs, so a second process on the same folder
  fails to start.
- The member and the pool notice two machines under one id and instance: for example the member stops offering, and
  the pool sends a `conflict` notice, when the node table shows the node's id with another host name.

To decide in the design: the lock (an OS file lock, released at process end), what tells two machines apart (the node
table's host name, or a machine id kept next to the identity), what the member does when it finds itself duplicated.

## Order

After `gate-openspec-reuse`, before `placement-resource-key`.

## Impact

`crates/cicada-actors` (placement member, identity), perhaps the pool and its contract. Additive.
