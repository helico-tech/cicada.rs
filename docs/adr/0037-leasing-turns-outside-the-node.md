# 0037. Leasing turns outside the node

Date: 2026-11-13

## Status

Accepted for `lease-turns-in-core`. The owner delegated technical design decisions to an Opus 5.5 adviser and chose
existing Rust actors over HTTP, with no affinity to the server that issued a lease.

## Context

ADR 0036 lets a server know contracts without hosting implementations. Remote execution next needs public lease
operations while Backend stays private. The store already provides claims, renewal, release and epoch fencing.
This change exposes those operations deliberately; completion and the Rust worker are separate successors.

## Decision

- Add ActorSystem lease_turns, renew_turn and release_turn with public core result types. Do not publish a transport
  trait or HTTP wire model before their consuming changes. A lease contains the claim record and granted TTL.
- Select 1..100 types by name/hash and claim 1..100 actors per call. Compare all requested hashes in one registry
  read before claiming; an absent or changed type refuses the request with no claim. This is not an atomic version
  pin against delete/re-register racing inside the call. Stop old workers before replacing contracts.
- The serving NodeConfig supplies lease TTL and claim limit. Every claim/renewal returns its granted TTL. Use a
  conservative deadline based on the caller's monotonic request start and min(TTL/10, one second) margin; store times
  do not synchronize clocks. A renewal's TTL is the requested interval; its actual expiry may be later.
- Make renewal atomically preserve the maximum of the held expiry and store now plus TTL on all backends. Return
  that maximum for both ordinary and cancel-requested renewal. Otherwise a short-TTL peer could expire a long lease
  while its reply is lost and the holder still follows the old confirmed deadline, violating G1. A worker may keep
  the later of its old confirmed deadline and the new request-start candidate. Backend's signature stays unchanged.
- Record the caller's bounded, NUL-free NodeId as owner, without node heartbeats or local runtime accounting.
  Address and epoch fence renew/release, including across actor lives. There is no owner or message-id check for
  these two operations. An epoch is not a secret; these are trusted executor operations, not authorization gates.
- Retain a cleanup guard until handoff, including a detached claim returning after its caller is gone. Check the
  deadline after claim and again before handoff. Release leftovers on cancellation/late results, best effort only.
  After handoff the caller owns release; lost responses can cost lost claims. Add no receipt store or new schema.
- Filter at-limit messages with one concurrent fenced failure-commit attempt each, using the existing cause/reason,
  watcher notifications and post-commit observer order. Any unconfirmed/failed result is released, never handed out.
  This differs from local execution's retry loop and is safe because no handler ran; local policy stays unchanged.
- Lease/renew validate config without initializing local handlers. Lease rejects a stopping/stopped system observed
  at its admission checks; renew/release remain available. Stop does not drain remote leases; Store close can prevent
  cleanup. A stop racing after the final check does not recall a handed-off lease.

## Consequences

- The server can be handler-free, and any peer on the same database can renew/release. A holder must stop work
  before release and must not release after its handler ended. Complete-leased-turns will add the terminal reports.
- Renew surfaces an operator cancellation request; it does not complete the message. The future worker must
  arbitrate cancellation against handler completion and maintain its watchdog. A peer cannot shorten the held lease.
- Renew/release may still succeed after store-time expiry when no later claim replaced the held epoch. This existing
  backend behavior does not permit a worker to continue past its conservative deadline.
- External leases appear in running-turn views, not as new nodes. Reusing an existing node id can mix display data.
  Local concurrency limits do not bound the number of external leases; callers/hosts own admission capacity.
- A failed cleanup can expire and count lost. An empty batch can reflect filtered or late claims rather than no
  due work. An at-limit commit can succeed even when its acknowledgement is lost; observers then remain silent.
- Renewal behavior changes atomically in memory, SQLite and PostgreSQL, without a Backend signature or SQL schema
  change. No delivery guarantee is weakened. Conformance covers the maximum rule and withheld responses; tests
  proving existing guarantees get table references. A compliant external executor owns the same deadline/G6 duties.
