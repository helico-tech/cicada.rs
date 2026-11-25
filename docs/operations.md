# Operate Cicada

Use this runbook to size a deployment and diagnose a running system.
For version changes use [the upgrade procedure](how-to/upgrading.md); for exact flags use [CLI help](reference/cli.md).

## Sizing

| Resource | Starting point | Watch for |
| --- | --- | --- |
| Turn concurrency | 16 per node | Lag for busy types; slow dependencies |
| PostgreSQL pool | At least concurrency + 4 (default 20) | Renewal delays from connection starvation; the start warning “The PostgreSQL pool has N connections, fewer than the M this node needs” |
| PostgreSQL connections | Sum node pools plus up to 2 listener/probe connections per node | Server `max_connections` |
| Lease TTL | 30 s, heartbeat every 10 s | Recovery speed versus tolerance of pauses |
| Maximum turn | 15 min, overridable by type | Timeouts and blocking handlers |
| Request body | 1 MiB default | Many large JSON requests exhausting memory |

Bound HTTP concurrency as well as bytes: parsing a body into JSON can take roughly 90 times its byte size for a list
of tiny values. Remote completion reports contain **all** effects and state, so size their limit for the whole turn.
Use `spawn_blocking` for blocking work; blocked tokio threads also stall heartbeats.

SQLite has one writer and belongs on local disk, on one host. Use PostgreSQL for multiple hosts.
[Store configuration](reference/stores.md) covers timeouts, WAL and TLS.

## Deploying

1. Build a host with the store feature and actor types it needs.
2. Set the same retention and claim limit on every node sharing the store.
3. Add authentication to the HTTP router before binding beyond loopback.
4. Configure readiness, process liveness and a sufficient stop timeout.
5. Start the host and check contract registration, readiness and message lag.

```sh
myapp serve --store "$DATABASE_URL" --listen 0.0.0.0:8080 --concurrency 32
myapp worker --store "$DATABASE_URL" --types billing.invoices
# Alternative worker without database credentials:
myapp worker --server https://actors.internal --types billing.invoices
```

The first command assumes the host has installed authentication. App defaults to loopback and provides no auth.
`serve --types none` serves without running turns. A worker serves no HTTP and has no readiness endpoint.
See [App](guide/app.md) for mode selection and startup errors.

The [sample image](../sample-service/Dockerfile) contains both sample binaries. With it built and the target API
reachable, override the entrypoint to run its executor as an HTTP worker. Disable the image's health check, which
asks a server's `/ready`; this worker serves no HTTP:

```sh
docker run --entrypoint /usr/local/bin/cicada-sample-executor --no-healthcheck \
  -e CICADA_TOKEN="$TOKEN" cicada-rs-sample \
  worker --server http://actors:8080 --types e2e.counter --node-id worker-1
```

