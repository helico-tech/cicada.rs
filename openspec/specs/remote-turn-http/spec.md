# remote-turn-http Specification

## Purpose
A trusted worker outside the database network leases, renews, releases and completes turns through the HTTP API, so
it needs only a Cicada URL and the host's credentials. `ActorClient` is the worker's `TurnSource` over these routes.

## Requirements

### Requirement: Trusted workers lease turns through the full HTTP router

The full router SHALL expose POST /leases, /leases/renew, /leases/release and /leases/complete. They SHALL use the
public core operations without server-local worker sessions or a requirement to host the selected types. The read
router SHALL expose none of these operations. The host SHALL remain responsible for authentication of these writes.

#### Scenario: Lease request and runnable batch

- **WHEN** a worker posts {owner, types: {typeName: contractHash}, limit} to /leases
- **THEN** the server applies core owner/type-count/batch-limit/hash checks and returns 200 with either
  {result:"leased",leases:[...]} or {result:"contractMismatch",type,registered,requested}
- **AND** registered is a string or null, and claim-limited messages are settled by the core and not handed out
- **AND** an empty runnable batch is a successful leased result, not a long poll
- **AND** core's confirmed claim-limit failures retain their server observer notifications; HTTP adds none

#### Scenario: A complete execution lease

- **WHEN** the leased result contains a turn
- **THEN** it contains actor, epoch, message, state, version, claimedAt, leaseUntil and ttl
- **AND** message contains id, to, from, type, payload, deliverAt, correlationId, headers, seq, attempt, failures,
  lostClaims, lastError, retry, causedBy, retention and notification with their full core values
- **AND** every listed field is required, including nullable fields and zero-valued counters

#### Scenario: Renew and release through a peer

- **WHEN** a worker posts {actor,epoch} to /leases/renew through another server on the same store
- **THEN** it receives 200 with {result:"renewed",leaseUntil,ttl,cancelRequested} or {result:"leaseLost"}
- **AND** renewal preserves the core's nonshortening and expired-but-held behavior
- **WHEN** it posts the same identity to /leases/release
- **THEN** release returns 204 without a body, including repeated or stale releases

#### Scenario: Lifecycle and access

- **WHEN** an unstarted handler-free system serves a lease request
- **THEN** it may lease under its core configuration without hosting the worker's actors
- **WHEN** that system is stopping or stopped
- **THEN** new lease requests return 503 while renew/release/complete retain their core lifecycle behavior
- **AND** a peer can continue the held turn while the shared store remains open
- **WHEN** any of the four POST routes is called on the read router
- **THEN** no lease operation executes

### Requirement: HTTP carries finished reports and explicit completion alternatives

POST /leases/complete SHALL accept {actor,epoch,messageId,report} and invoke core completion at most once. A report
SHALL be returned, failed or cancelled, never a caller-selected outcome. HTTP SHALL add no retry, fallback commit,
release or server observer. The response SHALL preserve the actual core result.

#### Scenario: Returned report

- **WHEN** report is {kind:"returned",state,effects}
- **THEN** state is {kind:"unchanged"}, {kind:"stop"}, or {kind:"set",value}, with value required even when null
- **AND** effects is required and contains sends, creates, cancels, watches and controls arrays, each defaulting to
  empty when omitted, preserving their order
- **AND** a create is {address,state,child}, a watch {target,set}, a control {effect,actor}, and a cancel a message id
- **AND** control effect is pause, resume, purge or delete

#### Scenario: Send effect

- **WHEN** a report contains a send
- **THEN** id, to, type and payload are required, and no id is generated
- **AND** optional deliverAt/delay, correlationId, headers, retry and retention follow existing send wire formats;
  headers defaults to an empty map and cannot be null, and other absent/null optional fields mean their defaults
- **AND** two non-null delivery fields are refused, neither means Now, and the delivery kind otherwise is preserved
- **AND** from, causedBy, notification, input retry counts and event ids cannot be supplied as effect fields

#### Scenario: Failure and cancellation reports

