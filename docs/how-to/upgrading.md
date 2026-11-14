# Upgrade Cicada

Use this procedure when changing the library version or an actor contract. Read the entries for every version you cross.


Stores write schema version 6 and apply any missing migrations in place when they open a database. Migrations are
additive, so nodes of the previous version keep working during a rolling deploy, unless a release says otherwise: a
migration that is not additive breaks the nodes of the previous version, so every old node is stopped before the first
new one starts. The changelog says so for each such release. From 0.11.0 on both stores name the columns they read, so
a later migration that only adds a column with a default can be additive: the statements of a running node of the
release before keep working while it is applied.

**Version 0.4.0 is such a release.** Its migration 2 (every actor has state, ADR 0018) gives every actor without a
state the state `{}`, makes the state column required, and removes the registrations of the types that were
stateless; an old node that writes an actor without a state fails. Stop every node of the old version, then start the
new ones, the nodes that host the formerly stateless types first. Until a node that hosts such a type starts, its
actors can be read and deleted, and messages already in a mailbox wait; a create of one and a send to one from outside
a turn are refused (over HTTP `404 no-such-type`), a retry of a failed message of one is accepted and waits, and a turn
on another node that creates one ends `failed` without a retry. The
changelog of 0.4.0 lists the steps, including the code changes.

**Version 0.6.0 is such a release.** A key is one name (ADR 0021), so key fields leave every contract and every
contract hash changes. Its migration 3 removes every registration and rewrites `%3A` to `:` in the address columns:
a `:` is an ordinary character of a name now, so `account/acme:7` is the address of the name `acme:7`. Before the
upgrade, find the old keys of two fields that become one name with the query in the changelog of 0.6.0 and delete one
actor of each pair with the old version; otherwise the migration fails and the database stays at version 2. A store
of 0.7.0 or later reports that on both stores as an invalid store that names schema version 3 and holds the database's
message, and a node stops; a store of 0.6.0 reported it as unavailable (on SQLite the store did not open), so a node
that retried an unavailable store kept retrying. Stop every
node of the old version, then start the new ones: the nodes that host a type before the nodes whose turns create
actors of it, and nodes that only send last. Until a node that hosts a type starts, that type is not registered, with
the effects described for 0.4.0 above; where two kinds of nodes create actors of each other's types, hold back the
messages that start such turns until every node is up. Upgrade `ActorClient` users together with the nodes: an old
client cannot decode the views of a new node. A prefix that holds the encoded colon itself (`x%3A` in code,
`%253A` in a URL) is written with `:`. The changelog of
0.6.0 lists the steps, including the code changes and what happens to a ref that changes its shape.

**Version 0.8.0 is such a release.** An actor may have a parent (ADR 0019), and a delete or a stop removes the actor's
descendants with it. Its migration 4 adds the column `actors.parent` and an index on it; every existing actor is
independent. A node of 0.7.0 that opens the migrated database refuses it, but one that is still running does not
notice: its deletes and stops remove one row and leave the children behind, as orphans that never go away. So stop
every node of the old version, then start the new ones. Clients of 0.7.0 keep working: they ignore the new field
`parent` of an actor's view.

**Version 0.9.0 is such a release.** A watcher is told of the failures and removals of the actors it watches, in the
transaction of the event (ADR 0023). Its migration 5 adds the table `watches` and the column `messages.notification`;
no existing message is a notification and no watch is stored. A node of 0.8.0 that opens the migrated database refuses
it, but one that is still running does not notice: a message that fails there tells no watcher, and an actor it removes
keeps its watches as a watcher. So stop every node of the old version, then start the new ones. Clients of 0.8.0 keep
working: they ignore the new field `notification` of a message's view. A host that ran `FailureWatcher` kept it until
0.14.0, which replaces it with a watch.

**Version 0.10.0 changes no schema.** A turn can pause, resume, purge and delete another actor (ADR 0024); its commit
applies those effects, and an actor on a node of 0.9.0 is paused or deleted as it would be from outside. So nodes of
0.9.0 and 0.10.0 can share a database during a rolling deploy, and clients need no upgrade.

