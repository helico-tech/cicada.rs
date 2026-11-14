# Core: build an actor

Use `cicada` to define actors and embed a node; use `cicada-test` to test them on virtual time.
If you just need a runnable binary, combine the actor below with [App](app.md).

## Define an actor type

An actor definition has four parts: a ref (its address), a message enum, state, and a handler.
An instance is created explicitly. Its state and buffered effects commit together after each successful handler.

With the [SQLite app dependencies](../how-to/install.md#choose-the-crates), put this in `src/main.rs`:

```rust,no_run
use cicada::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Counter {
    pub name: String,
}

impl ActorRef for Counter {
    const TYPE: &'static str = "counter";
    type Message = CounterMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum CounterMsg {
    Add { amount: i64 },
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct CounterState {
    #[serde(default)]
    pub total: i64,
}

pub struct CounterActor;

impl Actor for CounterActor {
    type Ref = Counter;
    type State = CounterState;

    async fn handle(&self, turn: &mut Turn<Self>, message: CounterMsg) -> HandlerResult {
        match message {
            CounterMsg::Add { amount } => turn.state_mut().total += amount,
        }
        Ok(())
    }
}

fn main() -> std::process::ExitCode {
    cicada_app::App::new().host(CounterActor).run()
}
```

Start the node, then create and use an instance from another terminal:

```sh
cargo run -- serve --store actors.db
# In another terminal:
export CICADA_SERVER=http://127.0.0.1:8080
cargo run -- create actor counter/a --state '{}'
cargo run -- send counter/a --type Add --payload '{"amount":5}'
cargo run -- get actor counter/a -o json
```

After the message runs, `state.total` is 5. The create initializes it to 0 through `#[serde(default)]`.
Keep the API on loopback until the host [adds authentication](http.md#run-it-in-an-axum-service).

**Addressing.** The ref serializes as one string: a struct with one string field or a string newtype.
A unit ref is a singleton at `type/_`. Keys are non-empty; reserved characters are percent-encoded, and `:` is literal.
For a composite key, implement conversion to one string; see [fanout.rs](../../examples/src/fanout.rs).

**State.** Every actor has state. Use `NoState {}` for an actor without data; state cannot be JSON `null`.
Use defaults for new fields and keep [state samples](#state-samples) from earlier versions.

### Design an actor

Choose the key for the work that must run serially: one account, import or document. Different keys can run in parallel.
Keep dependencies on the actor struct and durable progress in state. A handler processes one bounded piece of work;
send the next piece to yourself when it must survive a crash.

| Design decision | Rule of thumb | Example |
| --- | --- | --- |
| Consistency boundary | Keep data for one invariant together; cross-actor agreement requires messages | [Saga](../../examples/src/saga/) |
| Key | Use a stable tenant/order id; distinguish reused identities when work from an old life can remain | [Registry](../../examples/src/registry.rs) |
| State size | Keep bounded history, child actors or references to external data | [Fan-out](../../examples/src/fanout.rs) |
| Turn size | Process one bounded piece and queue the next; a singleton serializes all its work | [Paged import](../../examples/src/sync.rs), [singleton](../../examples/src/singleton.rs) |
| Message content | Carry the command/fact and data the receiver needs, including an explicit reply address when forwarding | [Request/reply](../../examples/src/reply/) |

Internal effects commit once; external calls can repeat after an abandoned claim or sender-requested retry.
Use `turn.message().id`, a flow step's key, or a business id that must deduplicate across messages. Attempt numbers,
clocks and new random ids do not protect repeated work. Put separate durable steps between external effects that
must retain independent progress; see [steps.rs](../../examples/src/steps.rs).

A turn cut off at its maximum duration retries only under its message's policy. A handler that returns only after the
maximum (blocking code) fails without replay. Use `spawn_blocking` for blocking work, but remember that cancelling its
await does not stop the underlying call. Exact rules are in [turn semantics](../architecture.md#turn-semantics).

Wait durably with [timers](#timers) or [flow sleep](actors.md#durable-sleep), rather than keeping a handler open to wait
for time or another actor. Bounded service calls within the turn timeout are fine.
Reading old state with serde defaults does not make its contract equal: snapshot changes still need a
[contract deployment plan](../operations.md#changing-a-contract). Check the [ready-made actors](actors.md) first when
one matches the job.

| In a handler | Use |
| --- | --- |
| Read the address or state | `self_ref()`, `self_address()`, `state()` |
| Change state | `state_mut()`, `set_state(..)` |
| Send | `send`, `send_with`, `send_self`, `send_self_with` |
| Send to a runtime address | `send_to` or `send_raw` (async registry lookup) |
| Create | `create`, `create_child`; `create_raw` variants for runtime addresses |
| End this actor | `stop()` |
| Refuse this message permanently | `return Err(turn.fail("reason"))` |
| Read envelope data | `message()` — id, sender, counters, headers, correlation and cause |
| Read time | `now()` is fixed at claim; `current_time()` adds elapsed turn time |

A `delay` counts from commit; `at` is absolute. Use `delay` for a wait after long work, rather than adding to `now()`.
The exact outcome rules live in [turn semantics](../architecture.md#turn-semantics).

### How deep a value can be

Payloads and states can contain at most **64 nested arrays/objects**, with no NUL in strings or keys.
Sends and creates reject invalid values; a handler producing invalid state fails without changing stored state.
Registered schemas have a separate limit of 120 levels.

A `Failed` wraps a payload one level deeper. If you keep it in state, account for that wrapping; the ready-made
[dead letters](actors.md#dead-letters) store deep payloads as text. Legacy data is covered in the
[upgrade procedure](../how-to/upgrading.md).

Avoid recursive untagged enums: decoding invalid input can take exponential time. Bound the depth of JSON you build
in code too; extremely deep values can overflow the stack during serialization or drop.
Finite floats round-trip exactly through JSON text; PostgreSQL normalizes negative zero to zero.

## Child actors

`turn.create_child(&child, &state)` gives the child a permanent parent. Deleting or stopping the parent removes its
whole tree and purges those mailboxes atomically. An existing actor with another parent, or no parent, is never adopted:
the commit fails. `create` makes an independent actor.

Pause, resume and purge affect only one actor. `list_children` returns direct children, paged by address.
See the [fan-out example](../../examples/src/fanout.rs).

## Watches

A watcher’s message family declares both `Failed(cicada::Failed)` and `Stopped(cicada::Stopped)`.
Use `turn.watch(target)` / `unwatch(target)`, or the corresponding `ActorSystem` operations from outside a turn.

| Target | Notification |
| --- | --- |
| One address, `worker/a` | Final failures and removal (`Stopped`) |
| Type prefix, `billing` | Failures of `billing` and `billing.*`, excluding `billings` |
| `*` | Every final failure |

A watch survives deletion and recreation of its target; deleting the watcher removes its watches.
Notifications commit with their event. Retried or cancelled messages do not notify, and a notification that fails
notifies nobody. Inspect its outcome directly. `Failed.cause` distinguishes handler, node, refused, timeout, message,
state, commit and claim-limit failures; see the [cause table](../architecture.md#turn-semantics).
Adding the two variants changes your contract. [Supervision](../../examples/src/supervision.rs) shows the full setup.

## Control effects

| Call | At commit |
| --- | --- |
| `pause(&actor)` | Stop new claims; let the running turn finish |
| `resume(&actor)` | Allow claims again |
| `purge(&actor)` | Purge pending messages except the running one |
| `delete(&actor)` | Remove the actor, descendants and mailboxes; fence running turns |

The `_at` variants take raw addresses. Missing targets are ignored. A turn stops itself with `stop()`, and cannot
delete an ancestor. To restart a child, call `delete` then `create_child`; creating before deleting is invalid.
Effects apply in a fixed order, regardless of call order: [turn semantics](../architecture.md#turn-semantics).
An old handler may briefly continue after deletion, but cannot commit. Its external calls still need idempotency.

## Open a store and run a node

Open the store, build and start the system, then stop the system before closing its store.
For most binaries, [App](app.md) owns this lifecycle for you. To embed directly:

```rust,no_run
use cicada::{ActorSystem, Store};
use cicada::store::SqliteOptions;
use cicada_examples::sync::ReportsActor;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let store = Store::sqlite("actors.db", SqliteOptions::default()).await?;
    let system = ActorSystem::builder(store.clone()).host(ReportsActor).build();
    system.start().await?;
    tokio::signal::ctrl_c().await?;
    system.stop().await;
    store.close().await;
    Ok(())
}
```

`ReportsActor` is from [sync.rs](../../examples/src/sync.rs). Select the `sqlite` or `postgres` feature to open that store.
`start()` checks hosted types and registers contracts; a different registered hash is `StartProblem::ContractConflict`.
`Store` clones share one backend; `ActorSystem` clones share one node. The caller owns the store.
For shutdown deadlines and a cancelled `close()`, see [shutdown guidance](../operations.md#shutdown-and-probes).

### List actors with their state

`list_actors` returns addresses. Use `list_actors_with_state(prefix, cursor, limit)` for complete snapshots in one read
per page. Keep the prefix and pass each page’s `next` cursor to the next call. Prefixes are literal address prefixes.

Rows in a page share a committed snapshot; pages do not. Versions count state writes of one actor life, restart at 0
on recreation, and do not track pause/resume. Pick small pages for large states. One unreadable row fails the page.

### Follow changes

Subscribe to `changes(prefix)` **before reading**. On the initial `Resync`, replace your view by listing the prefix;
then process queued hints. On `Changed { address }`, reread that actor, or remove it from the view if it is gone.
On every later `Resync`, list again.

Hints are best effort, without replay, state or versions. They cover committed actor changes, not mailbox or lease
changes. Memory/SQLite cover one `Store` and its clones; PostgreSQL covers upgraded nodes on the same schema.
`stop()` leaves the feed open; `Store::close()` ends it. Keep a periodic full read for separately opened SQLite stores,
older PostgreSQL writers or manual SQL. See [HTTP changes](http.md#follow-changes-over-http) for browser consumers.

### The contract of a type

`contract_of::<A>()` generates state and message schemas. Contracts must have exactly equal hashes on every host of a
type; schema descriptions are excluded from the hash. A compatible Rust change can still change the contract.
Pin it with [snapshots](#contract-snapshots) before deployment.

The HTTP API checks registered schemas; the turn decodes with serde. Recursive message-family references are loosened
in per-message schemas, so nested messages rely on decoding. An unusable schema is not enforced at the HTTP door;
[sample tests](http.md#the-schema-check) catch this. The 0.15.0 compatibility cases are in the
[upgrade procedure](../how-to/upgrading.md).

### Register a contract without hosting

Call `register_contract(&contract_of::<A>()?)` on a handler-free system. An equal hash returns the stored definition;
a different hash conflicts and replaces nothing. Creates and sends can then wait for a worker to start.
This does not start a node or prove that a handler exists. [HTTP registration](http.md#register-a-type-over-http)
uses the same operation.

### Lease turns for an executor outside the node

Use [Worker](#run-actors-in-a-worker) for Rust actors. Custom executors use `lease_turns`, `renew_turn` and
`release_turn`, with the server’s TTL and claim limit. Each request names 1–100 types and their exact hashes and
claims 1–100 turns. A lease is identified by actor and epoch, not an authenticated owner.

Keep a monotonic deadline from request start: `request_time + ttl - min(ttl / 10, 1 s)`. Never run past it.
Stop work before releasing; **never release after the handler ended**. Dropping a returned lease does not release it.
Implementors must read the complete [lease protocol](../architecture.md#leases-outside-the-node).

### Complete a leased turn

`complete_turn` takes `TurnReport::Returned`, `Failed` or `Cancelled`. It makes one fenced commit attempt.

| Result | Executor action |
| --- | --- |
| `Completed` | Finish observers with the acknowledged result |
| `Rejected(failure)` | Permanently replace the current report with that failure; submit it next |
| Store/transport uncertainty | Repeat the current report within the lease and completion budget |
| `LeaseLost` | Abandon; the earlier commit may have applied |

Never rerun a finished handler or restore effects after a rejection. The server checks storage limits; the executor
owns type checks and buffering. Read [the protocol](../architecture.md#leases-outside-the-node) before implementing it.

### Run actors in a worker

`Worker::builder(source).host(actor).build()` runs unchanged actors through a `TurnSource`.
`ActorSystem` is an in-process source; `ActorClient` is an [HTTP source](http.md#run-a-worker-over-http).
Call `start()` to register and poll, `stop()` to drain, and observe `stopped()` for fatal errors.

Workers have no node row, retention sweep or claim limit of their own. The source grants TTLs and applies its claim
limit. Concurrency includes in-flight lease requests. A worker never closes or stops its source.

### Configuration

Pass a `NodeConfig` to `.config(..)`, or use the [App flags](app.md#run-it-as-a-binary).

| Setting | Default | Meaning |
| --- | --- | --- |
| `node_id` | random | Lease owner, shown in `/messages/{id}` while running and by `/health`. |
| `lease_ttl` | 30 s | A turn's lease; a dead node's turn is re-claimed after it expires. |
| `heartbeat_interval` | `lease_ttl / 3` | Lease renewal; must be in `(0, lease_ttl / 2]`. |
| `poll_interval` | 500 ms | Longest sleep of the dispatch loop (wake-ups usually come sooner: own sends, PostgreSQL `NOTIFY`). |
| `default_max_turn` | 15 min | Turn timeout (`failed("timeout")`) unless the type sets `MAX_TURN`. |
| `concurrency` | 16 | Turns running at once on this node. |
| `shutdown_grace_period` | 30 s | Running turns may finish on stop; then they are cancelled and their leases released. |
| `shutdown_cancel_timeout` | 5 s | How long a cancelled turn may take to end before it is left to its lease. |
| `sweep_interval` | 1 h | Deletion of expired outcomes. |
| `max_lost_claims` | `Some(5)` | The [claim limit](#claim-limit): a message with this many lost claims ends `failed` without running; `None` turns it off, `Some(0)` fails the start. Set one value on every node. |
Store retention defaults to 7 days and is also the deduplication window. SQLite has a 5 s busy timeout; PostgreSQL
has a pool of 20 and a 30 s transaction timeout. See [sizing and tuning](../operations.md).

## Retry

The **sender** chooses each message’s `RetryPolicy` through `SendOptions::retry`. Without it, a failed handler is not
retried. `max_attempts` includes the first run; backoff is fixed or capped exponential.

<!-- doctest
@@5
# Ok::<(), Box<dyn std::error::Error>>(())
-->
```rust
use std::time::Duration;
use cicada::prelude::*;

let backoff = Backoff::exponential(Duration::from_secs(1), 2.0, Duration::from_secs(30))?;
let options = SendOptions::new().retry(RetryPolicy::new(4, backoff)?);
```

Use `send_with(&target, message, options)` to apply it. Retries discard the failed run’s buffered effects and keep the
message id. `turn.fail(..)` / `HandlerError::permanent(..)` veto retries. Undecodable state/messages and rejected
commits are not retried. See [saga](../../examples/src/saga/) for sender policy and external-call idempotency.

`attempt` counts claims, `failures` counts failed runs rescheduled by policy, and `lost_claims` counts replaced claims.
A crash recovery does not consume a policy attempt. Unknown stored policies are read as no policy, with a warning.

## Claim limit

After 5 lost claims by default, the next claimant commits `failed` without running the handler or consulting retry.
A lost claim is one replaced without a commit or release. It can come from a crash, an expired lease or an uncertain
commit; it may belong to work running beside the message that crashed the process.

Set `max_lost_claims` consistently across nodes; `None` disables it, `Some(0)` is invalid.
[Operations](../operations.md#the-claim-limit) covers diagnosis and recovery.

## Message persistence

`SendOptions::retention` sets separate `done` and `failed` rules; missing rules, cancelled and purged outcomes use
store retention. These policies affect visibility, while a minimal record preserves deduplication for the store window.

| Rule | Effect |
| --- | --- |
| `Keep::Remove` | Hide immediately and drop content |
| `Keep::keep_for(age)` | Keep readable for the positive duration |
| `Keep::last(count)` | Keep the newest 1–10,000 matching `Last` records per instance/outcome, within store retention |

`Retention::REMOVE_ON_DONE` is a useful shortcut. Expiry is cleaned by the sweep (default hourly).
A hidden outcome returns 404; resending its id within the dedup window still counts as a duplicate.

## Message causes

A send from a turn records its input message as `caused_by` (`causedBy` over HTTP).
`GET /messages/{id}/caused` lists direct children; follow `causedBy` to walk toward the initiating message.
Availability follows outcome retention.

## Timers

Use a delayed self-send. Keep the returned message id in state, then `turn.cancel(&id)` to replace it atomically.
A generation number can reject obsolete timeouts. The complete [timer example](../../examples/src/timer.rs) tests this;
[cron](../../examples/src/cron.rs) arms the next tick in the same commit.
For scheduled delivery without a custom handler, use the [scheduler](actors.md#scheduler).

## Observers

Attach a `TurnObserver` with `.observe(..)`. `turn_started` returns an `ObservedTurn`; `turn_ended` is called only after
an acknowledged commit. Drop without an end means the node did not confirm a commit. Methods are synchronous and must
not block; panics are caught and logged.

Observers can add send headers, but cannot refuse a turn. Explicit message headers win; invalid or excess observer
headers are omitted. `add_header_for(Sends::Timers, ..)` targets delayed self-sends; `Sends::Others` targets the rest.

### OpenTelemetry

Add `.observe(cicada_otel::ActorTelemetry::global())` and install the host’s SDK providers/exporters.
Use `current_trace_headers()` on sends from outside actors. Turns produce spans and the duration/lag metrics described
in [operations](../operations.md#telemetry).

Ordinary sends propagate the parent. Timers start new traces linked to their sending turn through `cicada-link-`
headers, so recurring work does not create an endless trace. Explicit parents win; links are best effort.

## Testing with `TestActors`

Use `#[tokio::test(start_paused = true)]` and enable tokio’s `test-util` feature. This example tests the reports actor
from [sync.rs](../../examples/src/sync.rs):

<!-- doctest
@@3
# #[tokio::main(flavor = "current_thread", start_paused = true)]
# async fn main() {
@@7
# }
-->
```rust
use cicada_examples::sync::{Reports, ReportsActor, ReportsMsg, ReportsState};
use cicada_test::TestActors;

let test = TestActors::builder().with(|node| node.host(ReportsActor)).build().await.unwrap();
let reports = Reports { tenant: "acme".into() };
test.create(&reports, &ReportsState::default()).await;
test.send(&reports, ReportsMsg::SyncFinished { imported: 5 }).await;
test.run_until_idle().await;
assert_eq!(test.state::<_, ReportsState>(&reports).await.runs, vec![5]);
assert!(test.failures().await.is_empty());
```

`advance_time_by(duration)` processes work along the virtual timeline. `pending`, `outcome`, `version` and `state_json`
inspect results. Nothing runs in the background. Dependencies should use `tokio::time` too.
Use `claim_and_abandon` to simulate a node crash; see the [runtime tests](../../crates/cicada/src/tests/).

### Contract snapshots

`cicada_test::assert_contracts_match("tests/contracts", &[contract_of::<A>()?])` checks for contract drift.
Missing snapshots are written; commit them. Review changes before accepting with `CICADA_UPDATE_CONTRACTS=true`.
A changed hash requires a [contract deployment plan](../operations.md#changing-a-contract).

### State samples

`record_state_sample(dir, name, &state)` saves a shape once. `assert_state_decodes::<State>(dir)` checks every saved
shape with the runtime’s decoder. Keep samples from old versions. Decoding alone does not prove meaning: a renamed
field might silently use a default, so assert important values too.

## Examples

[Trace the examples visually](patterns.md): paged import, fan-out/fan-in, request/reply and durable flow replay.

Each implementation has tests in [examples/tests](../../examples/tests/). Open the source for a complete pattern:

| Task | Source |
| --- | --- |
| Stateless singleton | [singleton.rs](../../examples/src/singleton.rs) |
| Replace a timeout | [timer.rs](../../examples/src/timer.rs) |
| Recurring work | [cron.rs](../../examples/src/cron.rs) |
| Request/reply | [reply](../../examples/src/reply/) |
| Saga with compensation | [saga](../../examples/src/saga/) |
| Durable steps | [steps.rs](../../examples/src/steps.rs) |
| Fan-out with children | [fanout.rs](../../examples/src/fanout.rs) |
| Instance registry | [registry.rs](../../examples/src/registry.rs) |
| Restart and redrive | [supervision.rs](../../examples/src/supervision.rs) |
| Shared rate | [rate_limit.rs](../../examples/src/rate_limit.rs) |
| Latest edit wins | [debounce.rs](../../examples/src/debounce.rs) |
| Bulk output | [batching.rs](../../examples/src/batching.rs) |
| Durable flow and sleep | [flow_onboarding.rs](../../examples/src/flow_onboarding.rs) |
| Flow approvals with deadlines; all-team sign-off and first quote | [flow_approval.rs](../../examples/src/flow_approval.rs) |
| Events starting flows | [flow_events.rs](../../examples/src/flow_events.rs) |
| Worker without database access | [remote_worker.rs](../../examples/src/remote_worker.rs) |
| Preserve dead letters during upgrade | [dead_letters_upgrade.rs](../../examples/src/dead_letters_upgrade.rs) |
| Paged import | [sync.rs](../../examples/src/sync.rs) |
