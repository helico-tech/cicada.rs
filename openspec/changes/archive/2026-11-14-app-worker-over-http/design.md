# Design

## Mode and settings

worker gains HTTP mode using the existing --server, --context and --token globals, accepted before or after worker.
Keep serve's refusal of all operator flags and worker's refusal of -o/--output. A binary without hosted types still
has neither serve nor worker, and worker --types none is still a usage error. node_command recognizes HTTP workers.

Select mode before resolving its settings: --store with --server/--context is exit 2; --store alone selects store;
--server or --context selects HTTP. Without mode flags, nonblank CICADA_STORE selects store even alongside operator
variables; otherwise nonblank CICADA_SERVER/CICADA_CONTEXT selects HTTP. No selector is exit 2 with a store/server
hint. current-context and token alone never select HTTP. --token in store mode is exit 2; ambient credentials are
ignored there. Preserve direct-store behavior; no new --remote flag or forced cleanup of operator environment.

Resolve store mode with the existing resolver. Add a private HTTP settings record holding Target, WorkerConfig and
selected types; do not run the store resolver first or validate an unused NodeConfig. Mode selection treats blank
variables as unset; for HTTP target resolution remove whitespace-only mode variables, then reuse config::resolve's
flag > env > selected/current-context rules. Preserve nonblank values and existing credential/header precedence.
The config file is read only for HTTP mode; its errors are exit 1 as for operator commands. current-context may supply
credentials even when --server overrides its URL; retain and document that behavior, without changing operators.

Map node_id to worker_id, and concurrency/poll_interval/default_max_turn/shutdown_grace_period to the matching Worker
fields: flag > env > host NodeConfig > library default. Copy host shutdown_cancel_timeout when given, otherwise use
Worker's default; it gains no flag. Type selection is unchanged. Worker validates its own limits; failures to parse
an applicable setting are usage errors, initialization/validation failures remain exit 1 as with a node.

In HTTP mode refuse explicit --store-schema, --store-pool-size, --retention, --lease-ttl, --heartbeat-interval and
--max-lost-claims with exit 2 and the offending flag (server-owned). Their environment variables and corresponding
host fields, including sweep_interval, are not read/validated. Ignore CICADA_LISTEN too: no HTTP listener is opened.
An explicit HTTP selector overrides even an invalid CICADA_STORE. No new turn-timeout setting: use ActorClient's 10s.

## Composition and client

Use a private Hosting::{System(ActorSystemBuilder), Worker(WorkerBuilder)} for the App::host closure, so ActorType<A>
moves once into the chosen builder. Selected actors and all Arc observers reach either builder, without changing the
core actor API. App::http remains serve-only. Store opening/check_built_in is reachable only from store mode.

An App with any on_start hook refuses HTTP mode as a usage error before reading config or sending requests: hooks
require ActorSystem and cannot be silently skipped or run against a fabricated system. Bootstrap the server separately
or move such setup out of the worker App. No new hook/remote-management facade; store-worker hooks remain unchanged.

Add an internal Target worker-client constructor sharing the existing header/token construction, with sensitive bearer
headers, redirects disabled and no added retry policy. Keep the existing operator client unchanged. Its total reqwest
timeout is 30s; the predecessor's 10s turn timeout further bounds lease/renew/release/complete including body reads.
Registration/listing retain the 30s bound. Host layers need access to GET /types, GET /types/{name} (conflict
diagnostics), PUT /types/{name} and /leases routes.

Give Target/settings a redacted Debug representation: omit token and header values and sanitize URL passwords. New
App logging must not print raw settings, tokens or request headers. Preserve existing error classifications and URL
redaction; this does not claim to sanitize arbitrary user/server-produced text or change all library logging.

## Start, run and stop

Watch the supplied stop from command entry. After usage/config/client checks, probe TurnSource::list_types, retrying
only Unavailable after 1s and starting no probe at/after 60s from the first attempt. Check before each attempt as well
as after sleeping. The last 30s-bounded probe may finish after that horizon; document approximately 90s worst case,
not a strict 60s total timeout. This bounds readiness preflight only: sequential registration and a possible conflict
GET follow it, with no total-startup bound. Fatal/Invalid fail at once with exit 1. A biased stop branch interrupts wait/request
and returns success. This is readiness preflight, not a promise that the following registration cannot fail.

Build the Worker with selected types, mapped config and observers; race start with stop. If stop wins, drop start,
call worker.stop and return success unless Worker recorded an automatic error. Late registration may have persisted
contracts, as documented by Worker. If start fails, stop for cleanup and return exit 1 with start diagnostics. Do not
retry StartError by inspecting strings; a registration failure after successful preflight fails this invocation.

After start, race stop.wait with worker.stopped. A signal calls worker.stop (grace then cancel timeout); inspect
stopped afterward so an already-recorded automatic error still yields exit 1 instead of being hidden by the signal.
None means normal exit 0, Some(error) means exit 1. Preserve Worker cancellation/handler-end/uncertainty rules: never
release a finished handler, and noncooperative work may outlive stop until process exit/lease expiry. App never calls
source stop or close and opens no listener; second signals retain current behavior. No readiness endpoint/node row.

## Verification and docs

Settings/parser tests cover all precedence rows, flag positions, blanks, explicit conflicts, poisoned ignored env,
applicable validation and broken config files. Test the preflight helper with virtual time/fake sources for late
readiness, request crossing the horizon, no post-horizon attempt, permanent failure and stop while a request hangs.
Integration uses an unchanged actor through App/ActorClient and a handler-free server, observing completion through
channels. Prove no database access with invalid PostgreSQL and unused SQLite paths, and compile App without store
features. Runtime/stop assertions use paused time and controlled I/O barriers; socket framing needs no timing claims.

Cover auth/context headers, 307 without following/resending, conflict on registration, stop during registration/work,
automatic-error/signal races, selected types and observer delivery, and refused hooks with zero HTTP requests. Run
existing store-worker/serve tests. ADR 0042 amends ADR 0027 for worker flags only. Update app guide, architecture,
operations, crate docs/README, compiled example and CLI reference. Four implementation tasks plus finishing suffice;
use the version chosen when built, not a speculative version from this design. No HTTP wire/core runtime change.
