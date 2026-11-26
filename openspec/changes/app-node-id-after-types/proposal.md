# Proposal

## Why

A host sets `NodeConfig::node_id` before `cicada-app` reads `--types`, so a node that does not host the container type
still takes the runner's placement identity instead of a random node id (Factory issue 0022; request from the Factory
session, 2026-11-26, its epic 0008).

## What Changes

- `cicada-app` lets the host decide the node config after the types are known: for example a hook that gets the hosted
  types and returns the `NodeConfig`, or the node id chosen after `--types` is read.

To decide in the design: the hook's shape, how it combines with `--node-id`, worker nodes over HTTP.

## Order

After `placement-full-notice`, before `tray-menu-bar`.

## Impact

`crates/cicada-app` (`App`, settings). Additive.
