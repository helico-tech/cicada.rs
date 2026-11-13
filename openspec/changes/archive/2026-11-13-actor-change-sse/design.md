# Design

## Scope

One HTTP transport for ADR 0034's feed. The owner's Opus 5.5 adviser settled four implementation tasks plus checks.
ADR 0035 records the SSE wire convention. Use the landed ActorSystem::changes/ActorChange and actor-state listing;
reconcile names and fixtures with both predecessors before building. No new dependency, client API or feed semantics.

## Route and frames

GET /changes?prefix= is in the common read route chain, on actor_api and actor_api_read, operationId streamActorChanges.
Reuse Params: decode the query once, take the first prefix (missing/empty means all), ignore unknown keys, and use
literal canonical-address starts_with with no new prefix validator. Parser errors use the normal 400 problem response
before opening the stream. No limit/cursor query semantics. The host's existing authentication/CORS layers still apply.
Map Changed { address } to event `changed`, data {"address":"<canonical>"}; map Resync to event `resync`, data {}.
Use axum Sse/Event and JSON serialization of a private address-only shape; no new public cicada::api row. Events are
UTF-8 SSE frames ending in a blank line. The resync JSON must be nonempty data so EventSource dispatches it. Emit no
id or retry fields; ignore Last-Event-ID, and every reconnection creates a new core subscription with initial resync.
Core currently has two variants; the non-exhaustive wildcard conservatively maps to resync, never a third wire event.
Not chosen: state/version payloads (duplicate reads and revive ordering promises), synthetic ids (imply replay),
stateful resume tokens, or /actors/changes (the proposal already specifies /changes).

## HTTP response and resource lifetime

Return 200 immediately after subscribing, without a health or store query. Sse supplies text/event-stream and
Cache-Control: no-cache; add X-Accel-Buffering: no as a proxy hint, not a promise that every intermediary obeys it.
KeepAlive::new().interval(15s).text("keep-alive") emits comments while the inner stream is pending; each event/comment
resets the interval. Comments may precede the first resync while PostgreSQL is Down; they do not signal readiness.
The first application event waits for the core's Up boundary and can wait forever during an outage. Hosts may enforce
an opening/readiness deadline. After 200 there is no problem or error event: transport faults end the body normally
or with its transport error. No Accept negotiation is added; this route's successful representation is always SSE.

One core subscription per response, no extra server queue or task per changed actor. Body backpressure eventually
causes core lag/resync. Dropping the body frees the receiver. KeepAlive stops when the inner stream ends. A closed
store yields 200 with an immediately ended body; Store::close ends an existing stream cleanly without a terminal
frame. This is the explicit streaming exception to nonstreaming reads returning 503 after close. System::stop alone
leaves feeds open. A host limits concurrent connections at its own middleware/proxy; no new quota or setting.
Not chosen: a health preflight (racy and stops unstarted read-only hosts), a subscriber cap, or configurable keep-alives.

## Shutdown

Keep cicada-app's current lifecycle: stop node, drain HTTP for DRAIN (5s), CutBody aborts remaining open responses,
then close Store. An open dashboard may use the whole drain after the node has stopped; node grace/cancellation time
is additional. This bounded policy prevents SSE from indefinitely delaying shutdown. Verify using the actual /changes
route and existing stop/drain harness, not just a synthetic stream. Independent axum hosts own their drain/cancellation
policy; plain graceful shutdown can wait for open SSE bodies unless the host closes/cancels them. No lifecycle change.
Update the runnable HTTP-guide mount: its shutdown future awaits node.stop(), then closes its Store clone before
axum finishes draining. This ends core feeds and prevents the guide example deadlock; leave cicada-app's drain intact.
Not chosen: ending streams on worker stop (no meaning for an unstarted read-only host, and core deliberately keeps them).

## OpenAPI

Installed aide has no SSE OperationOutput. Add a private EventStream marker following ProblemJson's manual response
pattern: 200 content has only text/event-stream, schema string, an example of both frames and a description of event
shapes, comments, readiness, no replay and close. Use a prefix-only query description, not ListQuery's limit/cursor.
Retain described(...).default_response::<ProblemJson>() for pre-stream errors and host layers. Regenerate OpenAPI and
assert both router documents expose the route. OpenAPI 3.1 has no event item schema; do not model the body as JSON.
No cicada-client SSE method: browsers use EventSource, in-process Rust uses changes, remote Rust clients bring an SSE
reader. Adding streaming client dependencies or a public event DTO is beyond this behaviour.

## Browser guidance

Show one guide-only JS EventSource and one serialized refresh worker, not an async fetch for every event. Wait for the
first resync before the initial full read. Keep a dirty-address Set capped at 1000 and a resync/generation flag: overflow
or resync clears dirty and requests a fresh paged read; a resync during that read invalidates its partial result. Gather
pages into a fresh Map, publish only a successful complete current-generation result, then reread dirty addresses.
Keep hints arriving during a read for later; a failed list/get retains dirty work or requests resync with delayed retry,
never silently loses the address. At most one worker/retry timer runs. Handle 404 as deletion, other errors as failure.
An error event with CONNECTING leaves browser reconnection to EventSource; CLOSED shows failure instead of assuming
recovery. Do not infer a particular HTTP status from CLOSED. A user/host may reconnect after fixing auth with backoff.
Dispose the source and pending timer/fetch work when the page goes away. Unknown event names are ignored.

Use each JSON address unchanged as the Map key and suffix of /actors/: it is already canonical/encoded. Encode query
prefix/cursor once with URLSearchParams; a literal %40 in the prefix becomes %2540 on the wire. Do not encode the whole
address again. Include slash/percent/@ examples. Native EventSource supplies no custom Authorization header option;
show same-origin session/cookie auth, and explain credentials/CORS for other origins. Hosts must allow streaming through
proxies, compression and timeouts; the header is only a hint. Prefer one stream per page; HTTP/1 connection limits may
matter. Avoid blanket claims that a particular middleware timeout always times the body rather than just headers.
Repeated resyncs reread many rows; no new snapshot/history guarantee, SQLite coverage or durable reaction is added.

## Verification and docs

Virtual-time body-stream tests cover events, comments, initial pending source, both routers, query/encoding, ignored
Last-Event-ID, lag/backpressure, dropped bodies and close/stop. Test the framing adapter with a pending source for
pre-Up keep-alive coverage without reaching private backend APIs from axum. A protocol integration test mirrors the
guide's serialized resync/list/get flow, including events during paging and retry after a failed read. The JS block is
not an executable Rust doctest; say so, and ground protocol behaviour in those tests. Add an actual app shutdown test.
Update axum crate route table, HTTP/core mounting guidance, architecture/closed-store exception and operations, plus
changelog/version. No actor contracts or SQL changes. Rust guide blocks remain compiled doctests.

Review 1: ready for tasks, no blockers. Made the standalone mounting-example fix explicit: close the store after
node stop inside graceful shutdown, so an SSE response cannot keep that example waiting for its own later close.