- **WHEN** report is {kind:"failed",cause,retryable,reason} or {kind:"cancelled"}
- **THEN** it carries no state or effects, and core validates the permitted cause/retryable combinations
- **AND** HTTP does not arbitrate handler end or operator cancellation on the worker's behalf

#### Scenario: Completed response

- **WHEN** core confirms application
- **THEN** the server returns 200 with {result:"completed",outcome,cause,version,sent,cancelled}
- **AND** outcome is {kind:"done"}, {kind:"cancelled"}, {kind:"failed",error}, or
  {kind:"rescheduled",error,deliverAt?|delay?}, preserving the core's delivery and actual result
- **AND** cause is present, non-null only for failed, and sent uses complete execution message views
- **AND** deterministic core validation may produce a failed completed outcome for a returned report

#### Scenario: Rejected response and permanent transition

- **WHEN** core returns a known refusal with a nonretryable failure report
- **THEN** HTTP returns 200 {result:"rejected",failure:{cause,retryable,reason}}, with no fallback commit
- **AND** ActorClient returns Rejected unchanged so Worker permanently switches to that failed report
- **AND** losing the following failed-report acknowledgement cannot restore original effects

#### Scenario: Lost lease or acknowledgement

- **WHEN** core returns LeaseLost
- **THEN** HTTP returns 200 {result:"leaseLost"}, without implying whether a prior completion succeeded
- **WHEN** a completion reply is lost or its HTTP future is dropped
- **THEN** the commit may have applied and the worker may retry only its current report under its existing budget
- **AND** no HTTP error or message GET is treated as permission to rerun the handler or release after handler end

### Requirement: Lease requests have unambiguous bounded JSON

The four routes SHALL enforce the configured request byte limit, default 1 MiB, and serde_json's default recursion
limit. Bodies SHALL be nonempty JSON objects with no trailing value. Unknown envelope fields SHALL be refused at
every level; arbitrary state/payload keys SHALL remain data. Duplicate decoded keys SHALL be refused at every depth
before invoking a core operation. The old routes SHALL retain their existing parsing behavior.

#### Scenario: Duplicate keys

- **WHEN** a request repeats a types key, report field, header key, or any key inside state/payload, including an
  escape-equivalent spelling of a key
- **THEN** it returns 400 bad-request and performs no lease/renew/release/completion operation

#### Scenario: Required fields and variants

- **WHEN** a required envelope field is missing, null where not nullable, of the wrong type, or an unknown field or
  enum variant is supplied
- **THEN** the route returns 400 bad-request without invoking core
- **AND** empty optional effect arrays and optional send fields are the documented exceptions to required fields

#### Scenario: Exact numbers and addresses

- **WHEN** epoch, version or seq is i64::MIN or i64::MAX in its supported request/response position
- **THEN** the wire round trip retains every bit as a JSON integer
- **WHEN** an integer field is a float, exponent number, or outside its type's range
- **THEN** decoding fails rather than rounding
- **WHEN** a worker echoes an actor such as counter/a%20b%2Fc in a request body
- **THEN** it addresses the same actor through the ordinary address parser without path re-encoding

#### Scenario: Duration precision

- **WHEN** a grant or delivery duration is PT0.000000002S
- **THEN** it decodes as two nanoseconds, with existing ISO formatting/parsing
- **AND** sub-nanosecond fractions truncate as in the existing codec, out-of-Duration-range values fail, and
  representable durations still undergo Worker's checked deadline validation

#### Scenario: Body and value depth limits

- **WHEN** a request exceeds max_request_body_bytes, with or without Content-Length
- **THEN** it returns 413 request-too-large before a core operation
- **WHEN** JSON exceeds the parser's nesting budget
- **THEN** it returns 400 bad-request
- **WHEN** a report value parses but exceeds core's storage depth limit
- **THEN** it reaches normal core completion validation, including a derived failed Completed where specified
- **AND** HTTP adds no JSON Schema check that depends on which handlers the server hosts

#### Scenario: Oversized completion

- **WHEN** a finished report is refused with 413
- **THEN** ActorClient returns Invalid and Worker abandons without release or a hidden replacement failure report
- **AND** later lost-claim recovery may run the handler again, limited only by the configured claim limit if enabled

### Requirement: ActorClient implements the core TurnSource contract

