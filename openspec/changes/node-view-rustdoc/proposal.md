# Proposal

## Why

The public rustdoc of ActorSystem::nodes in crates/cicada/src/runtime/system.rs still says "Display only".
Since node-down-alert (v0.20.2), ActorSystem implements NodeSnapshotSource using this method, and NodeDownMonitor
reads the result to publish heartbeat-silence alerts. The comment therefore understates the node table's use.
The docs session raised this producer-owned source comment during node-down upkeep; the owner agreed to a tiny
separate change. No later open proposal owns this area.

## What Changes

- Correct only the ActorSystem::nodes doc comment to describe observability, including node-down alerts.
- Preserve the authority boundary: the node table grants no authority to claim, move or fence actors; leases
  decide who runs what.
- Keep the existing description of the returned node status and running turns.

## Out of scope

Behavior, public signatures, wire formats, contracts, schemas, guides and other handwritten documentation.
No change to NodeSnapshotSource, NodeDownMonitor, claim eligibility or lease enforcement. No new ADR is needed.

## Order

After `sample-heartbeat-watch` (archived, released as v0.20.4). No following change.

## Impact

The runtime source change is only the public doc comment on ActorSystem::nodes in crates/cicada/src/runtime/system.rs.
The producing change owns this comment; the docs session does not edit it. Follow the normal patch-release path
(next patch currently 0.20.5), including changelog and version metadata, with publication approved in the build loop.
At archive time, align the stale node-view spec Purpose's "display only" wording with its existing observability
requirement; this is an editorial correction, not a behavior change. The build loop starts once tasks.md is committed.
