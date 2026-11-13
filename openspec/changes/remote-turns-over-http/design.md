# Design

## Routes and trust

Add POST /leases, /leases/renew, /leases/release and /leases/complete to the full router only; operationIds leaseTurns,
renewTurn, releaseTurn and completeTurn. JSON bodies carry actor/epoch, avoiding path re-encoding and collision with
GET /turns. Authentication remains the host's, covering /leases itself as well as its children and registration.
These are trusted writes: epoch is a fence, not a secret; no credentials, quotas, presence or sticky session is added.

Every core alternative is a 200 JSON result tagged by `result`; release is 204 with no body. In particular leaseLost,
contractMismatch and rejected are not HTTP problems: a proxy's 404/409 must not masquerade as a core decision.
OpError maps as today: Invalid/Store Invalid 400, unavailable 503, corrupt/config/start/panic 500; oversized body 413.
No server timeout or health precheck. Core lease/renew/release/complete lifecycle, cleanup and fencing rules apply.
Disconnect after lease handoff may lose a batch until expiry; a dropped completion may still commit. Completion adds
no server observer, and HTTP adds no observation; core lease admission still observes confirmed claim-limit failures.

## Wire model

Add documented serde/JsonSchema views in cicada::api (a leases module re-exported there): LeaseTurnsRequest/Response,
TurnLeaseView, TurnMessageView, RenewTurnRequest/Response, ReleaseTurnRequest, CompleteTurnRequest/Response,
TurnReportView and its state/effect/failure/outcome records. Core runtime types stay transport-independent. Reconcile
names with the landed predecessors; conversions preserve every field, effect order, delivery kind and null value.

Lease request: {owner, types: {name: hash}, limit}. Renew/release: {actor, epoch}. Complete:
{actor, epoch, messageId, report}. Returned report: {kind:"returned", state, effects}; failed: {kind:"failed", cause,
retryable, reason}; cancelled: {kind:"cancelled"}. State is {kind:"unchanged"|"stop"} or {kind:"set",value}.
Effects has optional empty arrays sends/creates/cancels/watches/controls; effects itself is required. Each send requires
id/to/type/payload and optionally has deliverAt or delay, correlationId, headers, retry, retention, as SendRequest.
Absent/null optional send fields mean defaults; headers defaults to {} and cannot be null. Never generate a send id.
Creates are {address,state,child}; watches {target,set}; controls {effect:"pause"|"resume"|"purge"|"delete",actor}.
Unknown envelope fields are refused, including nested report/policy fields; arbitrary state/payload keys stay data.

Leased result: {result:"leased",leases:[{actor,epoch,message,state,version,claimedAt,leaseUntil,ttl}]}.
Mismatch: {result:"contractMismatch",type,registered:null|string,requested}. Renewal:
{result:"renewed",leaseUntil,ttl,cancelRequested} or {result:"leaseLost"}. Completion:
{result:"completed",outcome,cause,version,sent,cancelled}, {result:"rejected",failure:{cause,retryable,reason}}, or
leaseLost. Outcome: {kind:"done"|"cancelled"}, {kind:"failed",error}, or {kind:"rescheduled",error,deliverAt?|delay?}.
No delivery fields means Now; both non-null fields is invalid. cause is non-null only for a failed completed outcome.

TurnMessageView uses all MessageView field names: id/to/from/type/payload/deliverAt/correlationId/headers/seq/attempt/
failures/lostClaims/lastError/retry/causedBy/retention/notification. Every field is required, even nullable ones; no
default counters and no lenient policy decoding. Other response record fields are likewise required except optional
outcome delivery fields. Ignore unknown response envelope fields; known retry/retention retain their strict validators.
Unknown variants or invalid policies fail decoding. Do not change the existing management MessageView.

