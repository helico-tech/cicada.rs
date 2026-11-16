# 0052. Actor placement by claim-owner identity

Date: 2026-11-16

## Status

Accepted for pinned-actors after Opus 5.5 review and resolution of its notes under the owner's autonomous design delegation. Qualifies store requirement S8 and extends
creation/view/completion wire forms. Builds on ADRs 0019, 0024, 0025, 0037, 0039, 0047 and 0048.

## Context

A host with a local resource needs particular actor instances claimed under its identity. Hosting a type on only
one node cannot express one instance per machine. Node heartbeats/labels are observations, not authenticated identity.
Remote workers already supply a NodeId as claim owner without registering a node row.

## Decision

- Give an actor an optional immutable pinned_to NodeId, chosen at creation and separate from its address/parent.
  It need not name a current node row. Default creates remain unpinned; children do not inherit placement.
- Filter claims and next-due reads by requested owner before limits or claim mutations on every backend. Local
  nodes and remote workers use their execution owner id, never the HTTP server id. Existing fencing remains.
- S8 liveness additionally requires a matching executor continuing to poll the actor's type. Without it, pending
  work remains pending without new attempts/lost claims or automatic failure/reassignment. Node status is not a gate.
- Preserve idempotent create of existing actors, including their original pin; retain foreign-child rejection.
  Conflicting pin/child choices for one address within a turn are invalid. Delete/recreate is a separate actor life.
- Add CreateOptions variants and Turn::node_id, plus optional pinnedTo on create/remote-create and actor snapshots.
  Omit absent pin in request/effect serialization to retain old portable fixture bytes. Expose pin in operator views.
- Append SQL migration 8 with nullable placement and a partial placement/type/due index; old rows remain unpinned. All old direct-store runtimes must stop
  before migration/restart because already-open old claimers ignore pins. Upgrade servers before pinned wire requests.

## Consequences

The host must assign stable, unique node/worker identities and restrict trusted executor access. An id is not proof
of machine location; duplicate ids can execute pins on different machines. Pinning neither stores live handles nor
fences arbitrary outside effects. A random id on restart leaves old pins waiting, deliberately. The process example
and node-down alerts follow separately. G1-G6 and the other store guarantees retain their existing meanings. The docs
owner integrates architecture/S8/upgrade guidance and any broken examples from the builder's handoff before landing.