ActorClient SHALL implement TurnSource using the four routes and existing type routes, without another remote
ActorSystem facade. The adapter SHALL add no application-level mutation retries. A positive turn_timeout, default
10 seconds, SHALL bound sending and reading each lease/renew/release/complete response. It SHALL not override a
shorter timeout of a supplied HTTP client; existing management methods SHALL retain their behavior.

#### Scenario: Status classification

- **WHEN** an exact expected status and valid result body are returned, or release returns 204
- **THEN** the adapter returns the corresponding core result
- **WHEN** HTTP returns 400, 413 or 422 outside registration's recognized alternative
- **THEN** it returns SourceError::Invalid
- **WHEN** HTTP returns 408, 429, 502, 503 or 504, or transport/body reading/timeout fails
- **THEN** it returns SourceError::Unavailable
- **WHEN** any other unexpected status arrives, including 3xx, 401, 403, 404, 405, 409 or 500
- **THEN** it returns SourceError::Fatal, not LeaseLost, Rejected or ContractMismatch
- **AND** classification uses the actual HTTP status even when the problem body claims a different status

#### Scenario: Strict execution response and tolerant envelope

- **WHEN** a success body has an extra response envelope field
- **THEN** the client ignores that field
- **WHEN** it lacks any required field, including a nullable message field, has an unknown variant or an invalid
  retry/retention policy, or cannot convert to the core result
- **THEN** the client returns Fatal rather than inventing a default or treating policy failure as null
- **AND** known policies keep their existing strict validation, and old management MessageView behavior is unchanged

#### Scenario: Timeout and request count

- **WHEN** a turn request or response body stays pending beyond turn_timeout
- **THEN** the client returns Unavailable and drops the request future, whose server outcome may be uncertain
- **AND** it sends no second application request, irrespective of send_attempts
- **WHEN** Worker cancels renew/complete sooner under its own deadline
- **THEN** dropping the adapter future does not initiate another request
- **WHEN** turn_timeout is set to zero
- **THEN** the builder method rejects it consistently with send_attempts rejecting zero

#### Scenario: Registration and type lookup

- **WHEN** registration PUT returns 200
- **THEN** the adapter returns the stored Registered type
- **WHEN** it returns 422 invalid-contract
- **THEN** it returns InvalidContract with the detail
- **WHEN** it returns 409 contract-conflict
- **THEN** it performs one GET of that type for diagnostic existing data and recomputes requested_hash from name/schemas
- **AND** an absent type on that follow-up is Unavailable; no replacement PUT or atomic conflict snapshot is promised
- **WHEN** GET /types succeeds
- **THEN** TypeDef reconstruction retains all served schemas/descriptions and served hashes

#### Scenario: Supplied HTTP client policy

- **WHEN** a host supplies a reqwest client
- **THEN** its authentication, TLS, redirects and retry settings still apply
- **AND** documentation and Worker examples disable redirects and custom mutation retries, since a redirect may
  otherwise resend POST even though the adapter itself adds no retries
- **AND** no guarantee of one wire request is made for arbitrary host client policies

### Requirement: OpenAPI and examples describe interoperable remote workers

OpenAPI SHALL include the four full-router operations with operationIds leaseTurns, renewTurn, releaseTurn and
completeTurn, their request/response schemas and default problem responses. Guides SHALL describe strict execution
envelopes, exact i64 parsing, trusted host authentication, report-size/depth limits, timeouts and uncertain completion.

#### Scenario: Published contract and library example

- **WHEN** OpenAPI is generated
- **THEN** docs/openapi.json matches the generated schemas and the read document contains none of the four operations
- **AND** a compiled library example runs an ordinary Rust actor through Worker/ActorClient without database access
  in the worker, using an explicitly configured HTTP client

#### Scenario: Two-server and fault verification

- **WHEN** integration tests lease through one server and renew/complete through another sharing its store
- **THEN** stopping the first server does not require sticky sessions or rerunning the handler
- **AND** tests cover reply loss after applied effects, explicit Rejected transition and operator cancellation
- **AND** runtime timing/race assertions use virtual time with controlled I/O barriers
