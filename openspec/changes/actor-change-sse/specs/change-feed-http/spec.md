## ADDED Requirements

### Requirement: Both HTTP routers stream actor change hints

GET /changes SHALL subscribe to ActorSystem::changes and return 200 text/event-stream on both full and read-only
routers, under the host's existing layers. Query parsing SHALL reuse Params: first decoded prefix wins, absent/empty
means all, unknown keys are ignored and no new prefix validation is added. A parser refusal SHALL use the normal 400
problem response before streaming. No health/store preflight SHALL be performed.

#### Scenario: Both routers and host layers

- **WHEN** GET /changes is requested through actor_api or actor_api_read
- **THEN** the successful response is 200 with text/event-stream, Cache-Control: no-cache and X-Accel-Buffering: no
- **AND** the host's wrapping layers remain effective and the read router still offers no write operation

#### Scenario: Literal first prefix

- **WHEN** prefix is absent, empty, order, order/, user%2Fjan%2540, or repeated
- **THEN** matching uses the first once-decoded value and the core's literal canonical-address prefix rules
- **AND** unknown query keys, including cursor and limit, add no filtering or paging semantics

#### Scenario: A query the existing parser refuses

- **WHEN** Params rejects the query before creating the subscription
- **THEN** the route uses the existing 400 application/problem+json response rather than starting an SSE body

### Requirement: The SSE wire format maps core hints without replay

Changed SHALL map to event changed with JSON data {"address":"<canonical>"}; Resync SHALL map to event resync with
JSON data {}. Each UTF-8 SSE event SHALL end with a blank line. No id or retry field SHALL be emitted. Last-Event-ID
SHALL be ignored, and each reconnection SHALL create a new subscription. No state, version, operation kind, history
or stronger delivery guarantee SHALL be added by the transport.

#### Scenario: First application event

- **WHEN** a subscription's core source is live
- **THEN** the first application event is resync with nonempty data {} so EventSource dispatches it
- **AND** comments may precede it, but no id or retry field occurs

#### Scenario: A changed address

- **WHEN** the core yields Changed for order/42 or user/jan%40example.com
- **THEN** an event named changed carries JSON with that exact canonical address
- **AND** a subsequent actor GET returns committed state or absence, possibly reflecting a later change

#### Scenario: Reconnect with Last-Event-ID

- **WHEN** a request carries any Last-Event-ID value
- **THEN** it is ignored and the new subscription follows the initial resync protocol, with no history replay

#### Scenario: A future non-exhaustive core variant

- **WHEN** the transport's wildcard handles a future core variant without a specific wire mapping
- **THEN** it conservatively emits resync rather than dropping the hint or inventing another event name

#### Scenario: Proxy buffering hint

- **WHEN** the stream is served through a proxy
- **THEN** the response includes X-Accel-Buffering: no
- **AND** documentation still requires the host to verify buffering/compression/timeouts; the header is not a universal
  guarantee that intermediaries flush events immediately

### Requirement: Keep-alives preserve readiness and bounded delivery

The response SHALL add fixed 15-second keep-alive comments with text keep-alive while the core stream is pending,
resetting the interval on output. Comments SHALL NOT imply readiness. Each response SHALL own one core subscription
without an unbounded additional event queue. Slow body consumption SHALL retain the core's lag/resync recovery.
Dropping the body SHALL release that subscription and its keep-alive work.

#### Scenario: Source is not yet ready

- **WHEN** PostgreSQL is Down and the first application event is still pending
- **THEN** headers return as 200 without a health query and pending output produces keep-alive comments at 15 seconds
- **AND** resync waits for source readiness; indefinite unavailability can keep the stream pending indefinitely

#### Scenario: Idle after an event

- **WHEN** no application event follows the last output for 15 seconds
- **THEN** a keep-alive comment is sent without creating a changed/resync event
- **AND** an ended inner stream ends the response instead of keeping it alive with comments

#### Scenario: Slow body reader

- **WHEN** the body is not consumed while enough source items accumulate to overrun the core queue
- **THEN** later body consumption receives the core's resync recovery without an unbounded HTTP-side queue

#### Scenario: Client disconnect