Use existing instant/ISO duration codecs, including nanoseconds (PT0.000000002S) and truncation below a nanosecond.
Durations beyond Duration's range fail; representable ones may still fail Worker's local deadline overflow checks.
epoch/version/seq are exact JSON i64 integers; counters use their existing unsigned widths. No float/exponent integer
coercion. Non-Rust clients must preserve all 64 bits, not round through a JavaScript Number. Addresses use the existing
parser and serialize canonically; echo `counter/a%20b%2Fc` in JSON without another percent-encoding pass.

## Parsing and completion boundaries

Keep the existing request byte limit (default 1 MiB). Add a duplicate-rejecting recursive JSON value reader for only
these four routes; retain serde_json's recursion limit and reject trailing JSON, duplicate decoded keys at any depth,
non-object/empty bodies and mistyped/missing fields with 400 before any operation. Reuse bounded byte reading without
changing old routes. No schema validation of report state/payload: core storage checks may derive a failed Completed
outcome. A syntactically valid over-depth value within the parser budget reaches those checks; beyond it is 400.

The complete handler calls core once and serializes its actual result. Rejected is not a commit: Worker switches
permanently to the supplied failed report. No retry, fallback, release, caller event id or server-derived sender field
is accepted/added by HTTP beyond core policy. 400/413 does not authorize a hidden failure report. Worker abandons
without release; another claim may rerun later, bounded only by the server's configured claim limit, if any. Hosts
must size body/proxy limits for whole reports, not individual sends. No separate completion limit in this version.

## ActorClient source

Implement TurnSource for ActorClient; no inherent lease facade. Use the common raw request helper, not send_message's
retry loop. Add the existing async-trait workspace dependency. One application request per lease/renew/release/complete.
turn_timeout(Duration) defaults to 10s and rejects zero like send_attempts rejects zero. A tokio timeout bounds send
plus body reading for these four calls; it does not override a shorter supplied reqwest/connect/read timeout. Existing
management calls retain their settings. Worker may cancel renew/complete earlier under its own guard/budget.

Classify by actual HTTP status, never a problem body's status: 400/413/422 -> Invalid; 408/429/502/503/504 and network/
body-read/timeout errors -> Unavailable; all other unexpected statuses (including 3xx/401/403/404/405/409/500) -> Fatal.
Encoding/URL/builder errors are Invalid; expected-status malformed bodies or invalid conversions are Fatal. Only exact
200 results or 204 release succeed. Treat unknown outcomes/causes as Fatal, never fabricate success or LeaseLost.

Registration/listing reuse PUT /types/{name}, GET /types and GET /types/{name}. Registration 200 becomes Registered;
only a 422 invalid-contract becomes InvalidContract; only a 409 contract-conflict triggers one GET for existing.
Recompute requested_hash from name/schemas, not Contract::of's cached registry hash. A vanished conflict is Unavailable;
no replacement PUT. Preserve the follow-up's observed registration as diagnostics, not an atomic conflict snapshot.
Reconstruct TypeDef JSON from TypeView fields and retain the served hash/descriptions. Other failures use the table.

Supplied reqwest settings still apply. Worker examples require redirects disabled and no custom retry policy; the
adapter cannot inspect/change an already built client's policy. Default reqwest may follow redirects, including
resending POST on 307/308, so document that limitation instead of promising one wire request for arbitrary clients.
Transport retries known not processed are permitted; no retry after possible server processing is added here.

## Verification and scope

Five tasks: wire/conversions, routes/parser/OpenAPI, client adapter, end-to-end/fault parity, docs; then finishing.
Runtime/race tests use virtual time and controlled barriers, including router oneshot tests and socket harnesses that
control clock advancement. Socket framing smoke tests need no timer-sensitive assertions. Cover two independent
servers on a shared store, stopped first server, response loss, explicit rejection, cancellation and every status class.
ADR 0040 records the wire format. No CLI, migration, receipt store or worker credentials. Older servers fail clearly;
legacy states too deep for a response parser may stop workers and leave claims to expire. Document that recovery cost.
