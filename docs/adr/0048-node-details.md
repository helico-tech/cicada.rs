# 0048. Node details in the node table

Date: 2026-11-15

## Status

Accepted for `node-details`. Supplements ADR 0015's display-only node table; no execution guarantee changes.

## Context

A random node id and a list of hosted types do not tell an operator which machine a dead node ran on or which
Cicada version it used. Stable ids can be configured today, but they do not provide machine metadata. Hostnames
and labels are useful for display and must not silently become an identity or authorization mechanism.

## Decision

- Node rows and their public views gain optional hostname, Cicada version and label fields. The version is the
  compiled core crate's version; an application may put its own version or other context in its label. No pid,
  API address, free-form label map or remote-worker row is added.
- NodeConfig has an optional label, default None. A present value is nonblank after trimming, NUL-free and at most
  200 Unicode scalar values. Valid text is kept verbatim. Existing configuration validation reports bad labels.
- At start, after initialization and before background tasks spawn, capture the OS hostname with whoami 2 (only its
  `std` feature) and the core version. Failed or non-UTF-8 lookup, blank, NUL or more than 200 characters becomes no
  hostname, with one debug diagnostic. Do not truncate, use a shell/environment fallback or add a hostname override.
  Failure does not prevent start. Captured details are fixed for that system's life.
- Every heartbeat carries the captured details through a private NodeDetails argument and replaces the row's three
  metadata fields, including absent values. Store time, started_at preservation, ten-second heartbeat, health,
  expiry and clean-stop rules stay as before. Start still schedules rather than awaits the first heartbeat write.
- Append SQL migration 7 with three nullable text columns: hostname, cicada_version, label. Old rows have NULL.
  Old running writers name their columns and continue; their inserts leave metadata absent, and their updates do
  not clear metadata left by another writer. New writers replace all fields. Memory follows the new semantics.
- HTTP names are hostname, cicadaVersion and label, omitted when absent. New decoders default missing fields to
  None; old tolerant readers ignore added fields. Node detail uses the same flattened view. Existing host access
  control covers these identifying details; the metadata supplies no authenticated claim about a machine.
- App adds --node-label and CICADA_NODE_LABEL with flag, nonempty variable, host, then default precedence for serve
  and direct-store workers. A worker over HTTP has no node row: an explicit flag is a usage error, and the variable
  and host label are ignored. No WorkerConfig or remote lease identity change is made.
- The label variable is read without the normal trimming helper: an empty value is unset, whitespace-only is
  invalid, and valid leading/trailing spaces are retained, just as for a flag or host configuration.
- Node tables show HOSTNAME; wide output adds VERSION and LABEL alongside HOSTED. Missing fields use the usual `-`.
  Metadata cells escape newline/carriage return/tab as `\n`/`\r`/`\t` and other Unicode control characters as
  `\u{hex}`, preserving printable Unicode. The shared renderer supplies the TUI; storage and JSON/YAML keep the text.
- Random ids remain the default. Configure a stable id explicitly where desired, and ensure uniqueness among live
  nodes. No duplicate-id check or fencing is added: a check followed by a heartbeat would race. Duplicate ids may
  overwrite metadata or remove each other's row on stop; an old writer may leave stale details. A changing hostname
  is not proof of a collision. Placement and identity safety belong to the separate pinned-actors design.

## Consequences

This is a breaking Rust API release because public structs gain fields, even though the HTTP additions are optional.
Those public structs are NodeConfig, NodeInfo and NodeView; NodeRecord remains crate-private.
Use 0.19.0, or the next breaking minor if the release line advances first. Do not add non_exhaustive and invalidate
existing `..NodeConfig::default()` construction. In-repo callers are updated; known protected doctests keep compiling.

The migration is additive for already-open schema-6 nodes, including prepared statements that name columns. A
schema-6 binary cannot open the migrated database, so an old node cannot restart after the first new node migrates
it; downgrade is refused. Replace old nodes during rollout. Upgrade tests exercise an old connection across the
migration and legacy insert/update shapes. No old migration is rewritten.

The node table remains best-effort display data, with dead rows retaining their last details until expiry. A
container hostname may be unstable. Initialize-only systems and remote workers still have no row; started
send-only systems and direct-store workers do. The hostname dependency becomes direct even in non-PostgreSQL builds.
The docs session receives the rollout limits and stable-id guidance through the builder's handoff.