- **WHEN** a response body is dropped during idle or active streaming
- **THEN** its subscription is released while other streams and ordinary API requests remain usable

### Requirement: Streaming lifetimes follow the store and host

Store::close SHALL end open SSE bodies without a terminal event; a request after close SHALL return an empty ended
200 SSE body. Worker stop alone SHALL NOT end streams. cicada-app SHALL retain its existing node stop, bounded HTTP
drain, forced cutoff and store-close sequence. Standalone hosts SHALL own their cancellation/drain and connection
limits. No new per-node subscriber quota or worker-state dependency SHALL be added.

#### Scenario: Store close

- **WHEN** the store closes during streaming or before the request
- **THEN** the response ends cleanly with no error frame; after-close requests receive 200 with no application events
- **AND** nonstreaming store-dependent reads retain their 503 behaviour

#### Scenario: Worker stop

- **WHEN** a system stops while its Store remains open and another system sharing that Store writes an actor
- **THEN** its existing stream can still receive the resulting hint

#### Scenario: App shutdown with an open stream

- **WHEN** cicada serve stops with a real /changes response still open
- **THEN** the node stops first, the response receives the existing HTTP drain (5 seconds in production), then its
  unfinished body is cut with a transport error and the store is closed
- **AND** the open stream does not add an unbounded wait; node shutdown/grace time is additional to HTTP drain

#### Scenario: Embedded axum host

- **WHEN** a host mounts the routers without cicada-app
- **THEN** it supplies its own shutdown policy for long-lived requests
- **AND** documentation warns that an unbounded graceful drain may wait for open SSE responses
- **AND** the runnable HTTP-guide example stops its node and closes its Store inside the shutdown future, before
  waiting for axum's drain to finish, so its own change streams terminate

### Requirement: OpenAPI describes the event stream representation

Both router documents SHALL include GET /changes with operationId streamActorChanges, only the prefix query, and a
200 response whose only media type is text/event-stream with a string schema and frame examples. Its description
SHALL explain the two JSON event shapes, comments, initial readiness, no replay and termination. The default error
response SHALL remain problem+json for pre-stream failures. The committed OpenAPI SHALL be regenerated and checked.

#### Scenario: Generated documents

- **WHEN** full and read-only OpenAPI documents are generated
- **THEN** both describe the SSE representation and its prefix query without a JSON-page response or limit/cursor
- **AND** docs/openapi.json matches the full document and every operation retains its problem error description

### Requirement: Guide consumers reconcile with bounded pending work

The HTTP guide SHALL show EventSource plus one serialized refresh worker, a bounded dirty-address set and resync
recovery using the state listing. It SHALL preserve canonical address encoding and retain/recover failed reads.
It SHALL state that the JS example is illustrative, while protocol behaviour is covered by Rust tests. No
cicada-client SSE method or public event DTO SHALL be required by this change.

#### Scenario: Hints during paging

- **WHEN** changed events arrive while a full resync listing is in progress
- **THEN** the example queues/coalesces their addresses and rereads them after publishing a complete current result
- **AND** a newer resync invalidates an older incomplete listing rather than installing it as current

#### Scenario: Overflow or fetch failure

- **WHEN** the dirty set would exceed 1000 addresses, or a listing/address fetch fails
- **THEN** the example schedules a resync or retains the work with delayed retry and one worker/timer
- **AND** it does not discard a failed address, publish a partial listing or create an unbounded task queue

#### Scenario: Encoding and disposal

- **WHEN** the example receives an address with encoded slash, percent or @ characters
- **THEN** it uses the canonical address unchanged in its Map and actor URL suffix, and encodes prefix/cursor query
  values exactly once with URLSearchParams
- **AND** disposal closes EventSource and cancels pending retry/fetch work

#### Scenario: Browser reconnect and authentication

- **WHEN** EventSource reports an error while CONNECTING, or has become CLOSED
- **THEN** the example lets browser reconnection proceed in the former case and displays the stopped connection in
  the latter, without inferring an HTTP status or promising automatic reconnect after fatal failure
- **AND** the guide explains native EventSource's lack of custom Authorization headers, session/cookie credentials,
  host CORS, streaming-aware proxies/timeouts and connection limits
