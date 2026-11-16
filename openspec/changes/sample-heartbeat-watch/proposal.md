# Proposal

## Why

The sample catalog does not host HeartbeatWatchActor, added in 0.18.10. The owner requested that addition at low
priority. Opus 5.5 identified it as independent of the HTTP executor in sample-worker-mode; splitting
it keeps both changes small. The owner delegated design and scope choices for this batch.

## What Changes

- Add the existing HeartbeatWatchActor to the shared sample catalog, available to the sample service and executor.
- Register its existing contract and check it in sample contract snapshots and text digests.
- Add a focused smoke for explicit creation and Beat, without creating watches automatically at startup.
- Hand off the catalog guidance to the docs session.

## Out of scope

Changing heartbeat-watch behavior, wire formats or existing contract hashes. Automatic instances, Beats or node
monitoring. NodeDownMonitor hosting needs an injected source and remains outside this sample catalog change.

## Order

After `sample-worker-mode`. No following change.

## Impact

sample-service/src/catalog.rs, sample contract snapshots/text digests and targeted sample tests; docs handoff for
the App guide's sample catalog guidance. No core/App behavior, new binary, Docker topology, schema or guarantee
change. The existing executor and image inherit the catalog addition. No new ADR is needed.
