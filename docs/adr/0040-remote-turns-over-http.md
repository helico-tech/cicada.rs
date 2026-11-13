# 0040. Remote turns over HTTP

Date: 2026-11-13

## Status

Accepted for `remote-turns-over-http`. The owner chose unchanged Rust actors first over a language-neutral protocol,
with any server sharing the database able to renew and finish. Technical decisions were delegated to Opus 5.5.

## Context

ADRs 0037–0039 define core leasing, explicit completion reports and a Rust Worker over TurnSource. HTTP must preserve
those boundaries across lost responses and load balancing without exposing Backend or creating server-local sessions.

## Decision

- Add four full-router POST operations: /leases, /leases/renew, /leases/release and /leases/complete. Put actor/epoch
  in JSON, so clients echo canonical addresses without path re-encoding. Host authentication covers the root and
  child paths; epochs remain non-secret fences, and workers have trusted write authority.
- Return core alternatives in 200 JSON tagged by result; release is 204. LeaseLost, ContractMismatch and Rejected
  cannot be inferred from a proxy's error status. Existing problem mappings handle operation errors.
- Publish transport views in cicada::api. A lease contains all execution message fields, state/version, store times
  and TTL. Message fields, including nullable policies and counters, are required and strictly decoded. Unknown
  response envelope fields are ignored; unknown variants and invalid policies are errors, not defaults.
- Requests reject unknown envelope fields and duplicate decoded JSON keys at every depth. Arbitrary state and payload
  remain data. Preserve existing body-size and JSON recursion limits; no new schema validation of completion effects.
  Valid reports reach core storage validation and its actual completion outcome. Existing routes keep their parsing.
- Use exact i64 JSON numbers and existing instant/ISO duration strings with nanosecond precision. Non-Rust consumers
  must preserve the full integer range. Body addresses use the ordinary parser and canonical serialization.
- Forward completion once. Rejected means no fallback commit was made and requires the caller to retain the new
  nonretryable failure report. Unknown responses, HTTP rejection and lost epochs never authorize release or hidden
  handler execution. Server-derived sender/caused-by and atomic effects remain core responsibilities.
- Implement TurnSource for ActorClient without another management facade. Bound the four turn calls with a positive
  turn_timeout, default 10s, through response body reading; shorter configured HTTP timeouts still apply. Reuse the
  existing type registration/list routes and fetch conflict diagnostics once without re-registering.
- Classify actual HTTP statuses: invalid request statuses are Invalid, transient gateway/rate/timeout/network failures
  Unavailable, and authentication/wrong-route/protocol/internal failures Fatal. Never trust Problem.status over HTTP.
  A malformed successful response is Fatal; only an explicit valid result establishes completion or lease loss.
- Add no application retry in the adapter. A supplied reqwest client retains its policy: worker hosts disable redirects
  and custom mutation retries. Default redirect behavior can resend requests; examples explicitly disable it rather
  than changing the behavior of every existing management client.

## Consequences

- A server needs no copy of a worker's handler and no sticky session. Stopping one server leaves renew/complete through
  another available while the store is open. No node presence, database migration, receipt store or new auth scheme.
- Whole reports must fit host and proxy body limits. Oversized or malformed completion is abandoned without a hidden
  fallback; later lost-claim recovery may repeat external effects until the configured claim limit, or indefinitely
  when it is disabled. Idempotency keys remain necessary. A separate larger completion limit is not added here.
- A lost lease response can leave leases to expire. Old deeply nested states may be unreadable through the response
  parser and stop the worker. Fix the data/configuration before restarting a loop of failing workers.
- A registration conflict followed by a GET is not an atomic snapshot; the returned existing definition is diagnostic.
  The adapter recomputes the requested hash, preserving registration semantics for Contracts read from the registry.
- Verification covers exact wire round trips, parser limits, two peers, cancellation, explicit failure transitions and
  lost acknowledgements. Runtime timing remains tested on virtual time; OpenAPI and guide examples are checked.
