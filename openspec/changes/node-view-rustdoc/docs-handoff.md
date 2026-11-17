# Docs handoff for node-view-rustdoc

Version: 0.20.5. Refresh the dependency pins of `docs/how-to/install.md` once it is published. Not breaking: only the
doc comment of `ActorSystem::nodes` changes. Behaviour, public signatures, actor contracts, wire formats, the store
schema and the guarantees are those of 0.20.4.

## What changed

- The rustdoc of `ActorSystem::nodes` (`crates/cicada/src/runtime/system.rs`) keeps its first sentence (the node
  table by id, status from store time, running turns) and replaces "Display only: leases, not this table, decide who
  runs what." with:

  > For observability, including node-down alerts. The node table grants no authority to claim, move or fence
  > actors; leases decide who runs what.

- It describes a use of the returned view; calling `nodes` publishes no alert and starts no monitor.
- No test, example, guarantee row, heading or skill route changes. No guide change is needed: the guides, the
  architecture and ADR 0054 already describe the node table as observability with no claim authority.

## Blocking

None.

## Archive note for the build loop

When archiving, align the Purpose of `openspec/specs/node-view/spec.md`, which still says the view is display only,
with its existing normative requirement: observability, including node-down alerts. Keep the lease boundary and every
behavioural requirement. This is a spec editorial correction, not a guide change.

## Status

- Proposed text: none for the docs session; no handwritten prose is required.
- Ran: `cargo doc -p cicada --no-deps` (clean); the remaining checks and the gate are listed in the build report.
