# Changelog

Releases of Cicada for Rust. The crates share one version. They are used as git dependencies, so pin a tag (README,
Using a release in another project).

Each section is headed `## X.Y.Z — YYYY-MM-DD`; the next version is `## X.Y.Z — unreleased` until it is released. While
the version is 0.x, a breaking change raises the minor number and anything else the patch number. A published tag never
moves: a correction is a new version.

## 0.16.5 — 2026-11-13

Nothing breaks: durable flows, a new kind of ready-made actor whose type name the host chooses. A node that hosts a
flow registers its contract at its first start; nothing changes for the other nodes, and no schema of the store
changes.

### Added

- Durable flows (ADR 0031): a multi-step job written as async code. A host implements `Flow` (its type name `TYPE`,
  its `Input`, `run` and, optionally, `retry`) and hosts it with `FlowActor::new`; each run is one actor, named by
  `FlowRun::<F>::named(key)` and created with `FlowState::new(input)`. In `run`, `step.run(id, closure)` returns the
  saved result of a step, or calls the closure with the key `<run address>/<step id>` and suspends: its result
  commits with the next `Continue`, one new step per turn. A public `FlowMsg::Continue` starts a run or recovers one
  whose continuation was cancelled, purged or ran out of attempts; it arms a fresh `Continue` with the flow's retry
  policy (by default the crate's shared one) and the correlation id, and only that armed `Continue` runs flow code.
  The state is `{"input", "journal", "status", "armed"}`, with journal entries `{"kind": "step", "id", "result"}` and
  `{"kind": "sleep", "id", "until"}`; the sleep entry is for a durable sleep of a later release. `FlowState::result`
  reads a saved result. A changed replay, a step id used twice, an early return, a step future dropped while its
  closure runs, a result that cannot be saved and a 1,001st entry fail the turn for good; a closure's error keeps its
  own retryability, even when the flow catches it. Done runs ignore `Continue` and stay until they are deleted. The
  actors guide, "Flows", has the rules; `examples/src/flow_onboarding.rs` is a worked example beside the unchanged
  `steps` example, and the sample service hosts it as `flows.onboarding` without starting a run.

## 0.16.4 — 2026-11-13

Nothing breaks: a new ready-made actor, the batcher. A node that hosts it registers its contract at its first start;
nothing changes for the other nodes, and no schema of the store changes.

### Added

- The batcher (`BatcherActor`, `cicada.batcher`, one instance for each stream or group, built with `Batcher::named`):
  `Add { item }` appends one JSON value, `null` included, in handling order (`BatcherMsg::add` builds one from any
  `Serialize` value). The `Add` that fills the batch, or that is handled at or after its deadline, sends the batch
  with its own item in the same turn, so a stored batch always holds fewer than `maxItems` items; otherwise the first
  item of a batch arms a public `Flush` at its store time plus `maxWait`. A flush sends the fixed message type with
  `{"items": [...]}` to the fixed target, with the batcher as its sender, the settings' retry policy and no
  correlation id, and cancels the armed flush and clears the batch in the same commit; with nothing held it does
  nothing. Its state holds the target, the message type, `maxItems` (1 through 1,000), `maxWait` and the target's
  retry policy, fixed at the create (`BatcherState::new`, checked like a decoded state, fails with `InvalidBatch`),
  and the items, the batch's first time and the armed flush. A flush that fails clears nothing, and an `Add` whose
  flush fails keeps no item: an operator's `Flush`, retry or redrive sends what is held then. `maxItems` bounds the
  count of items, not their bytes, and due times are not run times. Its contract is in
  `docs/reference/contracts/cicada.batcher.json`; the actors guide, "Batcher", has its rules, and
  `examples/src/batching.rs` is a worked example. The sample service hosts the type and creates no batcher.

## 0.16.3 — 2026-11-13

Nothing breaks: a new ready-made actor, the debouncer. A node that hosts it registers its contract at its first start;
nothing changes for the other nodes, and no schema of the store changes.

### Added

- The debouncer (`DebouncerActor`, `cicada.debouncer`, one instance for each key, built with `Debouncer::named`):
  `Submit { to, message, retry }` replaces the kept submission whole, with its correlation id, and arms a public
  `Flush` at the quiet period from the turn's store time, capped by the longest wait from the burst's first
  submission (`DebouncerMsg::submit` builds one from a typed ref and a message of its family). `Flush` sends the kept
  message to its target, with the debouncer as its sender and the kept retry policy and correlation id, and clears
  the burst in the same commit; with nothing kept it does nothing. Its state holds the quiet period and the longest
  wait, fixed at the create (`DebouncerState::new`, checked like a decoded state, fails with `InvalidDebounce`), and
  the kept `DebouncedMessage`, the burst's start and the armed flush. A flush that fails clears nothing: an
  operator's `Flush`, retry or redrive sends what is kept then. Due times are not run times: the longest wait bounds
  the scheduled flush, not one delayed by a pause or an outage. Its contract is in
  `docs/reference/contracts/cicada.debouncer.json`; the actors guide, "Debouncer", has its rules, and
  `examples/src/debounce.rs` is a worked example. The sample service hosts the type and creates no debouncer.

### Fixed

- The README and the core guide count the twelve worked examples, and the core guide's table of examples lists
  `rate_limit`.

## 0.16.2 — 2026-11-13

Nothing breaks: a new ready-made actor, the rate limiter. A node that hosts it registers its contract at its first
start; nothing changes for the other nodes, and no schema of the store changes.

### Added

- The rate limiter (`RateLimiterActor`, `cicada.rate-limiter`, one instance for each limit, built with
  `RateLimiter::named`): its one message, `Submit { to, message, retry }`, reserves the next free slot of a shared
  rate and sends the stored message to the target, due at that slot, with `retry` as its retry policy there
  (`RateLimiterMsg::submit` builds one from a typed ref and a message of its family). Its state holds the rate, the
  burst and the longest delay, fixed at the create (`RateLimiterState::new`, checked like a decoded state, fails with
  `InvalidRate`), and one GCRA timestamp; it keeps no queue. A slot beyond the longest delay, or a message the target
  does not take, is refused for good and reserves nothing; a target that is gone fails the commit. Slots are due
  times, not runs: messages that wait past their slots can run close together. Its contract is in
  `docs/reference/contracts/cicada.rate-limiter.json`; the actors guide, "Rate limiter", has its rules, and
  `examples/src/rate_limit.rs` is a worked example. The sample service hosts the type and creates no limiter.

## 0.16.1 — 2026-11-13

Nothing breaks: a new ready-made actor, the scheduler. A node that hosts it registers its contract at its first start;
nothing changes for the other nodes, and no schema of the store changes.

### Added

- The scheduler (`SchedulerActor`, `cicada.scheduler`, one instance for each schedule, built with
  `Scheduler::named`): on each slot of its `Schedule` it sends a stored message to a target. The schedule, the target,
  the message and a retry policy for the delivered message are in its state (`SchedulerState::new` builds one from a
  typed ref and a message of its family), so an operator creates and edits a schedule at runtime. `Start` arms the
  chain, `Run` sends now, `Update` replaces the schedule, the target, the message or the retry policy and keeps the
  counts; pause, resume and delete are the core's control. A delivery for a slot carries the header `cicada-slot`
  (`2026-11-13T02:00:00Z`). A target that is gone, or a message it does not take, fails that run alone, and missed
  slots are skipped. Its contract is in `docs/reference/contracts/cicada.scheduler.json`; the actors guide,
  "Scheduler", has its rules. The sample service hosts it.

## 0.16.0 — 2026-11-12

A breaking release: a payload and a state hold at most 64 levels of arrays and objects (ADR 0029). A deeper value
could end a node: it overflowed a stack on its way through the check, the store or the decoder, and the process ended
without an error. No schema of the store changes, and clients need no upgrade. Nodes of 0.15.0 and 0.16.0 share a
database.

### Upgrading

1. A host that sends or stores values deeper than 64 levels makes them shallower before the upgrade. Mind the values a
   host keeps inside its own states: kept a few levels down, they may be that much less deep. No check before the
   upgrade finds such a value; it shows up as a refusal or a failed turn after it.
2. After the upgrade, an actor whose turns fail with `State cannot be stored: The state is nested deeper than 64
   levels` gets a shallower state from its handler, or is deleted and created again, which drops its mailbox and its
   children and tells its watchers `Stopped`. Dead letters that hold a letter whose payload is deeper than 61 levels
   discard that letter (`Discard` with its `messageId`); a supervisor whose state holds a child state deeper than 61
   levels or a start payload deeper than 60 is deleted and created again with a shallower state. "Upgrading" in the
   operations guide has the details, also for a topic.

During a rolling deploy a node of 0.15.0 still takes a deeper value, and a client that repeats such a send to a node of
0.16.0 gets `400`, not the answer to a repeated send: the limits are checked before the repeated id.

**Chorus** keeps the output of a task three levels into the state of a workflow (`steps`, a list of steps): from this
release on, a task output deeper than 61 levels fails the turn of the workflow that would keep it. Check how deep the
outputs of its users' tasks can be, and bound them before they reach the state.

### Changed

- **BREAKING** A payload and a state are at most 64 levels deep (`cicada::runtime::limits::MAX_DEPTH`): `{"a": {"b":
  1}}` is 2 deep, a scalar 0. A deeper value is refused like every other value beyond a limit: `OpError::Invalid` from
  outside a turn, `TurnError::Invalid` at the call in a turn (by a typed ref and as raw JSON), and `400` over HTTP,
  before the repeated id and the schema check; a turn whose new state is too deep ends its message `failed` with the
  cause `handler`, and keeps the stored state. The error names the payload or the state and the limit: `The payload is
  nested deeper than 64 levels`.
- **BREAKING** Over HTTP a payload or a state of 65 to 126 levels is `400`, where it was checked against its schema
  (`202` or `422`).
- **BREAKING** for code that matches `RetryResult` exhaustively: an operator's retry checks the stored payload too. A
  payload deeper than 64 levels, stored before 0.16.0 or a `Failed` about a payload at the limit, is the new
  `RetryResult::TooDeep`, over HTTP `409 message-not-retryable`, and nothing is written; a bulk retry skips it and does
  not count it. Without the check a retried `Failed` would grow a level with every failure.
- The error of a new state that a check refuses (`null`, a NUL character, too deep) starts with `State cannot be
  stored:`, where it said `State cannot be encoded:`. A state that does not encode still says `State cannot be
  encoded:`.
- The check of a payload or a state finds a NUL character and measures the depth in one walk, which stops at the first
  level beyond the limit, so it refuses a value of any depth with an error. Nothing after it meets a value deeper than
  64 levels, except a `Failed` that the store writes, which holds the payload of the failed message one level down.
- Values stored before are delivered and decoded as before: only what is written is checked. A turn that writes such a
  state back fails.
- The ready-made actors do not change, and neither do their contracts. As they hold values three or four levels down,
  a letter keeps a payload of at most 61 levels (a deeper one fails the turn of the dead-letters actor that would
  record it, and no letter is recorded), a `Publish` a payload of at most 62, and a supervisor a child state of at most
  61 and a start payload of at most 60.

### Added

- Every view of a value written from this release on can be read back over HTTP: it is at most 69 levels deep (a page
  of `GET /outcomes` that holds a `Failed`), which every JSON parser that allows 128 reads. The HTTP guide says how deep
  each view is, and that System.Text.Json and Json.NET stop at 64 by default.
- The core guide names what the limit leaves to the host: a `serde_json::Value` built in code too deep for its own drop,
  serde's decoder of an untagged enum that holds itself, whose time doubles with every level of a value that does not
  decode, and the stack that the decoder of a type with large fields needs in a debug build. The operations guide names
  the memory that reading a body takes, up to about 90 times its size, and how to bound it.

## 0.15.0 — 2026-11-12

A breaking release for a few types: a message schema holds every definition it refers to, whatever its name, and a
reference to a family that holds itself accepts any value. The contracts of the types that are hit change; every other
type keeps its contract byte for byte. No schema of the store changes, and clients need no upgrade. A system without a
type that is hit can deploy it rolling: nodes of 0.14 and 0.15.0 share a database.

### Upgrading

1. After the dependency is raised to 0.15.0 and before any node is deployed, run the contract snapshot tests of every
   type (`assert_contracts_match`) against the snapshots of 0.14.0 (0.14.1 makes the same contracts). A snapshot that
   changes names a type that is hit. Without snapshots, a node of 0.15.0 that hosts such a type refuses to start
   against the registration of the release before, with a `ContractConflict`.
2. For such a type, follow "Changing a contract" in the operations guide: a versioned type name, or a stop of the world
   that stops every node that hosts it, deletes its actors and then its registration (`DELETE /types/{name}` answers
   `409` while an actor of the type exists).

### Changed

- **BREAKING** for a type that is hit: its message schemas change, so its contract hash changes, and the node follows
  "Changing a contract" in the operations guide. No other type changes. Three kinds of type are hit
  (`docs/guide/core.md`, The contract of a type):
  - A message whose schema reaches a type whose name a reference escapes, at any depth (a field of that type, or a
    field of a type `A` that holds one): a name with a space, `/`, `~`, `<`, `>`, another character outside the safe
    set of an address, or a letter that is not ASCII, from `#[schemars(rename)]`, a container `#[serde(rename)]` or the
    Rust name (`Größe`). Its message schema lacked the definition, so the door did
    not check that message type (it warned that the schema cannot be used) and `assert_message_sample` failed with
    "cannot be used". The schema now holds the definition, and the door checks the message type.
  - A family that holds itself (a message `Batch { items: Vec<Msg> }`). schemars refers to it with `"$ref": "#"`, which
    in the schema of one message type meant that message, so the door refused valid nested messages. There the
    reference now accepts any value (`true`), and the decoder checks the nested messages: at the door of a node that
    hosts the type, and in the turn on every node. The schemas of the other message types do not change.
  - A hand-written schema with a reference of several segments (`#/$defs/Line/properties/sku`): its message schema now
    keeps the definition `Line`.
- A reference to a definition is read as RFC 6901 reads a JSON Pointer in the fragment of an address: percent-decoded,
  split at `/`, with `~1` and `~0` unescaped. The name of the definition is its first segment.

### Fixed

- A newtype variant of a type whose name a reference escapes kept the node from starting ("must be an enum with
  `#[serde(tag = "type")]`"). It starts now, with its first contract: nothing of it was registered, so nothing breaks
  for it.
- A newtype variant whose definition the schema of the family does not hold, which only a hand-written schema can have,
  is an error that names the variant and the definition, instead of "must be an enum with `#[serde(tag = "type")]`".
- A newtype variant whose reference points inside a definition (`#/$defs/Line/properties/sku`, hand-written schemas
  only) is an error that names the variant and the reference. Inlining the whole definition would make the door
  stricter than the decoder; such a family did not start before 0.15.0 either.

### Added

- `sample-service/tests/contract-texts.json`: the SHA-256 of the text of each contract of the examples, the ready-made
  actors and the sample service, descriptions included. A test fails when one differs; `CICADA_UPDATE_CONTRACTS=true`
  rewrites the file.

## 0.14.1 — 2026-11-12

A release of the docs (ADR 0018, its last step): one guide per layer, references that a test keeps current, a table of
the tests behind each guarantee, and checks of the links between the docs. Nothing changes for a host: no code
behaviour, no public API, no guarantee, no wire format and no schema. Nodes of 0.14.0 and 0.14.1 share a database.

### Upgrading

Nothing to do. A host that built with warnings for a missing doc comment of its own is not affected: the lint is set in
the workspace of Cicada and reaches only its crates.

### Changed

- The README is an overview: what Cicada is, the layers with their crates, a quick start on `App`, how to use a
  release, and a table of the docs. Its text moved, by layer, into four guides in `docs/guide/`:
  - `docs/guide/core.md`, the core (`cicada`, `cicada-test`, `cicada-otel`): actor types, child actors, watches,
    control effects, opening a store and running a node with `NodeConfig`, retry, the claim limit, message persistence
    and causes, timers, observers and OpenTelemetry, testing with `TestActors`, contract snapshots and state samples,
    and the examples;
  - `docs/guide/http.md`, HTTP and the client (`cicada-axum`, `cicada-client`): mounting the routers in an axum
    service, the client, the HTTP API and the schema check;
  - `docs/guide/app.md`, the app (`cicada-app`): `App` with `serve`, `worker`, the flags and variables, the command
    line and the terminal UI, and the sample service; it links to the operations guide;
  - `docs/guide/actors.md`, the ready-made actors (`cicada-actors`): the supervisor, dead letters, the topic and
    `Schedule`.
- The table of the ADRs and the design questions moved from the README to `docs/adr/README.md`, where a change adds
  its ADR's row from now on; the design notes and the known limitations moved to `docs/architecture.md`.
- "Building and testing" in the README keeps four commands. The rest now lives only in [AGENTS.md](AGENTS.md): the
  test of `cicada-app` alone, clippy, the long randomised run (`PROPTEST_CASES`), that the PostgreSQL tests are
  skipped without `CICADA_PG_URL`, the conformance suite, and that CI builds `cicada` and `cicada-app` with each
  feature alone.
- The operations guide, the architecture, `AGENTS.md`, the rules of `openspec/config.yaml` and the proposal of
  `value-depth-limit` point into the guides instead of the README.
- The contract snapshots of the ready-made actors moved from `crates/cicada-actors/tests/contracts` to
  `docs/reference/contracts/`, and their test compares each as text, descriptions included, where it compared hashes:
  a reworded doc comment fails it too, and `CICADA_UPDATE_CONTRACTS=true` rewrites a file that differs in any way.
- `missing_docs` is an error in the workspace lints, so a public item without a doc comment fails `cargo build`, not
  only clippy in CI.
- Every spec in `openspec/specs/` has a real Purpose, so `openspec validate --specs --strict` passes.

### Added

- Every Rust block of the four guides is a doctest of `cicada-examples` (`CoreGuideDoctests`, `HttpGuideDoctests`,
  `AppGuideDoctests`, `ActorsGuideDoctests`), as the blocks of the README and the operations guide are.
- `docs/reference/cli.md`: the help of the plain `cicada` binary and of every command and subcommand, with the terminal
  UI, and of `serve` and `worker`. A unit test of `cicada-app` walks the tree of the command line, renders each help
  with a fixed name, fails when the file differs and rewrites it with `CICADA_UPDATE_CLI=true`. It runs with the
  feature `tui`: in `cargo test --workspace`, or with `cargo test -p cicada-app --features tui`.
- `docs/guarantees.md`: a row for each of G1–G6 and S1–S10 with the tests that prove it. A test of `cicada-examples`
  (`examples/tests/docs.rs`) fails when a guarantee of the architecture has no row, a row names no test, or a named
  file holds no function of that name.
- The same test file checks the relative links of every Markdown file of the repository outside `target` and
  `openspec/changes/archive`: a link to a file that does not exist, or with an anchor that names no heading of its
  file, fails it. Links to the web are not checked.

## 0.14.0 — 2026-11-12

A breaking release: the ready-made actors (ADR 0018, ADR 0028). The new crate `cicada-actors` replaces
`cicada-extras` with a supervisor, dead letters and a topic, each an ordinary actor on the public API under a fixed
type name. A `Failed` says why its message failed, and a turn can create an actor by its address. No schema of the
store changes, but the schema of `Failed` does, so this is not a rolling deploy for a system that has a type that
watches, or dead letters.

### Upgrading

1. Replace `cicada-extras` by `cicada-actors`. `Schedule` keeps its name (`cicada_actors::Schedule`). Drop
   `FailureWatcher`: a watch replaces it (step 3).
2. A type that watches: its contract changes with `Failed`. Let it handle the notifications in its mailbox (a `Failed`
   of 0.13.0 has no cause, and a watcher of 0.14.0 cannot decode it), then follow "Changing a contract" of the
   operations guide for it. A handler that builds a `Failed` by hand adds the cause.
3. Dead letters: their contract changes too, and their letters are lost.
   1. Save the letters if they matter: `cicada get actor cicada.dead-letters/_ -o json`.
   2. Stop every node. While a node of 0.13.0 runs, its `FailureWatcher` creates the actor again after it is deleted.
   3. Start one node of 0.14.0 that does not host the type (`serve --types none`). Delete the actor and then the
      registration: `cicada delete actor cicada.dead-letters/_`, `cicada delete type cicada.dead-letters`. A node that
      hosts the type refuses to start until that is done.
   4. Start the other nodes, create the actor again and set its watch on `*`
      (`cicada create watch --target '*' --watcher cicada.dead-letters/_`).
4. A supervisor and a topic are new; nothing moves. They watch, so create them only when no node of 0.13.0 is left.

The operations guide, "Upgrading", has the same steps with more detail.

### Changed (breaking)

- A new crate `cicada-actors` (layer 3) replaces `cicada-extras`. It uses only the public API of the layers below.
  `FailureWatcher`, which polled the failed outcomes on a timer, is gone: dead letters are told of a failure by a watch,
  in the transaction of the failure. `CrashLoopGuard` went with 0.11.0; the claim limit replaces it.
- A `Failed` says why its message failed: its new field `cause` (`cicada::FailureCause`, kebab-case on the wire) is
  `handler`, `node`, `refused`, `timeout`, `message`, `state`, `commit` or `claim-limit`. Every failed message has
  exactly one; the node decides it where it decides that the message fails, by the table in the architecture. A failed
  read of the store is `node`, also when a handler passes it on or wraps it, so that an outage of the store restarts no
  healthy child. The schema of `Failed` is part of the contract of every type that watches, so each such type gets a
  new contract. `TurnOutcome`, which an observer gets, does not change.
- The dead letters have a new contract and live in `cicada-actors`: the ref `DeadLetters` has a name
  (`DeadLetters::default()` is `cicada.dead-letters/_`), `RecordFailure` is gone (the actor takes `Failed` and
  `Stopped`), `Redrive` and `Discard` take `messageId`, and the state keeps the letters by the id of the failed message,
  each with the sender, the error, the cause, the attempts, the time it was recorded (`recordedAt`) and the `seq` of its
  notification, which orders letters recorded at the same time. `DeadLettersActor::with_capacity` returns a
  `CapacityError` in place of a `String`.
- The sample service hosts the three ready-made actors, creates `cicada.dead-letters/_` and its watch on every type at
  the start, and no longer reads `FAILURE_WATCHER_INTERVAL`; `Sample` has no `watcher`, and the type `Watcher` is
  gone.

### Added

- The supervisor (`SupervisorActor`, `cicada.supervisor`): it starts the children of its state, each in a turn of its
  own and tried again for 63 seconds, creates them as its own children and watches them. It restarts a child whose
  message failed with the cause `handler` or `timeout`, in one commit (delete, create, start message, `Restarted` to
  itself), once for one incident, as told by the order of its own mailbox; a restart empties the mailbox of the child.
  It gives up on a child whose state cannot be read (`state`), and does nothing for the other causes. After too many
  restarts (3 in 60 seconds by default) it gives the child up and fails a message of its own as a handler does, so that
  a supervisor above restarts it; a child it cannot start or read fails a message that is refused for good. A child
  that stops itself or that somebody deletes is forgotten. `StartChild`, `Restarted` and `Escalate` count only from
  the supervisor itself, and a `Failed` or a `Stopped` only when the store wrote it as a notification.
- Dead letters (`DeadLettersActor`, `cicada.dead-letters`): told of failed messages by a watch, with a capacity
  (1,000 by default), `Redrive` (once; it records the id of the new message, or why nothing was sent) and `Discard`.
  Its own failures are no letters, and neither is a `Failed` that is not a notification.
- The topic (`TopicActor`, `cicada.topic`): `Subscribe`, `Unsubscribe` and `Publish`; each publish is delivered to each
  subscriber in a turn of its own, with the correlation id of the publish. A subscriber that is gone fails its own
  delivery and is dropped through the topic's watch on itself, never for a `Failed` that is not a notification; a
  delivery that is refused at the call fails for good and keeps the subscriber; one whose read of the store fails is
  tried again.
- `StoredMessage`: a message an actor keeps to send later, a type name and a JSON payload, built from a typed ref
  (`StoredMessage::for_ref`), so that a message of the wrong family does not compile; `Child::start` checks its start
  message the same way. `TopicMsg::publish` takes a message of any family: a value without serde and a JSON Schema does
  not compile, and one that is not a message family is refused with `EncodeError`.
- `Turn::create_raw` and `Turn::create_child_raw`: a create by address with a JSON state, for a turn that does not
  know the type when it is compiled. A type that is not registered is `TurnError::Invalid`, a failed read of the
  registry `TurnError::Store`; a node that hosts the type refuses an address or a state it cannot read.
- The contracts of the three actors are pinned by snapshots in `crates/cicada-actors/tests/contracts`, and sample
  checks cover what their helpers and the actors write.
- ADR 0028 records the cause in a `Failed`, how a supervisor tells the lives of a child apart and how it escalates, and
  that a stored message holds no address.

## 0.13.0 — 2026-11-12

A breaking release: the app layer (ADR 0018, ADR 0027). The new crate `cicada-app` turns a set of actor types into a
binary with `serve`, `worker` and the operator commands, configured by flags and `CICADA_…` variables:
`App::new().host(actor).run()`. The crate `cicada-cli` goes into it. No schema of the store and no wire format
changes; nodes of 0.12.0 and 0.13.0 can share a database.

### Upgrading

1. A host may replace its own start-up code by `App`. Nothing forces it: the layers below are as they were.
2. Whoever installs the command line builds `cicada-app` where it built `cicada-cli`, with the feature `tui` for the
   terminal UI (`cargo install --path crates/cicada-app --features tui`). The binary is still called `cicada`.
3. A deployment of the sample service renames its variables: `PORT` becomes `CICADA_LISTEN` (and `serve` listens on
   `127.0.0.1:8080` unless it is set; the image sets `0.0.0.0:8080`), `DATABASE_URL` and `SQLITE_PATH` become
   `CICADA_STORE`, `DATABASE_SCHEMA` becomes `CICADA_STORE_SCHEMA`, `DATABASE_POOL_SIZE` becomes
   `CICADA_STORE_POOL_SIZE`, `HOSTED_TYPES` with its groups becomes `CICADA_TYPES` with type names, and `NODE_ID`,
   `CONCURRENCY`, `LEASE_TTL`, `HEARTBEAT_INTERVAL`, `POLL_INTERVAL`, `SHUTDOWN_GRACE_PERIOD` and `MAX_TURN` get the
   prefix `CICADA_`. `API_TOKEN`, `FAILURE_WATCHER_INTERVAL` and the variables of OpenTelemetry stay. For this release
   a node of the sample refuses to start while one of the old variables is set, and the error names the new one. The
   image runs `serve`.

### Changed (breaking)

- The crate `cicada-cli` goes: its command line moves into `cicada-app`, which still builds the binary `cicada` with
  the operator commands. The terminal UI is the feature `tui` of `cicada-app`, off by default: without it the binary
  has no command `tui` and builds neither `ratatui` nor `crossterm`. Whoever used `cicada-cli` as a library uses
  `cicada-app`, whose public API is `App`, `AppError`, `Env` and `node_command`.
- The help, the usage errors and the hints of the command line name the binary as it was called
  (`myapp config set-context`, not `cicada config set-context`). A command line that cannot be used exits with code 2,
  as clap's errors did, and every other error with code 1.
- The sample service runs on `App`: its variables are those of `cicada-app` (see Upgrading), it picks its types by
  their names with `--types` in place of the groups of `HOSTED_TYPES`, its `GET /` names the node and its types but no
  longer the store or the groups, and its `main` needs a command (`serve`).

### Added

- `cicada-app` (layer 2): `App::new().host(actor).run()` gives a binary with `serve` (the node, the HTTP API at the
  root and a public `/ready`), `worker` (the node, no HTTP) and the operator commands of `cicada`, which always go over
  HTTP. A binary without hosted types has neither `serve` nor `worker`, so the plain `cicada` binary never serves.
- The store from `--store` or `CICADA_STORE`: `postgres://…` or `postgresql://…`, `sqlite:FILE`, or the path of a
  SQLite file. Another value with `://`, or one that starts with `postgres:`, is refused and never printed, as it may
  hold a password. `--store-schema`, `--store-pool-size` (by default the larger of 20 and the concurrency plus 4) and
  `--retention`, each with a variable `CICADA_…`. On PostgreSQL, `serve` and `worker` wait about a minute for a store
  that answers "unavailable": a try can take seconds, so the wait can be longer, and a wrong password is tried again
  like an unreachable database. The stores are the features `sqlite` and `postgres` of `cicada-app`, none on by
  default; a store whose feature the binary was built without fails the command and names the feature. A usage error,
  of clap or of the settings, hides the password of an address it quotes, from a flag, a variable or an argument
  (`postgres://user:***@host`); it leaves alone a raw password with `/`, `#`, `?`, a quote or a space, an address
  without `//` (`postgres:u:pw@h`) and the libpq form (`host=h password=…`).
- `serve` listens on `127.0.0.1:8080` unless `--listen` (`CICADA_LISTEN`) says otherwise: `App` has no
  authentication of its own. `--types` (`CICADA_TYPES`) picks the types of a node: `all`, type names with commas, or
  for `serve` `none`. The settings of the node have a flag and a variable each (`--node-id`, `--concurrency`,
  `--lease-ttl`, `--heartbeat-interval`, `--poll-interval`, `--shutdown-grace-period`, `--max-turn`,
  `--max-lost-claims`); a flag wins over its variable, the variable over `App::node_config`, and that over the default.
  A duration is `250ms`, `5s`, `7d` or ISO 8601.
- `App::http` lets the host change the HTTP side (its layer, its routes); `/ready` is answered outside its router.
  `App::on_start` runs after the node started; its error stops the node and fails the command. `App::observe` adds an
  observer, `App::version` sets what `--version` prints (`cicada-app <version>` without it; the plain binary prints
  `cicada <version>`).
- `serve` starts in order (the store, the address, the routes, the node, `on_start`, then `Listening on
  http://<address>` on the output) and stops in good order: the node first, while the API keeps answering and `/ready`
  answers `503`, then the HTTP server, whose requests still open get 5 s before they are cut off, then the store.
  `serve` and `worker` refuse `--context`, `--server`, `--token` and `-o`, so that `serve --token` cannot look like
  protection. `run` sets up logging and watches SIGTERM and SIGINT only for `serve` and `worker`; `run_async` does the
  same on the caller's runtime, and `run_with` takes the arguments, the environment, the output and the stop from its
  caller. `node_command` says whether arguments would start a node.
- CI checks `cicada-app` with no feature and with each feature alone, runs `cargo test -p cicada-app` (without the
  terminal UI), and checks that it builds neither `ratatui` nor `crossterm` without `tui`.
- `cicada-app` depends on `http-body` (already in the tree through axum) and turns on clap's feature `string`.
- ADR 0027 records where the app layer departs from ADR 0018 and the names of its flags and variables.

### Fixed

- `cicada config` commands write through a config path that is a symlink: the file it points to gets the new text, with
  mode 0600, a relative link is read from the folder of the link, a target that is not there yet is created, and the
  link stays a link. They replaced the link with a regular file before.

## 0.12.0 — 2026-11-12

A breaking release: the schema check at the HTTP door (ADR 0018, ADR 0026). A send and a create over HTTP are checked
against the registered JSON Schema of their type on every node, whether or not the node hosts the type, so whether a
bad payload is refused no longer depends on the node a client reaches. No schema of the store changes; nodes of 0.11.0
and 0.12.0 can share a database, and during a rolling deploy a node of 0.11.0 still answers as before.

### Upgrading

1. Before the nodes are upgraded, add `cicada_axum::assert_message_sample` and `assert_state_sample` to the tests of
   each type, with what the type itself serialises and with what clients send over HTTP.
2. A sample that fails only the schema shows a type whose schema and serde disagree. When it is a form that only some
   clients send (a field under its `#[serde(alias)]`), stop sending it. When it is what the type itself writes (a field
   with `#[serde(with)]` and no `#[schemars(with)]`), the schema is wrong: making it right changes the contract of the
   type, so follow "Changing a contract" in the operations guide. There is no switch that turns the check off.
3. A client that sent such a form to a node that hosts the type, or anything that does not match its schema to a node
   that does not, now gets `422` where it got `202` (or `201` for a create). Clients need no upgrade.

### Changed (breaking)

- A payload or a state that serde accepts and the schema refuses is refused over HTTP from now on, by every node: a
  field under a `#[serde(alias)]`, a form that only a hand-written `Deserialize` takes, a payload that repeats `"type"`
  for a family that denies unknown fields (the decoder drops the key, the schema does not allow it). A node that does
  not host a type refuses what it accepted until now. A type whose schema refuses what the type itself writes gets
  `422` for every send of the typed client; it needs a right schema, which is a contract change.
- The status stays `422`, as for a payload that did not decode, where ADR 0018 said `400`: in this API `422` is what a
  client sent that the system cannot take, and `400` a request that cannot be read (ADR 0026).
- `POST /actors/{type}/{key}/messages` checks the payload against the schema of its message type, and
  `PUT /actors/{type}/{key}` the state against the state schema of the registered contract, on every node. A refusal is
  `422 invalid-payload` or `422 invalid-state` and stores nothing. Its detail names no place and quotes nothing of
  the value: `Payload of 'Add' does not match the registered schema; GET /types/counter has it`, or `State does not
  match the registered schema; …`, the same on every node. The check asks only whether the value matches, and after
  a refusal nothing else runs, so a refusal by the schema costs time and memory in proportion to the size of the
  request, whatever the type. A node that hosts the type no longer gives the decoder's reason for a payload or a state
  that the schema refuses (`Payload of 'Add' cannot be decoded: …` until now): it decodes only what the schema accepts,
  as serde's decoder is not linear for every type. The sample checks name the first place that does not match. A
  detail is cut at 2,000 characters (`MAX_REFUSAL_DETAIL`) and then ends with ` … (cut)`, as the reason of the decoder
  may quote the value; a decoder's detail had no limit before.
- Every payload and state is checked on a thread of its own with a stack of 64 MiB (address space; only the pages the
  check touches are used), so that the validator's recursion over a type that refers to itself, up to the 126 levels
  that the JSON parser of a request lets through, cannot overflow the stack of a thread of the runtime. The thread
  costs about 30 µs a check.
- A send that repeats the id of a message that is pending, or whose outcome is still shown, is answered as a repeat
  before it is checked, as before. One whose outcome its retention hides is checked first.
- Formats are not asserted (`format` is a note, as draft 2020-12 has it), the check loads nothing from outside the
  schema (no file, no network), and a schema that the validator cannot compile is not enforced: the door lets the
  request through and logs a warning once per schema. JSON that code sends (`turn.send_raw`,
  `ActorSystem::send_message`) stays unchecked.

### Added

- `ActorSystem::send_message_checked` and `ActorSystem::create_actor_checked` take a `SchemaCheck` from their caller
  (`cicada::schema::SchemaCheck`, `check(schema, value)`, true when the value matches) and run it where a hosting node
  decodes: after the repeated id, the registered type and the message type, and for a create after the `null` check and
  before the address and the decoder, so that a state that does not match gets the same answer on every node; a node
  that hosts a singleton answered `invalid-address` for a create of another name with a bad state, and answers
  `invalid-state` now. Its refusal is `SendResult::InvalidPayload` or `CreateActorResult::InvalidState` with "… does not
  match the registered schema; GET /types/… has it", and nothing else runs. `send_message` and `create_actor` are these
  with a check that accepts everything. The core has no validator.
- `cicada::runtime::MAX_REFUSAL_DETAIL` (2,000): the most characters of the detail of a refused payload or state; a
  longer one, such as a decoder's reason that quotes the value, is cut and ends with ` … (cut)`.
- `cicada_axum::assert_message_sample::<M>(&sample)` and `assert_state_sample::<S>(&sample)`: a sample must pass the
  registered schema and decode as the type, or the test fails, saying which of the two refused it and why, for the
  schema with the first place that does not match (which the door does not name; it can be slow for a deep value of a
  type with several shapes). A schema that the validator cannot use fails too.
- `cicada-axum` depends on the JSON Schema validator `jsonschema` 0.61 without its default features (draft 2020-12, set
  for the root of every schema; no format asserted; no fetching), with 26 crates new to the workspace; no other crate
  does.
- ADR 0026 records the check, the status it keeps, the check of a create and what the door does where a schema and
  serde disagree.

### Tests

- Sample checks for every message and state of the examples, the dead letters of `cicada-extras`, the types of the
  sample service, and `Failed` and `Stopped` in a watcher's family. Every one of them passes: no type of the
  repository changed its schema, so no contract changes.
- The end-to-end suite expects `422` from the send-only node for a payload that does not match, and keeps the failed
  delivery with an `i64` that only the decoder refuses.

## 0.11.0 — 2026-11-12

A breaking release: one turn observer in place of the plugin API, and the claim limit (ADR 0018, ADR 0025). An observer
watches the turns of a node start and end and has no say in them. A message counts its lost claims, and a node fails a
message whose turns keep dying instead of claiming it again for ever; the guarantee G4 names that limit. The store
schema goes to version 6 with a migration that is not additive, a message gains a field, and the view of a message
over HTTP gains `lostClaims`. Nodes of 0.10.0 and 0.11.0 must not share a database.

### Upgrading

1. Stop every node of 0.10.0. Then start the nodes of 0.11.0; the first one migrates the database. A node of 0.10.0
   that keeps running counts no lost claim, and on PostgreSQL its statements fail on the changed table until their
   connections are replaced. Clients of 0.10.0 keep working: they ignore the new field `lostClaims`.
2. Replace `install(plugin)` by `observe(observer)`. A plugin of your own becomes a `TurnObserver`: `on_claim` becomes
   `turn_started`, `on_outcome` becomes `turn_ended` of the value it returns, and what `on_handler_error` saw is the
   error of the outcome. Code that read `Turn::attributes` has no replacement, and neither has a plugin that refused
   turns in `on_claim`: an observer has no say.
3. Remove `CrashLoopGuard`. Set `NodeConfig::max_lost_claims` if 5 is not what you want. A node that had no guard now
   has a limit.
4. Code that builds a `Message` by hand adds `lost_claims: 0`; code that builds a `NodeConfig` field by field adds
   `max_lost_claims`.
5. A watcher that matches on the error of a `Failed`: the guard's text is gone; the limit's starts with `claim limit:`.

### Changed (breaking)

- `Plugin`, `Attributes`, the three hook contexts (`ClaimContext`, `HandlerErrorContext`, `OutcomeContext`),
  `HookResult`, `TurnInfo`, `ActorSystemBuilder::install` and `Turn::attributes` are removed, with the module
  `cicada::plugin`. `HandlerFailure` leaves the public API. The start problem for a plugin that is installed twice
  goes.
- `CrashLoopGuard` and `CrashLoopError` of `cicada-extras` are removed. The core has a claim limit in their place,
  `NodeConfig::max_lost_claims`, `Some(5)` by default: a message with that many lost claims ends `failed` with the
  error `claim limit: <n> lost claims`, without running its handler and without asking its retry policy, and a watch
  sees it. A claim is lost when another claim takes its place although its turn neither committed nor gave the lease
  back. `None` turns the limit off and `Some(0)` fails the start. A node that had no guard now has a limit. `NodeConfig`
  gets a public field. G4 says that a handler runs again only when its turn ended without a commit, and that a message
  whose claims were lost as often as the claim limit allows ends `failed`, whether or not its handler ever ran.
- A message counts its lost claims: `lost_claims`, a field of `Message` (code that builds one by hand adds it) and a
  column of both stores, and `lostClaims` in the view of a message over HTTP, read as 0 when a view lacks it. The claim
  that replaces a lease that ran out, which no commit, release, cancel or purge cleared, writes it. Nothing else changes
  it, and it never goes back. The wide table of `cicada get messages` has the column `LOST`, and the facts of a message
  in the terminal UI the line `Lost claims`.
- Migration 6 adds the column `messages.lost_claims`, 0 for every message that exists, so no message starts with lost
  claims from before it. It is not additive: every node of 0.10.0 is stopped before the first node of 0.11.0 starts.
- The sample service no longer reads `CRASH_LOOP_LIMIT`; it runs with the default limit.

### Added

- The module `cicada::observer`: the trait `TurnObserver`, added to a node with `ActorSystemBuilder::observe`, any
  number of times. `turn_started` is called once a message is claimed, before anything is decoded or run, with the
  actor, the message as claimed and the store time of the claim (`TurnStart`), and returns an `ObservedTurn`, whose
  `turn_ended` is told the committed outcome and the messages the commit sent (`TurnEnd`) once the commit succeeded,
  also the `failed` that the node committed in place of a rejected or refused commit. A value dropped without that call
  means that the node saw no commit of the turn succeed. `()` is an `ObservedTurn` that is told nothing; an `Arc` of an
  observer is an observer. The methods are plain functions, not async. Observers are called in the order they were
  added; a panic in one is caught and logged, and the turn goes on as if that observer were not there. A message that
  the claim limit fails is told to the observers after its commit, start and end together.
- A node gives back a claim whose answer arrives after the local deadline of its lease, without calling an observer
  or running anything, so that a slow store costs the message no lost claim. Before, the turn was dropped at once and
  its lease left to run out. A claim that arrives with less than one heartbeat of its lease left is renewed at once
  instead of one heartbeat later, after its deadline.
- ADR 0025 records the claim limit, what counts as a lost claim and why a release does not.

### Telemetry

- `ActorTelemetry` of `cicada-otel` is an observer: `ActorSystem::builder(store).observe(ActorTelemetry::global())`.
  Its spans, attributes and metrics stay.
- A turn that ends `failed` or is rescheduled now always has an `exception` event with the error, written from the
  outcome. A failure that is not the handler's (a rejected commit, a message or a state that cannot be decoded, a
  timeout, the claim limit) used to set the error status of the span without an event; now it writes the event too, so
  telemetry shows more `exception` events than before. A handler that failed in a turn whose commit then does not get
  through leaves a span that is `abandoned` and has no `exception` event; the event used to be written when the handler
  failed, before the commit.
- The span of a turn whose state cannot be decoded starts at the claim, like every other span.
- The `exception.message` of an `exception` event is the error of the outcome. For a timeout that is `timeout`; it used
  to be `timeout: the turn exceeded its maximum duration of PT…`, with the duration.

### Headers of a send

- A header that an observer adds never fails a send of the handler. One that the limits of a message refuse (a name of
  more than 200 or a value of more than 4,000 characters, or a NUL character) is left out when it is added, with a
  warning. A send takes its own headers first and then those of the observers, in the order of their names, as long as
  the message stays within 64 headers; the ones that do not fit are left out of that message, with a warning, and a
  trace breaks at that message. A header of a plugin used to count towards the limits, and over a limit the handler's
  send failed. A header that a send sets itself still keeps its value, and of two observers the one added later wins.

### Internal

- The statements of both SQL stores name the columns they read; none reads with `*`. On PostgreSQL a statement that a
  connection has prepared then keeps working when a later migration adds a column, and on SQLite sqlx no longer crashes
  its worker in that case, so such a migration can be additive again. A test per store adds a column with a default to
  every table under an open store.
- New conformance cases for the count of lost claims on all three stores, upgrade tests from schema version 5, the
  randomised comparison of the stores compares the count (155 of 400 generated lists reach such a claim), and the
  simulation of nodes under faults runs its nodes with a claim limit of 2 and checks that a message fails at the limit
  only after two lost claims and without a handler run for that claim. Its scenarios have outages, renewals that fail
  in a row on every node, so that they reach the limit.

## 0.10.0 — 2026-11-12

A breaking release: control effects (ADR 0018, ADR 0024). A turn can pause, resume, purge and delete another actor, in
the same commit as its own state and its other effects, with the meaning these actions have for an operator. To restart
an actor, a turn deletes it and creates it again. The commit gets new parts, so the guarantee G2 changes, and G1 and S7
get clearer words. No schema and no contract changes.

### Upgrading

1. Nothing to do for the store: there is no migration, and nodes of 0.9.0 and 0.10.0 can share a database. The effects
   are applied by the commit of the node that runs the turn; an actor on a node of 0.9.0 is paused or deleted as it
   would be from outside.
2. Nothing changes for a turn that does not use the new calls, and clients need no upgrade.

### Changed (breaking)

- A turn can pause, resume, purge and delete another actor, named by a typed ref or an address. Each is buffered like a
  send and applied by the turn's commit, together with the turn's other effects or not at all. G2 names the control of
  other actors among the parts of a commit that happen all together or not at all. G1 says what an instance is: one
  life of an address, from its create to its removal; a handler of an earlier life may still run for a moment after
  the actor was deleted and created again, and it commits nothing. S7 says that a commit can end the pending messages of
  another actor, by a purge or a delete. The sentence on leases is exact: a lease ends only with its turn or with its
  actor.
- A commit applies, in this order: the watches of the turn, the outcome, the cancels, the deletes, the creates, the
  purges, the pauses and resumes, the sends, the state, the release of the lease and last the notifications. Deletes
  before creates give the restart; creates before pauses let a turn create an actor that starts paused; purges before
  sends let it empty a mailbox and put a first message into it.

### Added

- `Turn::pause`, `resume`, `purge` and `delete` for a typed ref, and `pause_at`, `resume_at`, `purge_at` and
  `delete_at` for an address. Each means what the call from outside a turn means. A delete removes the actor, its
  mailbox and its descendants, voids a turn running on one of them (it commits nothing, and its message ends `purged`)
  and tells their watchers with a `Stopped` whose `by` is the turn's actor and whose `causedBy` is the turn's message:
  `deleted` for an actor the turn names, `parent` for a descendant, `stopped` for the turn's own actor when it stops in
  the same turn. No effect writes another actor's state.
- An effect on an actor that does not exist does nothing and does not fail the commit; a send to an actor that the turn
  deletes and does not create again still fails the turn's message. A restart of an actor that is not there only creates
  it and tells no watcher. A turn may control any actor, not only the ones it created.
- A turn cannot delete its own actor (`TurnError::Invalid` at the call; it stops itself) or create and then delete one
  actor (a restart is a delete, then a create); a delete of one of its ancestors fails its message and removes nothing.
  It may pause, resume and purge its own actor. Of a pause and a resume of one actor, the last call stands; the same
  effect twice is applied once.
- The node logs each control effect of a commit it saw succeed, at the info level, with the acting actor, the effect
  and the target (`Turn of … for …: delete …`), and wakes its dispatch loop after it, so that a resumed actor and
  a notified watcher are handled at once on every store. On PostgreSQL a resume by a commit wakes the nodes of the
  type.
- ADR 0024 records the rules and why a restart is not the forced release that ADR 0015 refused: the message of the turn
  that loses its actor ends `purged` and is never handled again.

### Internal

- The stores apply the effects in all three backends and pass new conformance cases, among them two races run 50
  times: two actors that restart each other's neighbours while a third sends to them, and a restart of an actor that is
  not there while another caller creates it. On PostgreSQL the controlled actors and the watchers of each deleted actor
  join the ordered lock of the commit, a delete of an ancestor is found by walking the parents up, a restart keeps the
  actor's row and resets it (so a transaction waiting for that row finds the new actor), and for an address that is
  absent the row that holds the lock is the create.
- The randomised comparison of the stores builds pauses, resumes, purges, deletes and restarts in its commits, and the
  simulation of nodes under faults has an actor that restarts another.

## 0.9.0 — 2026-11-12

A breaking release: watches (ADR 0018, ADR 0023). An actor can watch another actor, the actors of a type, or every
actor, and is told with a message in its mailbox when a message to a watched actor ends `failed` and when a watched
actor goes away, in the transaction of that event. A failed message can now cause messages, so the guarantees G2 and G4
change, the store schema goes to version 5 with a migration that is not additive, and a message gains a field. Nodes of
0.8.0 and 0.9.0 must not share a database.

### Upgrading

1. Stop every node of 0.8.0, then start the nodes of 0.9.0; the first one migrates the store to schema version 5. A
   node of 0.8.0 that opens the migrated database refuses it, but one that is still running does not notice: a message
   that fails there tells no watcher, and an actor it removes keeps its watches. Rolling back the code needs a database
   from before the migration, as for every schema version.
2. Nothing changes for an actor that nobody watches.
3. Code that builds a `Message` or a `MessageView` by hand names the new field `notification` (`false`).
4. A type that starts to watch adds the variants `Failed(cicada::Failed)` and `Stopped(cicada::Stopped)` to its message
   family, so its contract changes: upgrade the nodes that host it as the operations guide says under "Changing a
   contract". A family that has a variant `Failed` or `Stopped` of its own renames it first.
5. A host that runs `FailureWatcher` (`cicada-extras`) keeps it until `cicada-actors` replaces it with a watch.
6. `ActorClient` users need not be upgraded together with the nodes: a client of 0.8.0 ignores the new field
   `notification` of a message's view. Watches over HTTP need a client of 0.9.0.

### Changed (breaking)

- When a message to a watched actor ends `failed`, or a watched actor stops or is deleted, the same transaction puts
  one notification into the mailbox of each watcher: a `Failed` or a `Stopped`, one per watcher and actor, named after
  the id of the event (`<event>.0`, `<event>.1`, …) in the order of the actor and then of the watcher. A failed message
  can now cause messages: G2 and G4 name the notifications (they are written with their event or not at all, and each
  once), and S2 says that a failed commit, a stop and a delete write theirs with them. A watcher that does not exist,
  or that the same event removes, gets nothing; a notification never fails the commit of the watched actor, and one
  that fails tells nobody, so two actors that watch each other cannot loop.
- An appended migration (version 5) adds the table `watches (target, watcher)` with an index on `(watcher, target)` and
  the column `messages.notification`, in both stores; no existing message is a notification. A node of 0.8.0 refuses
  the migrated database, and one that keeps running would write no notifications, so every old node is stopped first.
- A message has the field `notification`, which only the store sets: `Message`, `MessageView` (over HTTP always
  written) and the envelope that an outcome keeps, where an older envelope without it reads as `false`. A sender cannot
  set it: a message of the type `Failed` that a client sends is an ordinary message, and its failure tells the
  watchers.

### Added

- `cicada::Failed`, `cicada::Stopped` and `cicada::StopCause`: the notifications, with fixed JSON Schemas that a
  snapshot test pins. A `Failed` holds the actor, the failed message's id, type, payload, correlation id and sender, the
  error and the attempts; a `Stopped` the actor, the cause (`stopped`, `deleted`, `parent`) and `by`, the actor whose
  turn removed it (`null` from outside a turn). `correlationId`, `from` and `by` are always written.
- `cicada::WatchTarget`: one actor (`type/key`), the types under a prefix that ends at a dot (`billing` matches
  `billing` and `billing.invoices`, not `billings`), or every type (`*`); a text that is none of them is refused. A
  watch on one actor tells of its failures and its going away, a watch on a type or `*` of failures only.
- `Turn::watch` and `Turn::unwatch` set and remove the watches of a turn's own actor, applied by its commit before its
  creates and sends, in the order of the calls; `watch` fails at the call (`TurnError::Invalid`) for a text that is no
  target and for a family without both notifications. A watch survives the delete and re-create of its target, and
  removing a watcher removes its watches.
- `ActorSystem::watch`, `unwatch` and `watches`; `WatchResult` names the refusals: a watcher whose type is not
  registered, that does not exist, or whose type does not declare both notifications with the core's schemas.
- `GET /watches?watcher=&target=&cursor=&limit=` on both routers, and `PUT /watches` and
  `DELETE /watches?target=&watcher=` on the full router, with the problems `404 no-such-watch` and
  `422 not-a-watcher`; `ActorClient::watches`, `watch` and `unwatch`; `cicada get watches`, `cicada create watch` and
  `cicada delete watch`, and the column `NOTIFICATION` of a message with `-o wide`; the listing `:watches` in
  `cicada tui`, where Enter on a row opens the watcher.
- A `Failed` is kept by the `failed` rule of the failed message's retention for both of its outcomes; a `Stopped` is
  caused by the message whose turn removed the actor. The runtime wakes its dispatch loop after a failure, a stop and a
  delete, and PostgreSQL announces a notification to the nodes of its watcher's type.
- ADR 0023 records the rules of a watch. The randomised comparison of the stores sets and removes watches, and the
  simulation under faults has a watcher and checks that every failure of a watched actor is told exactly once.

### Fixed

- The conformance case `s7_release_expiry_and_lost_commits_keep_the_message_pending` no longer depends on a lease of
  0.8 seconds holding on PostgreSQL: its last claim takes a long lease, as do the other cases that claimed with the
  default lease on a real-time fixture and then needed the lease to hold. It failed once in the release workflow of
  0.7.0, on a slow runner.
- The `children` tab of an actor in `cicada tui` says `1000+` when the actor has more children than it shows, as the
  mailbox fact does. The architecture says that the message of a turn that stops ends `done`, not `purged`.

## 0.8.0 — 2026-11-11

A breaking release: child actors (ADR 0019). An actor may have a parent, the actor whose turn created it with
`turn.create_child`, and deleting or stopping an actor removes every descendant with it. The guarantee S2 changes, the
store schema goes to version 4 with a migration that is not additive, and the public types of an actor gain a field.
Nodes of 0.7.0 and 0.8.0 must not share a database.

### Upgrading

1. Stop every node of 0.7.0, then start the nodes of 0.8.0; the first one migrates the store to schema version 4. A
   node of 0.7.0 that opens the migrated database refuses it, but one that is still running does not notice: it deletes
   or stops a parent without its children, which then stay behind for good. Rolling back the code needs a database from
   before the migration, as for every schema version.
2. Use `turn.create_child` where an actor should go with its creator; `turn.create` keeps creating an independent
   actor, so nothing changes for code that does not ask for it. Switch an existing type to `create_child` for new
   parents only: an actor created with `create` stays independent, and a parent that creates it again with
   `create_child` fails its message (the commit is rejected). Start its data anew, or let the parents that created
   actors the old way finish first (operations guide, Upgrading).
3. Code that builds `ActorRecord`, `ActorSnapshot` or `ActorView` by hand adds the field `parent` (`None` for an
   independent actor). Nothing else needs a code change: `create`, `delete_actor` and `stop` compile unchanged.
4. `ActorClient` users need not be upgraded together with the nodes: a client of 0.7.0 ignores the new field `parent`
   of an actor's view. Listing children needs a client of 0.8.0.

### Changed (breaking)

- Deleting an actor (`ActorSystem::delete_actor`, `DELETE /actors/{type}/{key}`, `cicada delete actor`) or stopping it
  (`turn.stop()`) also removes every descendant (its children, theirs, and so on) in the same transaction. Their
  pending messages end `purged`, a turn running on one of them commits nothing, and `DeleteResult::Deleted.purged`
  counts the messages of the whole tree. S2 says so: `delete`, and a commit that stops, remove the instance, its
  descendants and their mailboxes together. Actors created with `create` are independent and are not affected.
- The view of an actor names its parent: `ActorRecord`, `ActorSnapshot` and `ActorView` gain the field `parent`, an
  address or `None`; over HTTP `parent` is always written, `null` for an independent actor.
- An appended migration (version 4) adds the nullable column `actors.parent` and the index `actors_parent (parent,
  address)`, in both stores; every existing actor is independent. A node of 0.7.0 refuses the migrated database, and
  one that keeps running would delete a parent without its children, so every old node is stopped first.

### Added

- `Turn::create_child(&ref, &state)` creates an actor whose parent is the actor that runs the turn, with the arguments
  and checks of `Turn::create`. A child of the same parent that exists already is left untouched; an actor that exists
  with another parent or with none is never taken over: the commit is rejected and the message ends `failed` with a
  reason that names the actor. A turn that creates one address both ways gets `TurnError::Invalid` at the second call.
- `GET /actors/{type}/{key}/children?cursor=&limit=` (on both routers), `ActorSystem::list_children`,
  `ActorClient::children` and `cicada get children ADDR` list the direct children of an actor in address order, paged;
  an unknown actor is a `404 no-such-instance`. `cicada get actor -o wide` shows the column `PARENT`.
- The terminal UI shows the parent of an actor (or none), lists its direct children in a tab `children`, opens a child
  with Enter and the parent with `P`; its question before a delete says that the descendants go too.
- The `fanout` example creates its items with `create_child`, so deleting a batch removes the items that have not
  reported yet.

## 0.7.0 — 2026-11-11

A breaking release of the API: the core is tidied (ADR 0018, its fourth step; ADR 0022). An actor definition says its
message type once and has no unhandled policy, functions of the core return named errors, a start error holds a
problem per cause, and four errors say what is wrong. No schema change and no contract change: nodes of 0.6.0 and
0.7.0 can share a database (but see step 2 for a type that said `Unhandled::Drop`).

### Upgrading

1. In every actor definition, remove `type Message = …;` and `const UNHANDLED: Unhandled = …;`. The handler takes the
   message type of the ref, as before. Generic code that named `A::Message` names `<A::Ref as ActorRef>::Message`.
2. A type that said `Unhandled::Drop`: expect `failed` for a message that cannot be decoded, where it ended `done`
   before; there is no way to opt out. Nodes of 0.6.0 and 0.7.0 end such a message differently (`done` on an old node,
   `failed` on a new one), so upgrade every node that hosts such a type together.
3. Where a function's error was handled as a `String`, print the error, or keep it as the named type of its module
   (`RetryError`, `RetentionError`, `IdError`, `MessageError`, `ContractError`, `DefinitionError`, `RequestError`).
   `runtime::limits::check`, `check_envelope` and `check_json` are no longer public.
4. Match on `StartProblem` instead of comparing texts: a contract conflict is `StartProblem::ContractConflict`, with
   the type name and both hashes. `hash_conflict_reason` is gone.
5. A caller of `send_message` handles `SendResult::NoSuchType`. A client that tells `no-such-instance` from
   `no-such-type` gets the second for a send to an actor whose type is not registered; both are a `404`.
6. A node that retried its store at start stops on a migration that violates a constraint; read the reason, fix the
   data and start it again.
7. A ref whose `Serialize` writes a unit while its `Deserialize` reads a string has no address any more; make the two
   agree.

### Changed (breaking)

- `Actor::Message` is removed: the message type is declared once, on the ref (`ActorRef::Message`). A handler takes
  `<Self::Ref as ActorRef>::Message`, and so do `Turn::send_self` and `send_self_with`.
- `Unhandled` and `Actor::UNHANDLED` are removed, from the crate root and the prelude too. A message that cannot be
  decoded (a type name the family does not have, or a payload that does not fit) ends `failed` with the reason, without
  a retry, and no handler runs: what `Unhandled::Fail` did. `TurnInfo::unhandled()` stays and still gives a plugin the
  reason.
- Functions that returned a text as their error return a named error type of their module. It holds the text, prints
  it as before and implements `std::error::Error`, so `?` carries it into `anyhow` or an error of the host:
  `RetryError` (`RetryPolicy::new`, `Backoff::exponential`, `Backoff::fixed`), `RetentionError` (`Keep::keep_for`,
  `Keep::last`), `IdError` (`MessageId::new`, `NodeId::new` and their `TryFrom<String>`), `MessageError`
  (`message::encode`, `message::decode`, `FamilyInfo::of`, `schema::message_schemas`), `ContractError`
  (`Contract::from_json`, `Contract::of`), `DefinitionError` (`actor::contract_of`; it gives its reasons with
  `reasons()` and prints them joined with "; ") and `RequestError` (`BulkRequest::selection`,
  `SendRequest::delivery`).
- `runtime::limits::check`, `check_envelope` and `check_json` are no longer public; nothing outside the core called
  them. The limits themselves, the constants of that module, stay public.
- `StartError.problems` is a list of `StartProblem` (exported next to `StartError`), an enum open to new cases with
  two: `ContractConflict { actor, type_name, registered_hash, node_hash, reason }` and `Other(sentence)` for a setting
  out of range, a plugin installed twice, a malformed definition (one problem per reason), a type hosted twice, a store
  failure while registering and a node that is not new. `StartError` can still be cloned and prints what it printed
  before. `store::hash_conflict_reason` is no longer public.
- A send to an actor whose type is not registered answers `SendResult::NoSuchType`, over HTTP `404 no-such-type`, as
  a create of such an actor does; before it answered `404 no-such-instance`, "… does not exist", although the actor
  may exist. `no-such-instance` stays for a registered type that has no such actor. The check for a known id still
  runs first, so a repeated send answers as a duplicate while its message is pending or its outcome is still shown. A
  retry of a failed message is no send: it is accepted whether or not the type is registered, and waits in the mailbox.

### Fixed

- The detail of a `422 invalid-payload` says its reason once: `Payload of 'Add' cannot be decoded: …`, no longer
  `Payload of 'Add' cannot be decoded: Payload of 'Add' cannot be decoded: …`. For a payload that is not a JSON object
  it is `Payload of 'Add' must be a JSON object`, no longer with `Payload of 'Add' cannot be decoded: ` in front.
- A statement of a migration that violates a constraint of the database (SQLite `SQLITE_CONSTRAINT`, PostgreSQL
  SQLSTATE class `23`), such as two keys that become one name in migration 3, fails the open with
  `StoreError::Invalid`, which names the schema version it could not reach and holds the database's message; the
  database stays at its version. Before it was `StoreError::Unavailable`, which tells a node to try again. A lost
  connection, a locked database, a deadlock and a missing privilege stay `Unavailable`.
- A ref whose `Serialize` writes a unit while its `Deserialize` reads a string (a `Serialize` written by hand, `into` a
  unit type with `try_from = "String"`, `serialize_with` on a newtype) has no address: a send to it or a create of it
  fails at the call (`TurnError::Invalid`). Before it got the address of a singleton, `type/_`, for every value.

### Unchanged

- The text errors of `cicada-extras` (`CrashLoopGuard::new`, `DeadLettersActor::with_capacity`) and of `cicada-test`
  (`check_contracts`, `check_state_decodes`) stay as they are, and so do `AddressError`, `TimeError` and the text in
  `TurnError::Invalid`.

## 0.6.0 — 2026-11-11

A breaking release: a key is one name (ADR 0018, its third step; ADR 0021). A ref is written as one string, or is a
unit struct for a singleton; a `:` is an ordinary character of a name; contracts and the views of actors and types
hold no key fields. Every contract hash changes, and the store schema goes to version 3 with a migration that is not
additive. Nodes of 0.5.x and 0.6.0 must not share a database.

### Upgrading

1. A ref with two or more fields gets one string and joins its parts itself, with a separator its parts cannot hold. To
   keep reaching the actors that exist, join with `:` in the old order of the fields: the address `account/acme:7` of
   the old ref `{tenant: "acme", id: 7}` is the address of the name `acme:7`. A field that is not a string (a number, a
   boolean, a character, an enum variant) becomes one. A ref can also keep its typed fields and name `String` as its
   serde form (`#[serde(into = "String", try_from = "String")]`; README, Define an actor type). Such a ref is written
   differently inside states and messages, where refs travel as JSON (such as `reply_to: Order` in the saga example), so
   a state or a pending message from before no longer decodes and its turn ends `failed`. The plain way is to let its
   actors finish and drain their mailboxes with the old version. Where that is not possible, read the old form where the
   ref is held: `deserialize_with` on the field of the state or the message that holds the ref, not on the ref itself. A
   ref's one field cannot have a serde alias, and a ref cannot use a reader that takes more than one form
   (`deserialize_any`, `from` an untagged enum): the start check counts what the reader declares. A ref that keeps its
   one string field is not touched.
2. In the code, replace `ActorAddress::of(type, values)` with `ActorAddress::of(type, name)`, which takes one name,
   and `key_values()` with `name()`. Drop `to_params`, `from_params`, `RefBinding::key_fields`, the key argument of
   `Contract::new`, `Contract::key_fields`, `Contract::params_of`, `TypeView.key`, `ActorView.params` and
   `ActorSnapshot.params`. Contract snapshot files of an earlier version keep passing: a `key` entry is ignored.
   Then check every ref before the database is migrated: a ref that is not one string stops a node at start, which
   is after its store has migrated the database. Run the tests (a test on `TestActors` hosts the types and runs the
   same check), or start a node on a new database.
3. Upgrade every user of `cicada-client` together with the nodes: a client of 0.5.x cannot decode the view of an actor
   or a type from a node of 0.6.0, because its types require `params` and `key`.
4. With the old version, find the keys of two fields that become one name, such as (`a:b`, `c`) and (`a`, `b:c`),
   which both become `a:b:c`. This query lists them, on SQLite and on PostgreSQL (in the store's schema):

   ```sql
   SELECT replace(address, '%3A', ':') AS address, count(*) AS actors FROM actors
       GROUP BY replace(address, '%3A', ':') HAVING count(*) > 1;
   ```

   Delete one actor of each pair with the old version (`DELETE /actors/{type}/{key}`); an actor cannot be renamed.
   Otherwise the migration fails on the primary key of `actors` with the database's own error, and the database stays
   at version 2. Both stores report it as unavailable (`StoreError::Unavailable`). On SQLite the open fails and the
   node does not start; the sample node exits. On PostgreSQL a node that retries an unavailable store keeps retrying:
   the sample node logs "PostgreSQL unavailable" every second until its start timeout of 60 s. Outcomes are not
   checked: those of two actors deleted before merge under one address.
5. Stop every node of the old version: a node of 0.5.x does not open a database of version 3, and a running one
   refuses the contracts that the new nodes register.
6. Start the new version: the nodes that host a type before the nodes whose turns create actors of it, and nodes that
   only send last. The first node migrates the schema to version 3; each node registers its types. Migration 3
   removes every registration, so until a node that hosts a type starts, that type is not registered: its actors can
   be read and deleted, and messages already in a mailbox wait; a create of one and a send to one from outside a turn
   are refused (over HTTP `404`), and a turn on another node that creates one ends `failed` without a retry, and a
   raw send to one in a turn is refused. A host whose nodes all host the same types has no such window. Where two
   kinds of nodes create actors of each other's types, no order is safe: hold back the messages that start such turns
   until every node is up.

Or start on a new database; then steps 1 to 3 are left. There is no way back: 0.5.x does not open a migrated
database.

A listing prefix that holds the encoded colon itself stops matching, because the stored spelling of a colon is `:`
now: `x%3A` given to `ActorSystem::list_actors` (or another listing) in code, or `%253A` in a URL. Over HTTP a query
value is URL-decoded first, so `?prefix=x%3A` arrives as `x:` and still matches. Write `:`.

### Changed (breaking)

- A ref holds one name. What counts is what serde does with it: a struct of which serde writes one field, whose value is
  a string (a `String`, a newtype around one, a `Uuid`; a field with `#[serde(skip)]` does not count, so such a
  `PhantomData` is fine); a type that serde writes as a string itself (a newtype around a `String`, or one that names
  `String` as its serde form); or a unit struct or a struct that declares no field to serde, a singleton with the key
  `_`, as before. A ref that declares a field and writes none (`skip_serializing`, or `skip_serializing_if` when it
  skips) has no address. Only a string counts: a number, a boolean, a single character and an enum variant are refused,
  although the last two worked as a key field before. A ref's one field cannot have a serde alias, and a ref cannot use
  a reader that takes more than one form (`deserialize_any`, `from` an untagged enum): the start check counts what the
  reader declares. A node that hosts a type whose ref is not one string does not start, and the error names the ref and
  what its `Deserialize` asks for. `ActorRef::address` fails for a ref that serde does not write as one string, so a
  send to it or a create of it fails at the call (`TurnError::Invalid`) and the turn can go on.
- A name may hold any text and must not be empty. In an address it is percent-encoded as before, except that `:` is
  now written as it is: `percent::encode` keeps `A-Z a-z 0-9 . _ ~ -` and `:`. A `:` separates nothing, so an old
  address such as `account/acme:7` stays valid as the one name `acme:7`, and `ActorAddress::parse` reads `a%3Ab` as
  `a:b`. The keys `a:`, `:a` and `a::b` are valid now.
- `ActorAddress::of(type_name, name)` takes one name; the singleton is the name `ActorAddress::SINGLETON_KEY`.
  `ActorAddress::name()` gives the decoded name and replaces `key_values()`.
- `to_params`, `from_params` and the serde bridge behind them (`address/params.rs`) are removed; a small reader takes
  the one string out of a ref and puts it back. `RefBinding` holds no list of fields: `key_fields()` is removed, and
  `RefBinding::new` checks the ref by what its `Deserialize` asks for.
- A contract no longer lists key fields: not in `Contract::new`, not in its JSON form (`type`, `state` and
  `messages`), not in its hash. Every contract hash changes; renaming the one field of a ref keeps the hash.
  `Contract::key_fields` and `Contract::params_of` are removed. `Contract::from_json` ignores a `key` entry, so a
  contract of an earlier version reads as the same contract.
- `TypeView.key` (`GET /types`, `GET /types/{name}`), `ActorView.params` (`GET /actors/{type}/{key}`) and
  `ActorSnapshot.params` leave the wire format and the API; `docs/openapi.json` follows. The command line loses the
  `KEY` column of `cicada get types`, and the terminal UI the `Key` facts of an actor and a type.
- A node that hosts a type no longer refuses an address because it is not the spelling its ref writes (`x/007` for an
  integer): there is one spelling per name. It still decodes the address as its ref and refuses one the ref does not
  accept (`422 invalid-address`): a singleton accepts only `_`, and a ref with a reader of its own may refuse a name.
  A typed send to a turn's own ref goes to the address that ref writes, no longer always to the claimed address.
- Schema version 3 (both stores): migration 3 removes every registration and rewrites `%3A` to `:` in
  `actors.address`, `messages.actor_address` and `outcomes.actor_address`, only in rows that hold `%3A`. The sender
  of a message and the addresses inside a stored envelope are read through `ActorAddress::parse`, which gives the new
  spelling. It is not additive (see Upgrading).

## 0.5.1 — 2026-11-11

No change to the wire format or the store schema: nodes of 0.5.0 and 0.5.1 can share a database.

### Fixed

- A store on PostgreSQL opens over TLS. The feature `postgres` builds the driver with TLS: rustls on the crypto
  provider `aws-lc-rs`, the one of `cicada-client`, trusting the built-in list of public CAs. A URL with
  `sslmode=require`, `verify-ca` or `verify-full` no longer fails with "SQLx was built without TLS support enabled";
  `sslrootcert` names the file of a private or cloud-provider CA, and `sslcert` and `sslkey` those of a client
  certificate. Without `sslmode` in the URL the driver takes `PGSSLMODE`, and without that the default `prefer`,
  which now means TLS without checking the server's certificate when the server offers TLS, and plain text when it
  does not; once the server has offered TLS, a failed handshake fails the connection, and `sslmode=disable` is the
  way back to plain text. Set the mode on purpose (docs/operations.md, TLS for PostgreSQL). A host that adds a
  library that builds rustls on `ring` must install a default crypto provider at start. A host with the feature
  `postgres` now compiles a C library (`aws-lc-sys`), so its build needs a C compiler. The feature `sqlite` builds no
  TLS.

## 0.5.0 — 2026-11-11

A breaking release: the store trait leaves the public API (ADR 0018, its second step). A host opens one public type,
`Store`, and the two SQL stores move into `cicada`, each behind a cargo feature. Nothing changes in the database: no
migration, and nodes of 0.4.0 and 0.5.0 can share one.

### Upgrading

1. `Cargo.toml`: drop `cicada-store-sqlite` or `cicada-store-postgres`, and name the feature on `cicada`:
   `cicada = { git = "…", tag = "v0.5.0", features = ["sqlite"] }` (or `["postgres"]`, or both). `cicada-test` stays a
   dev-dependency.
2. Open the store with `Store::sqlite(path, options)` or `Store::postgres(url, options)` (`cicada::Store`), and pass it
   to `ActorSystem::builder`. Replace `Arc<dyn Store>` with `Store`, and `PostgresStoreConfig` with `PostgresOptions`
   (`cicada::store::PostgresOptions`; `SqliteOptions` is `cicada::store::SqliteOptions`).
3. Replace calls on the store with calls on the system: `store.now()` with `system.now()`, `store.next_due_at(&types)`
   with `system.next_due_at()` (it takes no types: it answers for the types the node hosts, and `None` when it hosts
   none), and `store.running_turn(…)` and `store.pending(…)` with the methods of the same name. What differs: every
   call on the system returns `OpError`, not `StoreError` (a store failure is `OpError::Store`), and `system.pending`
   answers an `Option` of a page, `None` when the actor does not exist, where the store answered an empty page.
   `system.now()`, `system.next_due_at()` and `system.running_turn(…)` answer the same values as the store did.
   `hash_conflict_reason` is now `cicada::store::hash_conflict_reason`.
4. Replace `system.store().close()` with `store.close()` on the store the host opened.
5. In tests: `TestActors` is unchanged. Replace `InMemoryStore::default()`, `with_clock(…)` and `new(…)` with
   `Store::memory()`, `Store::memory_on(…)` and `Store::memory_with(…)`; read a state with `system.get_actor(…)`
   instead of `store.get(…)`; replace a `store.claim_due(…)` that plays a crashed node with `claim_and_abandon`.

### Changed (breaking)

- The `Store` trait is private to `cicada` (named `Backend` inside it), and so are the types only a backend needs:
  `CommitRequest`, `CommitResult`, `Claim`, `ActorCreate`, `StateChange`, `OutgoingMessage`, `NodeRecord`, the results
  of store-only calls (`CreateResult`, `EnqueueResult`, `RegisterResult`, `RenewResult`), the schema modules of the
  stores (`schema::VERSION`), `is_listening` and `cicada::store::support`. The compiler enforces it: code outside the
  core can neither implement a store nor call one.
- `Store` (`cicada::Store`, also `cicada::store::Store`) is a public type that a host cannot look into: it holds one
  backend and is cheap to clone, and its clones share the backend. `Store::sqlite(path, options)` (feature `sqlite`)
  and `Store::postgres(url, options)` (feature `postgres`) open one; `close()` releases it for every clone.
  `ActorSystem::builder` takes a `Store`.
- `ActorSystem::store()` is removed. The host closes the store it opened; after `close` every operation fails as
  unavailable and HTTP answers `503 store-unavailable`.
- The SQLite and PostgreSQL stores live in `cicada`, behind the features `sqlite` and `postgres`. No feature is on by
  default: without one the crate builds and opens no store. The crates `cicada-store-sqlite` and
  `cicada-store-postgres` are removed, and with them the names `SqliteStore` and `PostgresStore`.
- PostgreSQL is opened from a URL only: `connect_with` and the re-exported sqlx types (`PgConnectOptions`,
  `PgSslMode`) are removed, so no type of the SQL driver is in the public API. `PostgresStoreConfig` is renamed
  `PostgresOptions` and keeps its fields.
- The in-memory store, the test clocks and `TestActors` live in `cicada::testing`, behind the feature `testing`, which
  `cicada-test` turns on. `cicada-test` exports them under the names they had, so a test on `TestActors` changes no
  import. The name `InMemoryStore` is removed: `Store::memory()`, `Store::memory_on(clock)` and
  `Store::memory_with(clock, retention)` open the in-memory store. `TestActors::store()` returns the `Store`.
- The crates `cicada-store-conformance` and `cicada-fuzz` are removed. The conformance suite, the randomised tests and
  the runtime tests that need the store trait are test code of `cicada`: `cargo test -p cicada` runs the conformance
  cases for the in-memory store and SQLite, and for PostgreSQL when `CICADA_PG_URL` is set. A long randomised run is
  `PROPTEST_CASES=100000 cargo test -p cicada --lib tests::fuzz`.

### Added

- `ActorSystem::now()`, the store's time, and `ActorSystem::next_due_at()`, the earliest time at which a message of a
  type the node hosts may become claimable (paused actors left out).
- `claim_and_abandon(store, types, lease, limit)` (`cicada::testing`, exported by `cicada-test`) plays a node that claims
  a turn and dies: it claims what is due as a node of its own, commits nothing and returns an `AbandonedClaim` per
  claim, so the message is claimed again when the lease runs out.
- `cicada::store::hash_conflict_reason`, the one function of the old `store::support` that stays public.
- A conformance case pins that every operation of a closed store fails as unavailable, on every backend.

## 0.4.0 — 2026-11-11

A breaking release: every actor has state (ADR 0018, its first step). The store schema goes to version 2 with a
migration that is not additive, and the contract of every type that was stateless gets a new hash. Nodes of 0.3.x and
0.4.0 must not share a database.

### Upgrading

1. In the code, replace `Stateless` with `NoState` and `create_stateless(&r)` with `create(&r, &NoState {})` (on
   `Turn`, `ActorSystem`, `ActorClient` and `TestActors`), and give every create over HTTP a `state`:
   `{"state": {}}` for a `NoState` type, `cicada create actor ADDR --state '{}'` on the command line.
2. Delete the contract snapshot files of the formerly stateless types; the next test run writes them again. A snapshot
   with `"state": null` is no longer a contract, so `assert_contracts_match` fails on it, also with
   `CICADA_UPDATE_CONTRACTS=true`.
3. A type whose state encodes to `null` (`()`, a unit struct, a top-level `Option`) cannot be upgraded in place:
   change its state to a struct and, with the old version, reset the type (delete its actors, then
   `DELETE /types/{name}`), as for any contract change.
4. Stop every node of the old version: an old node that writes an actor without a state fails on the new schema.
5. Start the new version, the nodes that host the formerly stateless types first. The first node migrates the schema
   to version 2, and each node registers its types. Until a node that hosts a formerly stateless type starts, that
   type is not registered: its actors can be read and deleted, and messages already in a mailbox wait; a create of
   one and a send to one from outside a turn are refused (over HTTP `404`), and a turn on another node that creates
   one ends `failed` without a retry.

Or start on a new database; then steps 1, 2 and 4 are left, and of step 3 only the change of the state to a struct
(no reset). There is no way back: 0.3.x does not open a migrated database.

### Changed (breaking)

- `Stateless`, the `ActorState` trait and `create_stateless` (on `Turn`, `ActorSystem`, `ActorClient` and
  `TestActors`) are removed. `Actor::State` is any `Serialize + DeserializeOwned + JsonSchema + Send + Sync` type.
- `NoState` (`cicada::NoState`, also in the prelude) is the state of a type that keeps no data: an empty struct, stored
  as `{}`. It refuses unknown fields, so a node that hosts the type refuses a state such as `{"oops": 1}`.
- State is never absent. In the store the column is `NOT NULL`; `Claim.state`, `ActorRecord.state`,
  `ActorCreate.state` and `StateChange::NewState` hold a `Value`, and `Store::create` takes one. A contract always has
  a state schema: `Contract::new` and `Contract::state_schema` use a `Value`, and `Contract::from_json` refuses a
  contract whose `state` is absent or `null`. On the wire `ActorView.state`, `TypeView.state` and
  `CreateActorRequest.state` are a `Value` and always written; `ActorSnapshot.state` is a `Value` too, and
  `CreateActorRequest` no longer implements `Default`.
- Schema version 2 (both stores): migration 2 gives every actor without a state, or with the state `null`, the state
  `{}`, makes the column `NOT NULL` and removes the registration of every type without a state schema, so that a node
  that hosts such a type registers it again at start. It is not additive (see Upgrading).
- `PUT /actors/{type}/{key}` needs a state: an empty body or a body without `state` is a `400`.
  `ActorSystem::create_actor` and `ActorClient::create_actor` take a `Value`. `cicada create actor` needs `--state`.
- A state that is JSON `null` is refused everywhere: `Turn::create` returns `TurnError::Invalid`,
  `ActorSystem::create_actor` returns `CreateActorResult::InvalidState` on every node (`422 invalid-state` over HTTP),
  and a turn that ends with a `null` state ends its message `failed`, without a retry, and keeps the stored state.
- The contract hash of every type that was stateless changes, because its contract gains a state schema.
- A turn of a `NoState` type that takes `state_mut()` or calls `set_state` writes the state and increases the version,
  as for any other type.
- The command line's types table drops the column that said whether a type has state, and the terminal UI drops the
  "Stateful" fact of a type. `describe_changes` (`cicada-test`) reports any change of the state schema as "state
  schema changed".

## 0.3.2 — 2026-11-11

No library crate changed, only the test crate `cicada-fuzz`: nodes of 0.3.1 and 0.3.2 can share a database.

### Fixed

- The backend comparison of `cicada-fuzz` no longer depends on the order in which a store hands out epochs to
  different instances. When two instances are due at the same instant, `InMemoryStore` and `SqliteStore` may give
  out their epochs in another order (S4 promises an order per address only), and a stale or future epoch
  (`epoch_delta`) then reached different claims in the two. It now moves by rank among the epochs handed to the same
  address. The list that failed a CI run of 0.3.1 is now a test of its own.

## 0.3.1 — 2026-11-11

No change to the wire format or the store schema: nodes of 0.3.0 and 0.3.1 can share a database.

### Fixed

- A duration whose parts sum out of range is a parse error, also over HTTP (the `delay` of a send, a retry or
  retention policy). Before, a debug build panicked and a release build let the sum wrap, so a crafted duration could
  be accepted as a small one.

### Added

- `cicada-fuzz`, a test-only crate of randomised tests: the parsers, a comparison of the store backends, and a node
  simulation under injected store faults. A short run is part of `cargo test --workspace`;
  `PROPTEST_CASES=100000 cargo test -p cicada-fuzz` is a long one. For repeatable runs, the three `tokio::select!`
  calls of the runtime are now `biased`.

- `cicada-cli`, the binary `cicada`: a command line and terminal UI for any node that serves `actor_api()`
  (`actor_api_read()` for the commands that only read). It talks to the node only over HTTP, through `cicada-client`.
  - kubectl-style commands, verb first: `get`, `describe`, `create`, `delete`, `send`, `cancel`, `retry`,
    `deliver-now`, `pause`, `resume`, `purge`, `stats`, `health`. Output as a table, `-o wide`, `-o json` or
    `-o yaml`; `--all` follows the cursor to the last page. A missing item or an error response is exit code 1.
  - Named contexts in `~/.config/cicada/config.toml` (or `$XDG_CONFIG_HOME`, or `$CICADA_CONFIG`), each with a server,
    an optional bearer token and headers; `cicada config get-contexts`, `current-context`, `use-context` and
    `set-context`. `--server`, `--token` and `--context` win over `CICADA_SERVER`, `CICADA_TOKEN` and
    `CICADA_CONTEXT`, which win over the file. On Unix the file has mode 0600 after every write.
  - `cicada tui`: a k9s-style terminal UI. Actors, pending messages, outcomes, running turns, nodes and types,
    refreshed every 2 s; overviews of an actor, message, node or type; a live filter, sorting and the mouse; and
    delete, cancel, retry, deliver-now, pause, resume, purge and stop-turn behind a confirmation.

## 0.3.0 — 2026-11-09

A breaking release (ADR 0017). Every contract hash changes, so nodes of 0.2.0 and 0.3.0 must not share a database: a
0.3.0 node refuses to start on a store where a 0.2.0 node registered its types.

### Changed

- A contract hash leaves out the `description` keywords of the state and message schemas, at any depth. A reworded doc
  comment no longer changes the hash or stops nodes from starting. A property, definition or message type named
  `description` still counts. The published contract (`Contract::to_json`, `GET /types`) keeps its descriptions;
  after a doc comment change it keeps those of the first registration.
- `SqliteStore::open` refuses a database that has no schema version but holds tables, with `StoreError::Invalid` naming
  the path, and leaves the file as it was. A new or empty file is still set up. WAL mode is now set after the
  migration.

## 0.2.0 — 2026-11-09

A breaking release that makes the library smaller (ADR 0011, ADR 0012, ADR 0013) and adds management endpoints (ADR 0014). Nodes of 0.1.0 and 0.2.0 must not share a
database.

### Added

- `actor_api_read`: only the routes that read. Both routers serve `GET /openapi.json`, an OpenAPI 3.1 document, with a
  copy in `docs/openapi.json`. `GET /types/{name}` and `ActorClient::get_type`.
- `ClaimContext::send_headers`: a plugin's `on_claim` can add headers to every message the turn sends; `ActorTelemetry`
  uses it, so trace context reaches the messages a turn sends without code in the handler.

- Management reads and writes over HTTP (ADR 0014): `GET /outcomes?status=&prefix=&since=` (finished messages of any
  status), `GET /stats?prefix=` (`StatsView`: actors, pending messages, oldest due), `GET /messages/{id}/caused` (what
  a message caused, one level down) and `DELETE /types/{name}` (problem type `type-in-use`). In code:
  `Store::outcomes`, `Store::stats`, `Store::caused_by`, `Store::remove_type` (`StoreStats`, `RemoveTypeResult`),
  and `ActorSystem` and `ActorClient` methods of the same names. The schema keeps `caused_by` in a column of
  `outcomes` and indexes it, with `messages.caused_by`.

- Operator actions (ADR 0015). Nodes: `nodes` table written by a 10 s heartbeat of every node, `GET /nodes`,
  `GET /nodes/{node}` (problem type `no-such-node`); `Store::heartbeat_node`, `list_nodes`, `remove_node`;
  `ActorSystem::nodes` and `node`. Running turns: `GET /turns`, `GET /actors/{type}/{key}/turn` (problem type
  `no-running-turn`) and `POST /actors/{type}/{key}/turn/cancel`: the owning node stops the turn at its next heartbeat
  and commits the new `TurnOutcome::Cancelled` (`Store::running_turns`, `running_turn`, `request_cancel`,
  `RenewResult::CancelRequested`, `actors.cancel_epoch`). Mailboxes: `GET /actors/{type}/{key}/mailbox`, with
  `state` per message; `POST .../mailbox/pause`, `/resume` and `/purge` (`Store::set_paused`, `purge`,
  `actors.paused`, `ActorView::paused`). Messages: `POST /messages/{id}/retry` (problem type `message-not-retryable`),
  `POST /messages/{id}/deliver-now` (`Store::deliver_now`), `POST /messages/bulk-cancel` and `/bulk-retry`. Schema
  version 1 gets the table, columns and the partial indexes `actors_due` (now also without paused actors) and
  `actors_running`.

### Changed

- `Store::create` answers `CreateResult::NoSuchType` when the type is not registered, and a commit whose creates name
  an unregistered type is `CommitResult::Rejected`: the store refuses atomically. `ActorSystem::create_actor` no longer
  looks the type up again after the insert. Store implementations and fixtures must register a type before creating an
  instance of it.
- `POST /messages/{id}/cancel` replaces `DELETE /messages/{id}` (same semantics); `GET /actors/{type}/{key}/mailbox`
  replaces `GET /actors/{type}/{key}/messages`; `POST .../messages` stays as send. `ActorClient::pending` is now
  `ActorClient::mailbox`.

### Removed

- `GET /failures`, `Store::failures`, `ActorSystem::failures` and `ActorClient::failures`: use `outcomes` with
  `OutcomeStatus::Failed`.
- The crates `cicada-ui` (the console) and `cicada-docker` (container handles). Their last version is commit `7d43cf0`.
- `Unhandled::Custom`, `Actor::on_unhandled` and `UnhandledDecision`. Use `Unhandled::Fail` or `Unhandled::Drop`.
- Plugins: the `before_commit` hook, `HookSend` and sends from hooks, `HandlerErrorContext::set_outcome`, `outcome`
  and `failure_reason`, reserved `cicada-` headers (`SendResult::InvalidHeaders`, problem type `invalid-headers`), and
  plugins on one type (`ActorType::install`). `Attributes::remove`, `with_mut` and `contains`.
- `ActorSystemBuilder`: `install_shared` and the setters `node_id`, `lease_ttl`, `heartbeat_interval`,
  `poll_interval`, `default_max_turn`, `concurrency`, `shutdown_grace_period`, `shutdown_cancel_timeout` and
  `sweep_interval`. `ActorType::max_turn` and `ActorType::install_shared`.
- Message chains: `GET /messages/{id}/chain`, `ActorSystem::chain`, `ActorClient::chain`, `MessageChain`,
  `MessageChainView`, `ChainNodeView`. `causedBy` stays on every message.
- `GET /types?uri=`, `ActorSystem::type_of`, `ActorClient::type_of`, `TypeMatchView`, `ResolvedActorType`.
- `Store`: `retention()`, `type_def()`, `outcome()`, `resolve()`, `chain_entries()`, `chain_children()`; `ChainEntry`;
  `Page::map`.
- `UriTemplate`, overlap checks between types, `support::resolve`, `ResolvedType`, `ActorAddress::to_http_path` and
  `from_http_path`, `ActorSystem::hosted_templates` (now `hosted_types`).
- `PostgresConnection` and `PostgresStore::connect_as`. Store settings: `transaction_retries`, `connection_timeout`,
  `channel`, `min_idle`, `listener_reconnect_delay` (PostgreSQL) and `readers` (SQLite).
- `cicada-extras`: `Timer`, `ActorOperations`, `Failure`, `Sent`, `OperationError` and the `client` feature.
- `api::Params`, `From<MessageView> for Message`, `ActorSystem::wake_up`, `ActorTelemetry::current_trace_headers`
  (the free function `cicada_otel::current_trace_headers` stays), `cicada_otel::trace_headers` (now automatic),
  `time::SignedNanos`, `time::serde_signed_duration`.
- `sample-service`: `/console/`, `CONTAINERS`, `DATABASE_USER` and `DATABASE_PASSWORD`.

### Changed

- Actors are addressed by type name and key instead of URIs (ADR 0013): `ActorRef` has `const TYPE: &'static str = "sync"`
  instead of `const TEMPLATE`, and the address is `sync/acme`. Several ref fields form the key `b1:7`; a ref without
  fields is the singleton `pricing/_`. `TypeName` replaces `UriTemplate`.
- HTTP paths are `/actors/{type}/{key}` and `/actors/{type}/{key}/messages`. `ActorView` has `address` and `type`, and
  `TypeView` has `name` and `key`. `prefix` is a prefix of the address, without a scheme.
- `Store::create` has no template argument; `ActorRecord`, `Claim` and `ActorCreate` have no template field, and
  `TypeDef.template` is `TypeDef.name`. Every contract hash changes.
- Plugin hooks take a shared context: `on_claim(&self, &ClaimContext)`, `on_handler_error(&self,
  &HandlerErrorContext)`. An `Arc` of a plugin is a plugin.
- Node settings: `ActorSystem::builder(store).config(NodeConfig { lease_ttl, ..NodeConfig::default() })`.
- Durations on the wire are ISO-8601 strings only (`"PT5S"`); `"5s"` and numbers of milliseconds are refused, and so
  is a negative duration. `time::parse_duration` returns a `Duration`.
- Retry and retention policies are decoded strictly everywhere; error texts changed.
- `ActorView::params` is a `BTreeMap`; its JSON keys are in alphabetical order.
- Length limits count characters; canonical JSON sorts keys by bytes.
- `FailureWatcher` reads `outcomes` with the status `failed`.
- `FailureWatcher::new` takes an `ActorSystem`. Dead-letter message ids are `dead-letter-<sha256>`.
- Error texts name hooks as `on_claim`, `on_handler_error`, `on_outcome`.
- The HTTP API has twelve endpoints.

### How to move

- A timer: keep the id that `turn.send_self_with(..)` returns in state and cancel it with `turn.cancel(&id)`
  (README, Extras).
- A plugin that sent messages or changed an outcome: do it in the handler.
- PostgreSQL credentials: put them in the URL, or build `PgConnectOptions` and call `connect_with`.

## 0.1.0

The first version: the port of the Kotlin library (ADR 0001 to 0009).