In Compose, set `entrypoint: ["/usr/local/bin/cicada-sample-executor"]`, supply `worker --server ...` in `command`,
and use `healthcheck: { disable: true }`. Pass the client token through `CICADA_TOKEN`; `API_TOKEN` protects a served
sample API. The executor creates no singletons or watches; [bootstrap the workload separately](guide/app.md#sample-service).

Pass an explicit `--server`: it overrides the image's `CICADA_STORE`, while `CICADA_SERVER` alone loses to that
store default. Supplying both `--store` and `--server` is a usage error. SIGTERM drains the worker and exits 0;
allow its configured grace period and cancellation time. Its id is trusted metadata, not proof of one physical
process. The image's `VOLUME /data` still creates an anonymous volume even without a database mount; the HTTP worker
never writes to it. Remove that volume with `docker rm -v` when removing the container.

Executors that own pinned actors need stable ids, unique among live nodes and workers: set `--node-id` or
`CICADA_NODE_ID` in App, `NodeConfig::node_id` in an embedded node, or `WorkerConfig::worker_id` in a worker. A random
id after a restart leaves existing pins waiting. Labels and host names do not select placement.

### Shutdown and probes

Serve handles SIGTERM/SIGINT in this order: stop claims, drain/cancel turns, drain HTTP (up to 5 s), close store.
Default turn grace is 30 s and cancellation timeout 5 s. Give the platform more than their sum plus HTTP drain,
for example 45 s. A forced kill uses up lost claims and delays recovery until lease expiry.

Use `/ready` for readiness: 200 when the node runs and reaches the store, 503 during outage/stop.
Keep liveness independent of the database, so an outage does not restart every node.
Embedded hosts stop the node inside axum’s shutdown future, then close the store or explicitly bound streams.
See [HTTP lifecycle](guide/http.md#run-it-in-an-axum-service).

For a host that owns child processes, close admission and stop and reap those children while the node, pump
and store still run. Deliver their last observations, then stop the node, stop and join the pump, and close the
store. The [pinned-process demo](../examples/examples/pinned_process.rs) uses `run_host` to follow that order even
when its work returns an error. Its resource cleanup has a five-second budget; it names unconfirmed exits and
undelivered observations and exits with a failure if cleanup is incomplete. Budget additional time for the node's
stop and store close. Kill-on-drop is best effort: abrupt host death may leave an orphan. The demo's inert child
exits when stdin closes; arbitrary programs need their own cleanup policy.

`Store::close` has no deadline: PostgreSQL waits for borrowed connections; SQLite waits for its writer, but not reads.
A host may time it out and call it again: the store stays closed and the next call resumes cleanup. Cancellation
leaves no background cleanup task, so call close again or drop the last store handle. The
[architecture](architecture.md#design-notes) defines the close contract and driver/runtime limits.

### Streams and proxies

For `GET /changes`, disable buffering/compression and use idle/response timeouts above 15 s.
Limit concurrent subscriptions in the host/proxy. Browser EventSource uses cookies rather than custom authorization
headers. A store close ends streams; node stop alone does not. App bounds the drain for you.

### Changing a contract

A node requires exact contract-hash equality, even for a field added with a serde default. Snapshot the contract and
check old state samples before deployment. Descriptions do not affect hashes; schemars upgrades can.
Adding a `ReplyTo<T>` field to a target actor's message changes that target's contract like any other field.

| Strategy | Procedure |
| --- | --- |
| Versioned type | Add `billing.v2.invoices`, switch new senders, let old actors finish or explicitly migrate them |
| Maintenance window | Stop all hosts/executors of the type; back up/export as needed; delete its actors, then registration; start new hosts |

`DELETE /types/{name}` refuses while actors exist. Deleting an actor also removes descendants, purges its mailbox and
notifies watchers: plan for those effects before resetting. A type gaining watch variants, or a new Failed cause,
changes watcher contracts too. [Upgrading](how-to/upgrading.md) has the version-specific procedures.

## Upgrading

Follow [Upgrade Cicada](how-to/upgrading.md) for every version crossed.
Store opening applies missing migrations; it refuses a newer schema. Only additive migrations permit already-running
old nodes to continue. Start new nodes first for an additive migration, then stop old ones; do not restart/roll back
old nodes against the migrated schema. A non-additive migration requires stopping all old nodes first.
Upgrade nodes before clients unless the release requires coordinated replacement.
Version 0.20.0's schema 8 requires draining every old direct-store node and worker before migration; old claimers
ignore pins. Follow the [0.20.0 procedure](how-to/upgrading.md#upgrade-to-0200).
Version 0.21.0 changes the contract of every container job type and of every type that takes a `ContainerEnded`;
follow the [0.21.0 procedure](how-to/upgrading.md#upgrade-to-0210).

## Releasing

Follow [Release Cicada](how-to/releasing.md). The local gate checks a pinned commit and publishes its immutable tag
and GitHub Release. Pushes and tags start no automatic Actions; Linux CI is available manually.

## On call

Start with `health`, `get nodes`, `get turns`, `get outcomes --status failed`, and `stats --prefix TYPE/`.
Stats scans rows under the prefix: avoid frequent unfiltered polling on large stores.

| Symptom | Check | Action |
| --- | --- | --- |
| Every node unready, HTTP 503 | Store connectivity/pool | Restore the store; nodes retry automatically |
| Mailbox grows | Hosted types, paused flag, pin, turn duration, concurrency | Resume or add eligible capacity; [check the pinned owner](#pinned-actors-that-wait) |
| Process run shows `NeedsReconciliation` | Old host process, restart or another live host sharing the node id | [Check and clean up the outside process](#process-runs-needing-reconciliation), then explicitly replace the run if needed |
| Every message fails as state | Stored shape versus decoder | Restore compatibility/defaults or migrate data |
| `claim limit: N lost claims` | Node crashes, pauses, blocked handlers, uncertain commits | Fix cause, then retry selected messages |
| Failed send at commit | Target exists, child-parent/control conflicts | Repair the target/configuration before retrying |
| Timer chain stopped | Cancelled/purged/exhausted timer | Scheduler: Start; debounce/batcher: Flush; flow: Continue to recover, or `FlowMsg::Cancel` to end the run |
| Heartbeat watch stays alive without an alarm | `armed` timer missing/failed, target type/instance and outcomes | Fix the cause; send `HeartbeatWatchMsg::Rearm` at the original deadline. Retry/redrive copies are ignored; Beat claims new activity. See [heartbeat recovery](guide/actors.md#heartbeat-watch) |
| No node-down alerts | Monitor exists, pending `Poll`, `lastProblem`, `lastCheckedAt`, topic/subscribers and monitor failures | Fix the cause; recreate a missing topic and subscriptions, then send `NodeDownMonitorMsg::Rearm`. See [node-down alerts](#node-down-alerts) |
| Flow run parked for long | Untimed open wait as last journal entry, `armed: null`; target mailbox/state | Have targets send missing replies through their saved routes, or delete the run; Continue only parks it again |
| Flow join has only some replies | Open wait names several requests; `replies` holds a subset | Find the missing targets; a timed join keeps partial results when its timeout decision commits. Replies to closed joins are ignored |
| Flow wait past deadline | Paused run or cancelled/purged/failed deadline Continue | Resume / send public Continue to decide timeout; a reply handled first can still win |
| Flow Cancel/cleanup message failed | Older-host refusal or known rejected commit | Reread status. Upgrade all hosts/workers; resend Cancel to a running run, Continue to resume cleanup. A refused cleanup reply needs its target to resend |
| Flow stuck in cancelling | Failed/purged/exhausted cleanup Continue, or an open cleanup wait | Fix the cause and send public Continue to replay cleanup. Another Cancel does nothing; a parked cleanup needs its reply |
| Supervisor gave up | Child entry, state, restart limit | Correct the entry/deployment; send Start |
| HTTP worker stops | `stopped()` error; auth, route versions, response format | Fix fatal source/protocol problem before restarting |

For failures beyond outcome retention, host [dead letters](guide/actors.md#dead-letters) and explicitly set a watch.
Watch their mailbox too: notifications can queue up. A notification that fails does not notify another watcher.

### Pinned actors that wait

Read the actor's `PINNED-TO` / `pinnedTo` and inspect its mailbox:

```sh
actor=reports/host-a
cicada get actor "$actor" -o wide
cicada get mailbox "$actor"
cicada get nodes
```

The chosen owner must poll that type. A node row can help find it, but a live heartbeat proves neither eligibility
nor polling; workers over HTTP have no node row. Other owners leave the work pending without new attempts, lost
claims, failures or automatic reassignment. Start the owner with its original id and the needed type.

To change a pin, first plan for the actor's state and pending work: delete and recreate is a new life, and deletion
purges its mailbox and removes descendants. A repeated create cannot move it. Pause, purge, cancel and delete can
still be issued through any node; their [destructive effects](#destructive-controls) are unchanged.

### Process runs needing reconciliation

Run exactly one [process host](../examples/src/pinned_process.rs) per stable node id. On restart, its fresh epoch
marks old nonterminal runs `NeedsReconciliation`; a second live host with the same id can mark them too. The new
host neither adopts a stored PID nor stops or restarts those runs. Check whether the old process still exists and
verify or clean it up outside Cicada. Only then delete the actor with the ordinary delete; if work is still needed,
explicitly create a replacement under a new, never-used key. No command or late observation clears the uncertain
status, and nothing replaces the run automatically.

For a run still owned by its original host, send `RunMsg::Stop` and see a terminal observed status before deleting
it. Deleting the actor does not stop its child and proves no cleanup; the live host still owns the child and keeps
its slot for the host's whole life, even after exit. [The design guide](guide/design.md#work-with-outside-systems)
explains the host-lifetime limit.

### Containers and Docker placement

Every node that can claim a [container actor](guide/containers.md#the-container-actor) must reach the same Docker
daemon. When only some nodes do, host the job's type only there, or pin its actors to them with
`CreateOptions::pinned_to`. A node that reaches another daemon sees the container missing and ends it `failed`, or
creates a second one there; the actor cannot tell.

An image pull and the stop grace run inside one turn: set the job's `ContainerJob::MAX_TURN` above both. Once a job sets
one, the node's `--max-turn` does not change that type's limit. Keep the log target
`bollard` at `info` or above, as it logs request bodies with secret environment at `debug`. A container actor that
ends `runtimeErrors` met `max_errors` runtime errors in a row: check that Docker answers on that node. Send `Remove`
before deleting a container actor; leftovers carry the spec's labels, so `sweep` with a label removes them.

### Destructive controls

| Operation | Effect |
| --- | --- |
| Pause | Stop new claims; keep running turn and pending messages |
| Purge | End pending messages except the running one as purged |
| Cancel running turn | Request cancellation at heartbeat; only wins before the handler ends |
| Delete | Remove actor and descendants; purge all their pending/running messages |
| Operator retry | Create a new message/id; may repeat external work |

For a flow, [FlowMsg::Cancel](guide/actors.md#cancelling-a-run) starts cleanup and keeps main progress. Cancelling its pending
Continue or running turn only stalls it; deleting a run runs no cleanup.

Inspect direct children before deleting a tree. Independent actors created with `create` remain.
A deleted handler may continue briefly and perform external calls, but cannot commit, even after recreation.
Large trees delete in one transaction: remove leaves first or raise the management node’s PostgreSQL transaction timeout
if the removal cannot finish. Long removals can also block watchers’ renewals.

### The claim limit

The default limit is 5 lost claims. A claim is counted lost when another claimant replaces it without a commit/release.
Hard kills, lease expiry and uncertain commits can cause it; committed handler retries and confirmed releases do not.
The count never decreases, and includes losses beside the work that actually crashed a node.

Inspect `lostClaims` / the wide listing’s `LOST` column. Keep `max_lost_claims` identical on every node;
`None` disables the protection. Fix the underlying problem before retrying: each operator retry starts a fresh message.
Use the structured `Failed.cause` when available, rather than matching an error-text prefix.

### Node table

Nodes heartbeat every 10 s. Display status is alive below 30 s, suspect below 2 min, dead afterward.
A clean stop removes its row; rows over a day old are cleaned by node heartbeats.
The table feeds displays and sampled [node-down alerts](#node-down-alerts): pins select eligible owners and leases
fence turns; node status gates neither. Started systems, including send-only nodes and App workers on the store,
have rows. Initialize-only systems and workers over HTTP do not.

`cicada get nodes` shows `HOSTNAME`; `-o wide` adds `HOSTED`, `VERSION` and `LABEL`, as does the TUI's wide nodes view.
The host name is looked up once at start; an unavailable, non-UTF-8, blank, NUL-containing or over-200-character name
is omitted without preventing startup. `VERSION` is the compiled `cicada` crate's version. Set a label such as your
application and its version with `NodeConfig::label`, `--node-label` or `CICADA_NODE_LABEL`
([settings](guide/app.md#settings)). JSON/YAML use `hostname`, `cicadaVersion` and `label`, omitted when absent;
tables show `-`. Table/TUI metadata cells escape newline, carriage return and tab as `\n`, `\r` and `\t`, and other
control characters as `\u{hex}`; JSON/YAML keep the text.

Details stay fixed while a node runs and each heartbeat replaces all three, including absent values. A dead row
keeps its last details until expiry. Container host names may change on restart. Ids remain random by default:
set `CICADA_NODE_ID` where an id should survive restarts, for example from a StatefulSet pod name, and keep it unique
among live nodes. Nodes sharing an id share one row: the last writer's details win, and either clean stop removes it.
A running 0.18 node can leave stale details during a [rollout](how-to/upgrading.md). Host names and labels are not
unique or authenticated identities; host names may identify machines, and the API's access control covers them.

### Node-down alerts

Host the optional monitor once per store, unpinned, with every host reading that same store.
[The setup](guide/actors.md#node-down-alerts) creates its topic, subscriptions and monitor, then sends `Rearm`.
Watch the monitor for terminal failures or deletion, and inspect `lastProblem` and `lastCheckedAt` separately:
caught read failures succeed as turns, so watches say nothing. A missing topic fails the poll and loses its timer;
repair the topic and subscriptions, then send `Rearm` to recover without clearing alert history.

Each due poll calls `ActorSystem::nodes()`, reading every node row and every running turn, within five seconds;
the next poll is due ten seconds after the check ends. The 1000-row cap bounds monitor state, not that read.
Busy-store reads can time out repeatedly and suppress alerts indefinitely (`lastProblem: timeout`). One invalid
row, a backwards sample time or more than 1000 rows rejects the entire sample: no alerts, unchanged history and
`lastCheckedAt`, with polling continuing. A good sample clears `lastProblem`.

A new monitor alerts for every retained dead row; a resumed monitor alerts for newly observed episodes while
keeping its watermarks. Either can produce a burst after downtime. Each new heartbeat followed by two minutes
of silence can alert again; there is no rate limit. Apply one in the receiver if needed.

Sampling creates no death history. Clean-stop removal or pruning before a read leaves no trace; a heartbeat can
race an alert, making its `lastSeen` stale on arrival. Shared node ids share one row, and HTTP workers have none.
An all-cluster outage has no observer until a store and polling monitor are back. Alerts prove neither physical
failure nor that someone was paged; never use them to move pins, claim work or fence outside effects.

### External leases

Remote turns appear under the executor’s id in running turns, outside local concurrency. The server’s TTL/claim limit
apply; stopping a server does not release them. Pins are matched against the requesting executor's id, never the
server's. Workers own their lifecycle and never close their source.

| Failure | Consequence |
| --- | --- |
| Lost completion response | It may have committed; repeat the same report within budget, never rerun the handler |
| Rejected report | Replace it permanently with the returned failure |
| 413 completion | Report too large; repeated losses can exhaust the claim limit |
| Old deeply nested data | Lease response may be unreadable and fatal; repair data before restarting workers |
| 401/403/404/500, redirect or unknown result | Fatal source error; inspect credentials, server version and protocol |
| Failed renewal | No extension confirmed; work stops at its deadline unless a later renewal succeeds |

Use an HTTP client without redirects or hidden mutation retries. Every selected server must support lease routes.
Set timeouts below the lease budget; App uses a 10 s turn timeout and a 30 s overall request timeout.
A proxy’s timeouts/body limit must accommodate those requests and whole completion reports.
[HTTP workers](guide/http.md#run-a-worker-over-http) describes classifications and required permissions.

### Logs

Install a `tracing` subscriber in embedded hosts. App does this and accepts `RUST_LOG`.

| Signal | Likely cause / response |
| --- | --- |
| Claiming failed / retention sweep failed | Store unavailable; inspect connectivity and capacity |
| Commit refused or rejected | Inspect error, target and contract/control assumptions |
| Lease lost / claim arrived after deadline | Slow store, process pause or blocked runtime |
| Handler still running, does not yield | Move blocking work to spawn_blocking |
| Runtime task panic | Runtime bug; preserve error/context |
| Schema cannot be used and is not enforced | Fix schema and add representative sample tests |
| Observer panic / omitted header | Observer bug or header limits; tracing can be incomplete |
| Completion abandoned / may have applied | Uncertain remote commit; preserve idempotency |

Committed control effects are logged with actor, effect and target. The store does not retain an audit history of
who paused/purged/deleted an actor. A Stopped notification identifies the deleting actor when deletion came from a turn.

## Store tuning

[Store configuration](reference/stores.md) lists all settings and clock behavior.
For PostgreSQL, monitor `pg_notification_queue_usage()`: a listener holding a transaction open or not reading can fill
the queue. A full queue fails COMMIT with SQLSTATE 54000 (unavailable). Polling still handles work while the listener
reconnects, but change feeds need Resync when listening resumes. Older writers/manual SQL need periodic full reads.

### TLS for PostgreSQL

Set `sslmode=verify-full` and `sslrootcert` for a private CA; do not rely on the default prefer mode for verification.
[The TLS reference](reference/stores.md#tls-for-postgresql) covers modes, environment variables and client certificates.

## Telemetry

Attach [ActorTelemetry](guide/core.md#opentelemetry) and configure the host’s OpenTelemetry SDK/exporters.

| Signal | Meaning |
| --- | --- |
| Rising `cicada.message.lag` | Too little capacity for a type or slower handlers/dependencies |
| `cicada.turn.duration`, outcome failed | Terminal failures; inspect span exception |
| Outcome rescheduled | Sender policies are absorbing failures; inspect the dependency |
| Outcome abandoned | No acknowledged commit: lost lease, interrupted stop or uncertain commit |
| Growing pending count / oldestDue | Queue backlog; read stats for a targeted prefix |

Abandoned spans have no exception event even if the handler failed. Timer spans are new traces linked to the sending
turn; ordinary messages propagate their parent. Mailbox depth requires a store query and is not an exported metric.