**Version 0.11.0 is such a release.** A message counts its lost claims, and a node fails a message at its claim limit
(ADR 0025, [The claim limit](../operations.md#the-claim-limit)). Its migration 6 adds the column `messages.lost_claims`; no existing
message starts with a lost claim. A node of 0.10.0 that opens the migrated database refuses it, but one that is still
running does not notice: it counts no lost claim, so the claims it replaces escape the limit, and its statements that
read a message with `*` break on the changed table. On PostgreSQL they fail ("cached plan must not change result
type") until the connection is replaced; on SQLite the worker of the driver (sqlx) crashes on them, and the store
answers unavailable. So stop every node of the old version, then start the new ones. Clients of 0.10.0 keep working:
they ignore the new field `lostClaims` of a message's view. A node that had no `CrashLoopGuard` now has a limit of 5;
the changelog of 0.11.0 lists the code changes.

**Version 0.12.0 changes no schema.** A send and a create over HTTP are checked against the registered JSON Schema of
their type on every node (ADR 0026), so nodes of 0.11.0 and 0.12.0 can share a database; during a rolling deploy a node
of 0.11.0 still answers as before. A client gets `422 invalid-payload` or `422 invalid-state` where it got `202` or
`201` in two cases: what it sends serde accepts and the schema refuses (a field under its `#[serde(alias)]`, a form
that only a hand-written `Deserialize` takes), or it sent anything that does not match its schema to a node that does
not host the type. Before the nodes are upgraded:

1. Add `cicada_axum::assert_message_sample` and `assert_state_sample` to the tests of each type, with what the type
   serialises and with what clients send over HTTP ([The schema check](../guide/http.md#the-schema-check)).
2. A sample that fails only the schema shows a type whose schema and serde disagree. When it is a form that only some
   clients send (an alias), stop sending it. When it is what the type itself writes (a field with `#[serde(with)]` and
   no `#[schemars(with)]`), the schema is wrong and every send of the typed client would be refused; there is no switch
   to turn the check off. Making the schema right changes the contract of the type: follow
   [Changing a contract](../operations.md#changing-a-contract).

Clients need no upgrade.

**Version 0.13.0 changes no schema and no wire format.** It adds `cicada-app` (ADR 0027), so nodes of 0.12.0 and
0.13.0 can share a database. A host may move its start-up code onto `App`; nothing forces it. The command line moved
from `cicada-cli` into `cicada-app` and is still the binary `cicada`; install it with `--features tui` for the
terminal UI. A deployment of the sample service renames its variables ([Sample service](../guide/app.md#sample-service));
a node of it refuses to start while one of the old ones is set.

**Version 0.14.0 changes no schema of the store, but it changes a wire format.** A `Failed` holds its cause (ADR
0028), so the contract of every type that watches changes, and `cicada-actors` replaces `cicada-extras`, whose dead
letters get a new contract. A system without a type that watches and without dead letters can deploy it rolling: nodes
of 0.13.0 and 0.14.0 share a database, and clients need no upgrade. A system with either cannot; follow the steps:

1. **The crate.** Replace `cicada-extras` by `cicada-actors` in the manifests. `Schedule` keeps its name
   (`cicada_actors::Schedule`). Drop `FailureWatcher` and the task that ran it: a watch replaces it (step 3).
2. **A type that watches.** Its contract changes with `Failed`. A `Failed` that a node of 0.13.0 wrote has no cause, and
   a watcher of 0.14.0 cannot decode it: that message ends `failed`, and a notification that fails tells nobody. So
   first let the watchers handle the notifications in their mailboxes (`GET /stats?prefix=<type>/` shows none
   pending), then follow [Changing a contract](../operations.md#changing-a-contract) for each such type. A handler that builds a
   `Failed` by hand adds the cause.
3. **Dead letters.** Their contract changes too, and the letters of 0.13.0 are lost:
   1. Save the letters if they matter: `cicada get actor cicada.dead-letters/_ -o json`.
   2. Stop every node. While a node of 0.13.0 runs, its `FailureWatcher` creates the actor again after it is deleted.
   3. Start one node of 0.14.0 that does not host the type (`serve --types none`). Delete the actor and then the
      registration: `cicada delete actor cicada.dead-letters/_`, then `cicada delete type cicada.dead-letters`. A node
      that hosts the type refuses to start until that is done: its contract conflicts with the registered one.
   4. Start the other nodes, create the actor again (`DeadLetters::default()` with `DeadLettersState::default()`, or
      `cicada create actor cicada.dead-letters/_ --state '{}'`) and set its watch on every type: `ActorSystem::watch`, or
      `cicada create watch --target '*' --watcher cicada.dead-letters/_`.
4. **A supervisor and a topic** are new: nothing moves. Both watch, so create them only when no node of 0.13.0 is left:
   such a node writes a `Failed` without a cause. Host a supervisor on nodes that host the types of its children, so
   that a wrong entry is refused when the child starts ([Supervisor](../guide/actors.md#supervisor)).

The sample service no longer reads `FAILURE_WATCHER_INTERVAL`: a node that hosts the dead letters creates them and their
watch at the start.

**Version 0.15.0 changes no schema of the store, but it changes the contracts of some types.** A message schema holds
every definition it refers to, whatever its name, and a reference to a family that holds itself accepts any value;
[The contract of a type](../guide/core.md#the-contract-of-a-type) names the three kinds of type that are hit. The contract
hash of such a type changes, so a node of 0.15.0 that hosts it refuses to start against the registration of the
release before (`ContractConflict`). Every other type keeps its contract byte for byte: a system without a type that
is hit can deploy 0.15.0 rolling, with nodes of 0.14 and 0.15.0 on one database. Clients need no upgrade. Before any
node is deployed:

1. Raise the dependency to 0.15.0 and run the contract snapshot tests of every type (`assert_contracts_match`) against
   the snapshots of 0.14.0 (0.14.1 makes the same contracts). A snapshot that changes names a type that is hit.
2. For such a type, follow [Changing a contract](../operations.md#changing-a-contract): a versioned type name, or a stop of the world
   that stops every node that hosts it, deletes its actors and then its registration (`DELETE /types/{name}` answers
   `409` while an actor of the type exists).

A type with a newtype variant of a type whose name a reference escapes could not start before 0.15.0, so nothing of it
is registered: it registers its first contract.

**Version 0.16.0 changes no schema of the store, but it refuses values nested deeper than 64 levels** (ADR 0029,
[How deep a value can be](../guide/core.md#how-deep-a-value-can-be)). Nodes of 0.15.0 and 0.16.0 can share a database.
During a rolling deploy a node of 0.15.0 still takes a deeper value, and a client that repeats such a send to a node of
0.16.0 gets `400`, not the answer to a repeated send, as the limits come before the repeated id. Clients need no
upgrade. No check before the upgrade finds a deep value; it shows up as a refusal or a failed turn after it.

1. A host that sends or stores values deeper than 64 levels makes them shallower before the upgrade. Over HTTP a
   payload or a state of 65 to 126 levels was checked against its schema and is `400` now; from code any depth was
   taken. Mind the values that a host keeps inside its own states: a state that holds them a few levels down holds
   them a few levels less deep.
2. After the upgrade, an actor whose turns fail with `State cannot be stored: The state is nested deeper than 64
   levels` holds a state from before that is too deep. Its turns that leave the state as it is still end as before.
   Give it a shallower state from its handler, or delete it and create it again, which also drops its mailbox and its
   children and tells its watchers `Stopped` (a supervisor of it then forgets it as a child). A state deeper than 126
   levels, whose view `cicada-client` refuses, can be read with a parser that allows more, such as `curl` and `jq`.
3. The ready-made actors keep values a few levels down ([Ready-made actors](../guide/actors.md)):
   - **Dead letters** that hold a letter whose payload is deeper than 61 levels record nothing more, and a `Redrive` or
     a `Discard` of another letter fails, until that letter is discarded: send `Discard` with its `messageId`.
     Deleting and creating the actor works too, and drops all its letters.
   - **A supervisor** whose state holds a child state deeper than 61 levels or a start payload deeper than 60 fails
     every turn that writes its state. Delete it, which deletes its children too, and create it again with a shallower
     state; its `Start` starts them again.
   - **A topic** with a pending `Publish` whose payload is deeper than 62 levels fails that `Publish`, as its
     `Deliver` is refused at the call; the topic goes on with the next message.
4. An operator cannot retry a message stored before the upgrade whose payload is deeper than 64 levels (`409
   message-not-retryable`); a bulk retry skips it.

**Version 0.16.10 changes no schema and no wire format of the HTTP API.** It adds change hints (ADR 0034,
[Follow changes](../guide/core.md#follow-changes)). On PostgreSQL every transaction that changes an actor now also sends
an `@` payload on the store's channel; nodes of 0.16.9 ignore those payloads, so both versions can share a database.
Only nodes of 0.16.10 send hints, though: a host that follows changes sees the writes of an older node only at its
next `Resync`. Upgrade every node that writes, then let the followers subscribe again (or restart them), so that they
read everything once.

**Version 0.16.11 changes no schema, contract or existing route.** It adds `GET /changes` (ADR 0035), so a page that
uses it needs a server of 0.16.11; an older one answers `404`. A node that serves it still sees only the hints of
nodes of 0.16.10 or later. With a page open, `serve` may now use its whole 5 s drain on stop.

**Version 0.16.13 changes no schema, contract or route.** It adds leases for executors outside the node (ADR 0037,
[Lease turns for an executor outside the node](../guide/core.md#lease-turns-for-an-executor-outside-the-node)), and a
renewal no longer shortens a held lease. Nodes that share one `lease_ttl` see no difference, so nodes of 0.16.12 and
0.16.13 can share a database. Lease and renew through nodes of 0.16.13 only: a node of 0.16.12 still sets a renewed
lease to its own TTL, and one with a shorter TTL could end a lease a worker still holds.

**Version 0.16.14 changes no schema, contract, route or store behaviour.** It adds the completion of leased turns
(ADR 0038, [Complete a leased turn](../guide/core.md#complete-a-leased-turn)); nodes of 0.16.13 and 0.16.14 can share a
database. Complete through nodes of 0.16.14 only: an older node has no `complete_turn`.

**Version 0.16.15 changes no schema, contract, route or store behaviour.** It adds `Worker`, which runs Rust actors
through a turn source (ADR 0039, [Run actors in a worker](../guide/core.md#run-actors-in-a-worker)); a node runs its turns
as before. A worker leases, renews and completes through its source, so serve it from nodes of 0.16.14 or later.

**Version 0.16.16 changes no schema, contract, store behaviour or existing route.** It adds the four `/leases` routes
of the full API and `ActorClient` as a turn source (ADR 0040, [Run a worker over
HTTP](../guide/http.md#run-a-worker-over-http)). Point a worker over HTTP only at servers of 0.16.16 or later behind its
load balancer: an older server answers `404`, which stops the worker with a fatal error. Extend the host's
authentication to `/leases` and its children before exposing them.

**Version 0.16.17 changes no schema, contract, route or store behaviour.** `worker` of `cicada-app` can run over HTTP
(ADR 0042, [A worker over HTTP](../guide/app.md#a-worker-over-http)) with `--server`, `--context` and `--token`, which it
refused before. A worker on the store stays one: `--store`, or a non-blank `CICADA_STORE`, wins over `CICADA_SERVER`
and `CICADA_CONTEXT` set for the operator commands, and it reads neither the config file nor `CICADA_TOKEN`. Only a
worker without a store that has `CICADA_SERVER` or `CICADA_CONTEXT`, which failed before, now runs over HTTP.

**Version 0.17.0 changes no schema of the store, but it changes the contract of the dead letters** (ADR 0043). A
letter keeps a payload deeper than 61 levels as JSON text in `payloadJson` ([Dead letters](../guide/actors.md#dead-letters)),
so the contract hash of `cicada.dead-letters` and the Rust type `DeadLetter` change; a `Failed`, every other contract
and the store stay as they were. A system without dead letters can deploy 0.17.0 rolling, with nodes of 0.16 and
0.17.0 on one database. JSON text is now parsed with exact float rounding on every node of 0.17.0, which only makes a
read more exact. A system with dead letters cannot deploy rolling: a node of 0.17.0 that hosts them refuses to start
against the registration of 0.16 (`ContractConflict`), and a worker of 0.16 that leases them is refused
(`ContractMismatch`). But a start-time check does not stop a node that runs already, and a reader of 0.16 cannot
decode a letter that holds `payloadJson`. Keep the letters across a new registration in a maintenance window, with the
tested steps of [examples/src/dead_letters_upgrade.rs](../../examples/src/dead_letters_upgrade.rs) (example 16):

1. **Check the shape.** The steps keep dead-letters actors that are independent (no parent), have no children and that
   no actor watches by address. Deleting another shape removes its children and sends `Stopped` to its watchers: give
   it a migration of its own. `export` checks this and stops, and so do the steps below whenever a check fails.
2. **Quiesce.** Stop the senders of the watched work and let the watched turns finish, then let every dead-letters
   mailbox drain (`GET /stats?prefix=cicada.dead-letters/` shows nothing pending, and no turn runs). A failure that
   happens while the dead letters are gone tells nobody, and no letter is written for it. If the work cannot stop,
   that gap stays: an outcome may be expired or hidden by then, and retrying the business messages is no copy of the
   letters, as it can repeat their external effects.
3. **Stop every node of 0.16**, local and remote workers too, and every hook that creates the dead letters at its start
   (the sample service does). A node of 0.16 still running would go on reading and writing the old shape.
4. **Back up the database, and export.** On a management node of 0.17.0 that hosts nothing, `export` reads every
   dead-letters actor: its state, version, pause flag and the watches it holds. Write it to a file with `serde_json`,
   never through a tool that rounds numbers, and check the file before you go on. Keep the backup until the end.
5. **Convert.** `convert` moves each raw payload deeper than 61 levels to `payloadJson`, keeps every letter with its
   metadata and `redrivenAs`, and runs `check`: each state must be one that `restore` can create, with no NUL
   character in a string or a key, at most 64 levels, and the state type of 0.17.0. A legacy payload deeper than 64 levels is kept as text; a redrive refuses it, and `Discard` drops it. An
   export that cannot be read needs a recovery from the backup.
6. **Reset.** `reset` runs `check` again and checks that the store holds exactly the exported actors at their exported
   versions; only then does it delete them, remove the registration and register the contract of 0.17.0.
7. **Restore.** `restore` creates each actor again at its address with its converted state, pauses the mailboxes that
   were paused, sets its watches again and compares. The actors are new instances: their version, epoch and creation
   time are new, and no lease held before is valid. Then start the nodes of 0.17.0 and resume the senders.

<!-- doctest
@@4
# async fn upgrade(store: cicada::Store) -> Result<(), Box<dyn std::error::Error>> {
@@7
# Ok(())
# }
-->
```rust,no_run
// examples/src/dead_letters_upgrade.rs
use cicada::ActorSystem;
use cicada_examples::dead_letters_upgrade::{Export, convert, export, reset, restore};

let manage = ActorSystem::builder(store).build(); // hosts nothing
std::fs::write("dead-letters.json", serde_json::to_string_pretty(&export(&manage).await?)?)?;
// … check the file and keep the backup …
let mut saved: Export = serde_json::from_str(&std::fs::read_to_string("dead-letters.json")?)?;
convert(&mut saved)?;
reset(&manage, &saved).await?;
restore(&manage, &saved).await?;
```

Clients that read the state of the dead letters with `DeadLetter` upgrade with the nodes: `payload` is a
`LetterPayload`, read with `value()`. A raw letter deeper than 61 levels in a state that is not converted still fails
every turn that writes the state; nothing repairs it by itself.

**Switching a type to `create_child`.** An actor created with `turn.create` stays independent: a later `create_child`
of it by its creator does not adopt it, but fails the creator's message (the commit is rejected, and a handler cannot
test for that). So switch a parent type from `create` to `create_child` only when no parent of it is left that created
actors the old way: start its data anew, or let those parents finish first. New parents can use `create_child` from
the start.

**Order of a rolling deploy.** For an additive migration: the first node of the new version migrates the database;
from then on a node of the old version refuses to start (it does not open a schema newer than its own), while old nodes
that are already running keep working. So: start the new nodes, then stop the old ones. Do not restart an old node
after the migration (a crash-looping or rescheduled old pod will not come back), and do not roll back against a
migrated database.

**Upgrade nodes before clients.** Nodes decode request bodies strictly, so a request field a node does not know yet
is a `400`. `ActorClient` leaves absent optional fields out of request bodies (no `"retry": null`), so a new client
works against an older node as long as it does not use a newer feature. A release whose views lose a field that old
clients require, as 0.6.0 does, needs its clients upgraded together with the nodes; the changelog says so.

## Upgrade flows to 0.18.0

**Every flow contract changes; the store schema and other types' contracts, including the event, do not**
([ADR 0044](../adr/0044-flow-requests-waits-and-cancellation.md)). A 0.18.0 host refuses a flow registration from
0.17 with `ContractConflict`; a 0.17 worker leasing the new type is refused with `ContractMismatch`.
Choose one deployment:

1. **New type names.** Keep pinned 0.17 binaries hosting the old flow names until their runs finish. Host new code
   under new names, such as `flows.onboarding-v2`, on 0.18.0 binaries sharing the store. Every co-hosted type must
   still match its registration. Renaming inside an upgraded binary alone does not keep the old runs working.
   [The onboarding upgrade test](../../examples/tests/flow_onboarding.rs) demonstrates the two registrations.
2. **Drain and reset.** Stop starting runs; let existing runs finish and archive what you need. Stop all hosts and
   workers of the type, delete its runs, delete its registration, then start 0.18.0 hosts. Deleted journals are gone.

The wire now reserves Reply/Cancel messages, cancelling/cancelled statuses, send/wait entries, kept replies and a
cleanup journal. Only steps, sleeps and sends execute in 0.18.0. Exhaustive matches on `FlowMsg`, `FlowStatus` or
`JournalEntry` need the new variants; `FlowState` literals need `replies` and `cleanup`, or `..FlowState::new(input)`.

Saved 0.17 progress decodes with empty new lists; that **does not make registrations compatible** or provide a
migration utility. Restoring exported progress at the same address is a host-specific procedure for the **same job**:

1. Quiesce all producers and workers. Verify the backup, typed decoding and core state limits before deleting anything.
2. Clear an armed id whose message no longer exists. Preserve journal entries and saved results.
3. Restore pause flags, watches and child topology deliberately; deleting a run also removes its children.
4. Restore the run and send a public Continue to each running run when ready to resume.

A restored run has a new instance identity and mailbox: pending messages are lost, and an uncommitted external step
may repeat with its old key. Do not reuse its address for a different job.

Later reply-wait, timeout, join, cancellation and cleanup features are intended to keep the reserved contract.
Equal hashes therefore cannot prove feature support: upgrade **every host and worker of a flow type** before enabling
new flow code or newly supported messages, or use a new versioned type name. A 0.18.0 host permanently refuses Reply,
Cancel, cancelling runs and replay of saved requests/waits.

## Enable flow replies in 0.18.1

No store schema or flow contract hash changes. `Step::request`, `Step::wait` and `ReplyTo::send` now support a single
reply wait without a timeout. Upgrade **every host and worker of the flow type** before deploying flow code that
requests, or use a new type name: 0.18.0 permanently refuses Reply and cannot replay a saved request despite equal hashes.

A reply refused by 0.18.0 must be sent again by its target after upgrading. Dead-letter redrive sends as the dead
letters actor, so the run ignores it. A run stalled at a saved request recovers with public Continue after upgrading.
Adding `ReplyTo<T>` to a target's message changes that target's contract: follow
[Changing a contract](../operations.md#changing-a-contract).
