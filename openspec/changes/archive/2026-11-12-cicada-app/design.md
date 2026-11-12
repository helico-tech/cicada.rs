# Design

Described on 2026-11-12 (change loop, Describe), while `json-schema-at-the-http-door` is built. The owner handed the
decisions of this change to the main session on 2026-11-11 ("Dor change 5 until 9 i fully trust you"). Every
decision below is made by the main session; each gives the options and why. Reviewed five times; after the third
it was written out again in one piece. The fifth said "ready for tasks after small text fixes". "Changed by the
reviews" at the end lists what the reviews changed.

## Context

What the code does today (release 0.11.0):

- The sample service is the only runnable node, and its host code is what every host repeats. It reads 15
  environment variables (`PORT`, `NODE_ID`, `HOSTED_TYPES`, `DATABASE_URL`, `DATABASE_SCHEMA`, `SQLITE_PATH`,
  `API_TOKEN`, five timings, `CONCURRENCY`, …), sets up logging, opens a store (and waits up to 60 s for
  PostgreSQL), builds the node, creates its singleton actors after the start, mounts `actor_api` behind an optional
  bearer token next to a public `/ready`, serves on `0.0.0.0:$PORT`, and stops on SIGTERM or SIGINT.
- `HOSTED_TYPES` selects groups of the sample's types; `none` gives a node that serves HTTP and hosts nothing. The
  end-to-end suite runs nodes with `all` and with `none`.
- The sample's actors for the end-to-end tests get the node id when they are built, before the node exists. Its
  `GET /` names the node, its groups, its types and its store. It starts a `FailureWatcher` task and aborts it
  when it stops; `cicada-actors` removes that watcher.
- The image of the sample service sets `PORT`, `SQLITE_PATH` and `SHUTDOWN_GRACE_PERIOD`, checks its health on
  `${PORT}`, and starts the binary without a command. The end-to-end suite also starts single nodes with
  `docker run`, without the compose file. The tests of the sample bind port 0 and read the port from the listener,
  and spawn the serving future.
- The operations guide and the README stop a serving node inside the graceful shutdown of the HTTP server: the
  node stops first, the API keeps answering while turns finish, and `/ready` answers `503` from the moment the stop
  begins. A stop of a node can take its grace period and a little more.
- The operator tool is the crate `cicada-cli` with the binary `cicada`: commands in the style of kubectl, the
  terminal UI, and a config file with contexts. It talks to a node only over HTTP, and finds its server from
  `--server`, `CICADA_SERVER` or a context; a token from `--token`, `CICADA_TOKEN` or the context. These flags and
  `-o` are global: every command accepts them. The help and the hints print the name `cicada`.
- `cicada config` saves by renaming a new file over the path. When the path is a symlink, that replaces the link
  with a regular file and leaves the link's target with the old text.
- A node takes observers with `ActorSystemBuilder::observe`, and its limit of lost claims and its timings in
  `NodeConfig`. A store has options beyond its address: the retention of outcomes (S10, 7 days), and time-outs.
- A store is opened with `Store::sqlite(path, options)` or `Store::postgres(url, options)`, each behind a feature
  of the core. A build of the workspace turns on every feature of the core, because some member asks for each; CI
  and `AGENTS.md` add `cargo check -p cicada` with no feature and with each alone.
- Chorus and Karakuri do not use `cicada-cli` as a library.

ADR 0018: "`App::new().host(actor).run()` gives a binary with `serve` (worker and HTTP), `worker` (a subset of
types, no HTTP) and the operator commands. The store and the listen address come from flags or environment
variables. `App` only composes layers 0 and 1; a host that needs more uses those directly. The operator commands
always go over HTTP. The unfinished `cicada-cli` becomes this crate's command line; the terminal UI is a feature
that is off by default."

## Goals / Non-Goals

**Goals**

- A host's `main` is a few lines: its actors, and `run()`.
- The sample service runs on `App`, with everything the end-to-end suite needs from it.
- One command line, in the host's binary and in the plain `cicada` binary.

**Non-Goals**

- TLS, and an authentication scheme of its own. The host adds its own layer.
- A config file for the server.
- OpenTelemetry in this crate. A host adds `cicada-otel` as an observer.
- Packaging and publishing binaries.

## Decisions

### 1. `App`: what a host writes

```rust
fn main() -> std::process::ExitCode {
    cicada_app::App::new().host(CounterActor).host(InvoiceActor).run()
}
```

The public API of the crate:

```rust
pub type Env = BTreeMap<String, String>;

impl App {
    pub fn new() -> Self;
    /// Adds a type, as `ActorSystemBuilder::host`.
    pub fn host<A: Actor>(self, actor: impl Into<ActorType<A>>) -> Self;
    /// Adds an observer of turns.
    pub fn observe(self, observer: impl TurnObserver) -> Self;
    /// The host's `NodeConfig`; flags and variables override it (decision 4).
    pub fn node_config(self, config: NodeConfig) -> Self;
    /// Lets the host change the HTTP side (decision 6).
    pub fn http(self, f: impl FnOnce(Router, &ActorSystem) -> Router + Send + 'static) -> Self;
    /// Runs after the node started (decision 7).
    pub fn on_start<F, Fut>(self, f: F) -> Self
    where F: FnOnce(ActorSystem) -> Fut + Send + 'static, Fut: Future<Output = anyhow::Result<()>> + Send + 'static;
    /// What `--version` prints.
    pub fn version(self, text: impl Into<String>) -> Self;
    /// Builds a runtime, reads the arguments and the environment of the process, and runs the command. For `serve`
    /// and `worker` it sets up logging and stops the node on SIGTERM or SIGINT. Returns the exit code. It must not
    /// be called inside a runtime.
    pub fn run(self) -> ExitCode;
    /// `run` on the caller's runtime: for a host whose `main` is async.
    pub async fn run_async(self) -> ExitCode;
    /// The same on the caller's runtime, with everything from outside passed in. `args` starts with the name of
    /// the program, as `std::env::args_os()` does. `shutdown` stops `serve` and `worker`; the other commands do not
    /// look at it. The future is `Send + 'static`.
    pub fn run_with(self, args: Vec<OsString>, env: Env, out: impl Write + Send + 'static,
        shutdown: impl Future<Output = ()> + Send + 'static)
        -> impl Future<Output = Result<(), AppError>> + Send + 'static;
}

pub enum AppError {
    /// The command line cannot be used: exit code 2.
    Usage(String),
    /// The command failed: exit code 1.
    Failed(anyhow::Error),
}

/// True when `args` name `serve` or `worker`.
pub fn node_command(args: &[OsString]) -> bool;
```

- The logging of `run`: `tracing` events to standard output, filtered by `RUST_LOG` (`info,sqlx=warn` when unset),
  with colours only on a terminal. It is left alone when the process has logging already.
- **Signals are watched only for `serve` and `worker`.** For an operator command `run` installs nothing, so Ctrl-C
  ends the process as it does today, and the terminal UI leaves the terminal as it does today. A second signal
  while a node stops changes nothing: the node takes its grace period, and whatever started the process may kill
  it.
- If the terminal UI cannot run inside a future that is `Send`, `run_with` gives it a thread of its own.

- `run` is `run_async` on a runtime of its own. A host with `#[tokio::main]`, or with actors that are built with
  `await`, calls `run_async`; without it such a host would have to write the signals, the logging and the exit
  codes itself around `run_with`, which is the code this change takes away.
- `http` and `on_start` can each be called more than once; the functions apply in the order of the calls.
- `run` and `run_async` read the environment without failing on a variable that is not UTF-8: they leave it out.
- `node_command` reads the arguments as the command line does: it is true when they would start a node. It is
  false for `--context serve get actors`, and for `serve --help`.
- `run_with` never ends the process and sets up no logging. It writes help and the version to `out` and returns
  `Ok`.
- `run` prints an error to standard error as `error: …`.
- The help, the usage errors and the hints name the binary as it was called, not `cicada`.
- `--version` prints what the host gave to `version`, and `cicada-app <version of the crate>` when it gave nothing:
  `App` does not know the version of the host. The plain `cicada` binary gives `cicada <version>`.

The exit codes:

| Case | Code |
| --- | --- |
| help, the version, a command that succeeded, a node that was stopped | 0 |
| a command line that cannot be read: an unknown command or flag, a value of a flag or of a variable that cannot be read, no store | 2 |
| an operator flag on `serve` or `worker`, a name in `--types` that the binary does not host, `worker --types none` | 2 |
| everything else that fails: a backend that is not built in, a store that stays away, an address that is taken, a node that cannot start (also for a setting that can be read and is not valid, `--concurrency 0`), an `on_start` that fails, an operator command that fails | 1 |

What a host builds before `run` is built for every command, also for `get actors` and `--help`. Actors are usually
cheap to build. A host with something costly, or something that an operator command lacks (an API key, an exporter
of telemetry), asks `node_command` first and builds it only for a node. Not chosen: taking functions that build
the actors, so that only `serve` and `worker` call them. It would make the one line above longer for every host.

### 2. The commands

`serve`, `worker`, and the operator commands of today's `cicada` at the same level (`get`, `describe`, `create`,
`delete`, `send`, `cancel`, `retry`, `config`, …, and `tui` with the feature of decision 10).

- `serve` hosts the types, serves the HTTP API at the root and `/ready`, and stops in good order.
- `worker` hosts types and serves nothing. It has no `/ready`: whatever runs it sees only the process. The
  operations guide says so.
- Both take `--types` (`CICADA_TYPES`): `all` (the default), or a list of type names with commas. `serve` also
  takes `none`: a node that serves HTTP and hosts nothing; `worker --types none` is an error. A name the binary
  does not host is an error that lists the hosted ones, and so is an empty name (`a,,b`); a name twice counts
  once. `all` and `none` are keywords when they are the whole value; a type of that name is picked in a list with
  another, or with `all`.
- **A binary without hosted types has neither `serve` nor `worker`.** The plain `cicada` binary is one, and it must
  not serve: nobody wrote a layer for it, so its `serve` would be a write API that nobody protected. One rule for
  it and for every binary without types keeps `App` free of a special case. The cost: a binary that only serves,
  without actor code, cannot be built on `App`; a host that wants a node that only sends runs its own binary with
  `serve --types none`.
- `serve` and `worker` refuse the flags of the operator commands, `--context`, `--server`, `--token` and `-o`.
  Today they are global, so `serve --token x` would be accepted and do nothing, and an operator would think the API
  is protected. The error says that `serve` has no authentication of its own and names `App::http`. The variables
  `CICADA_TOKEN` and `CICADA_SERVER` are left alone: a container carries them for the operator commands that are
  run inside it.

Not chosen: `--types` on `worker` only, as ADR 0018 reads. The end-to-end suite needs a serving node that hosts
nothing, and the sample service picks its types with this flag (decision 12), so one rule serves both commands.

### 3. The store and the listen address

| Flag | Variable | Default | Meaning |
| --- | --- | --- | --- |
| `--store` | `CICADA_STORE` | none: required | which store (below) |
| `--store-schema` | `CICADA_STORE_SCHEMA` | `public` | the schema, on PostgreSQL |
| `--store-pool-size` | `CICADA_STORE_POOL_SIZE` | the larger of the store's default and the concurrency plus 4 | the connections of this node, on PostgreSQL |
| `--retention` | `CICADA_RETENTION` | the store's default, 7 days | how long outcomes are kept, which is also the window in which an id is known (S10) |
| `--listen` | `CICADA_LISTEN` | `127.0.0.1:8080` | the address `serve` listens on |

A flag wins over its variable. A blank variable counts as unset.

The value of the store:

| Value | Store |
| --- | --- |
| starts with `postgres://` or `postgresql://` | PostgreSQL |
| starts with `sqlite:` (with or without `//`) | the SQLite file named after it |
| starts with `postgres:` or `postgresql:` otherwise, or holds `://` | refused |
| anything else | the path of a SQLite file |

A mistyped address (`postgress://…`, `postgres:db`) must not become the name of a SQLite file. A value that is not
taken as a SQLite file is never printed, in an error or in a log line: it can hold a password. The path of a SQLite
file may be named.

The retention has a flag because every node of a system should use the same one, and a host that wants another
would otherwise have to leave `App`. The pool size has one because the nodes of a system together must stay under
the connections the database allows, which depends on the deployment; the sample service has it today. The other
options of a store (time-outs) have none: a host that needs them uses the layers directly.

- **`serve` listens on the loopback address unless told otherwise (chosen).** The API can change and delete
  everything, and `App` has no authentication of its own. A host that opens it to a network says so with
  `--listen`, and should add its layer (decision 6). The docs say this where they show `--listen 0.0.0.0:8080`.
- `0.0.0.0` by default, as the sample service has. The first run of a new host would put an open write API on
  every interface.

A store that is not there yet: on PostgreSQL, `serve` and `worker` try again every second for up to 60 s while the
store answers "unavailable", as the sample service does; containers start in any order. The wait has no flag. It is
a loop over a function that opens the store, on the time of tokio, so its tests give it a function that fails and
run on virtual time.

The crate has the features `sqlite` and `postgres`, which turn on the features of the core. Neither is on by
default: a host names its store, as it does with the core.

- **No store by default (chosen).** It is the rule of the core, and the plain `cicada` binary, which can never open
  a store, builds none.
- Both by default. A first binary would run on either store without a line in its manifest, and every host and the
  operator's tool would build two database drivers they may not use.

A store whose backend is not built in is an error that says so and names the feature.

### 4. The settings of the node

Each has a flag and a variable, and maps to one field of `NodeConfig`:

| Flag | Variable | Field |
| --- | --- | --- |
| `--node-id` | `CICADA_NODE_ID` | `node_id` |
| `--concurrency` | `CICADA_CONCURRENCY` | `concurrency` |
| `--lease-ttl` | `CICADA_LEASE_TTL` | `lease_ttl` |
| `--heartbeat-interval` | `CICADA_HEARTBEAT_INTERVAL` | `heartbeat_interval` |
| `--poll-interval` | `CICADA_POLL_INTERVAL` | `poll_interval` |
| `--shutdown-grace-period` | `CICADA_SHUTDOWN_GRACE_PERIOD` | `shutdown_grace_period` |
| `--max-turn` | `CICADA_MAX_TURN` | `default_max_turn` |
| `--max-lost-claims` | `CICADA_MAX_LOST_CLAIMS` | `max_lost_claims`: a number, or `off` |

A duration is a number with `ms`, `s`, `m`, `h` or `d` (`250ms`, `5s`, `7d`), or ISO 8601 (`PT5S`, `P7D`).

The order: a flag, then its variable, then the host's `node_config`, then the default of the library. These are
the settings the sample service reads today, plus the limit of lost claims. A field without a row is set in code.

- **Flags and variables for these (chosen).** An operator changes them per deployment without a new build.
- Code only. The sample service would keep its own parsing, and so would every host.

Every variable starts with `CICADA_`. The operator commands already use `CICADA_SERVER`, `CICADA_TOKEN` and
`CICADA_CONFIG` and `CICADA_CONTEXT`; nothing clashes.

### 5. The operator commands go over HTTP, in every binary

They are today's commands, with today's rules for the server and the token (flag, variable, context). In a binary
that also serves, they still talk to a server over HTTP: `myapp get actors --server http://127.0.0.1:8080`. There
is no default server.

The plain `cicada` binary stays: it is `App::new().version(…).run()`, in `cicada-app`. It hosts nothing, so it has
the operator commands only (decision 2).

- **Keep `cicada` (chosen).** It costs three lines, and an operator need not have the host's binary.
- Drop it. The tests of the command line and the docs would need a host binary.

The crate `cicada-cli` goes. Its modules move into `cicada-app`.

### 6. The host changes the HTTP side with one function

```rust
App::new().host(CounterActor).http(|api, system| api.layer(my_auth).merge(my_public_routes(system)))
```

`http` gets the router of the API and the node, and returns the router to serve. A layer put on `api` covers the
API and whatever the host merged before it; routes merged after it are not covered. That is how axum works, and it
is what the sample service does today with its token.

`/ready` is answered by `App` outside the host's router and before it: no layer of the host covers it, and a route
`/ready` of the host is never reached. The docs of `http` say so.

Not chosen: a bearer token built into `App`. The proposal leaves authentication to the host, and one function
covers every scheme.

### 7. The start, and work after it

`serve` starts in this order:

1. It reads its settings and opens the store.
2. It binds its address. An address that is taken fails here, before a node exists.
3. It builds the node and the router: the functions of `http` run here. A route of the host that clashes with one
   of the API makes axum panic, and it does so before a turn was claimed.
4. It starts the node.
5. It runs the functions of `on_start`.
6. It writes one line to `out`, `Listening on http://<address>` with the address it bound, and takes requests. A
   test that asks for port 0 reads the port there.

`worker` does steps 1, 4 and 5.

`on_start(|system| async move { … })` runs once after the node started. An error stops the node and fails the
command. The sample service creates its singleton actors there.

Not chosen: leaving this to hosts "that use the layers directly". Creating an actor that must exist is what nearly
every host does at its start, and without it the sample service does not fit on `App`.

There is no hook for the stop. A host that started something of its own stops it when `run` or `run_with` returns.

### 8. Observers and telemetry

`App::observe(observer)` hands an observer to the node. `cicada-app` does not depend on OpenTelemetry: a host that
wants it asks `node_command`, sets up the SDK for a node, adds `ActorTelemetry` as an observer, and flushes when
`run` or `run_with` returns.

### 9. Stopping

`run` stops on SIGTERM and SIGINT. `run_with` stops when the future it was given completes. The stop is watched
from the first moment of the command, so SIGTERM is never left to kill the process.

`serve` stops as the operations guide says a host should:

1. The node stops: it claims no more, and running turns get the grace period of the node. Meanwhile the API keeps
   answering, and `/ready` answers `503`.
2. The HTTP server stops. Requests that are open get five seconds to end. The five seconds are a value that the
   inner function takes, so a test passes a shorter one.
3. The store is closed, and the command ends with exit code 0.

With the HTTP server first, as the sample service has it today, one request that stays open (a route of the host
that streams) would hold up the stop of the node until the platform kills the process, and every turn that runs
then is a lost claim (ADR 0025).

`worker` does steps 1 and 3.

A stop during the start:

- while the store is waited for: the wait ends at once, and the command ends with exit code 0;
- while the node starts: it is taken when the start has ended, and the node then stops as in step 1;
- while `on_start` runs: its function is dropped, and the node stops as in step 1, which can take the grace period
  of the node.

### 10. The terminal UI is a feature

The feature `tui` of `cicada-app` is off by default. Without it the binary has no `tui` command and does not
depend on `ratatui` or `crossterm`.

The checks of the workspace must still see the terminal UI and both stores, as they see every feature of the core.
The sample service asks for `cicada-app` with `tui`, so a build of the workspace has every feature of `cicada-app`
on (it asks for both stores too), and `cargo test --workspace` and clippy cover the terminal UI as today. CI and
`AGENTS.md` add what they have
for the core: `cargo check -p cicada-app` with no feature and with each of `sqlite`, `postgres` and `tui` alone.
They also add `cargo test -p cicada-app`, which builds the crate alone and so without `tui`: it runs the test that
such a binary does not know the command, and a `cargo tree` check that it does not depend on `ratatui` or
`crossterm`, as CI checks the drivers of the stores.

The rule for a backend that is not built in is a function of the value of the store and the list of backends that
are built in, so its test runs in every build.

Not chosen: the terminal UI tested only in a run of its own. The checks of the change loop run the workspace
commands, and its 2,900 lines would have gone unchecked there.

### 11. A config path that is a symlink

`cicada config` commands save through the link: they write the new file next to the link's target and rename it
over the target. The link stays a link. A relative link is read from the folder of the link. A link whose target
does not exist yet gets its target. The mode 0600 and the rule that a reader of the old file reads only the old
text hold for the target.

Not chosen: a note in the README. The fix is small, and a dotfiles setup that links the file is common.

### 12. The sample service on `App`

- Its `main` hosts every type of the sample and leaves the choice to `--types` (`CICADA_TYPES`): `HOSTED_TYPES` and
  its groups go. `on_start` creates the singleton actors of the types the node hosts.
- It reads `CICADA_NODE_ID` itself, because its actors need the node id when they are built, and passes it to `App`
  in `node_config`. Its `on_start` fails when the node runs under another id (a `--node-id` flag): its actors
  would record the wrong one. Turns that ran before that check have recorded it already; the flag is not for the
  sample, and the end-to-end suite never gives it.
- `http` adds its `/` route, its effects route and its bearer token. `GET /` names the node and its types; the
  store leaves it, since `http` does not get one.
- It starts its `FailureWatcher` in `on_start` and aborts it when `run_async` returns.
- Its `main` is async and calls `run_async`. For a node (`node_command`) it sets up its telemetry before and
  flushes it after.
- The variables it keeps: `API_TOKEN`, `FAILURE_WATCHER_INTERVAL`, and those of OpenTelemetry. All others are the
  variables of `App`: `PORT` becomes `CICADA_LISTEN`, `DATABASE_URL` and `SQLITE_PATH` become `CICADA_STORE`,
  `DATABASE_SCHEMA` becomes `CICADA_STORE_SCHEMA`, `DATABASE_POOL_SIZE` becomes `CICADA_STORE_POOL_SIZE`, and
  `NODE_ID`, `CONCURRENCY` and the timings get the prefix `CICADA_`.
- For this release the sample refuses to start a node when one of its old variables is set, and the error names
  the new one. Otherwise a deployment that still sets `DATABASE_URL` and `HOSTED_TYPES=none` would start without a
  word, on the SQLite file of the image, hosting every type.
- The image follows: its entry point runs `serve`, it sets `CICADA_LISTEN=0.0.0.0:8080`,
  `CICADA_STORE=/data/actors.db` and `CICADA_SHUTDOWN_GRACE_PERIOD=3s`, and its health check asks the port of
  `CICADA_LISTEN`. The
  listen address is set in the image and not in the compose file, because the end-to-end suite also starts nodes
  with `docker run`.
- The end-to-end compose files, the code that starts single nodes, and the tests that read `GET /` follow.

## Risks / Trade-offs

- The names of the sample service's variables change, `HOSTED_TYPES` and its groups go in favour of `CICADA_TYPES`
  with type names, and `GET /` loses the store (decision 12). Only the sample service and the end-to-end suite
  read them.
- `serve` on the loopback address surprises a host that runs it in a container: nothing answers from outside
  until `--listen` is set. The README and the operations guide show it in the first example.
- An `App` without the host's layer serves an API without authentication. The docs say so next to `http`.
- What a host builds before `run` is built for every command. A host that reads a setting itself, as the sample
  reads the node id, can disagree with a flag of `App`; the sample checks for it (decisions 1 and 12).
- A `worker` has no readiness check, and a binary that only serves cannot be built on `App` without a type
  (decision 2).
- The plain `cicada` builds axum, which the operator commands do not need.
- The crate `cicada-cli` is gone; whoever depended on it as a library uses `cicada-app`.
- A host names the features of its store on `cicada-app`; a binary built with none fails at `serve` and says so.
- The help of a binary depends on its name and on the feature `tui`. The proposal of `docs-per-layer`, which checks
  the help for drift, has a line on it.

## Migration Plan

Version 0.13.0. No migration of the store, no change of the wire format. ADR 0027 records where the app layer
departs from ADR 0018 and the names it fixes.

1. A host may replace its own start-up code by `App`. Nothing forces it: the layers below are as they were.
2. Whoever installs the command line builds `cicada-app` where it built `cicada-cli`, with the feature `tui` for
   the terminal UI. The binary is still called `cicada`.
3. A deployment of the sample service renames its variables (decision 12).

## Changed by the reviews

- First review: the sample service did not fit (decision 12); `serve` writes its address; the store value is read
  strictly and never printed; `--retention`; `on_start` in `worker`; the binary's own name and `version`; a
  `worker` without `/ready` is accepted.
- Second review: the node stops before the HTTP server (decision 9; the first version had it the other way round);
  `serve` and `worker` refuse the operator flags; the cost of what a host builds before `run`; the image of the
  sample; the features in the checks of the workspace (decision 10).
- Third review: the signatures and the exit codes (decision 1); the address is bound before the node starts
  (decision 7); a stop during `on_start` takes the grace period, not "at once"; `node_command`; a binary without
  types has no `serve` (the first versions gave the plain `cicada` one); the warning for `CICADA_TOKEN`; `/ready`
  outside the host's router; `postgres:` without `//` is refused; a duration in days; a dangling symlink.
- Fourth review: signals are watched only for `serve` and `worker`; the two fixed waits can be tested; a test run
  of the crate without `tui`; no warning for `CICADA_TOKEN` (the third review had added one); the honest reason for
  "no `serve` without types"; `--store-pool-size`; the sample calls `run`, so `init_logging` and `shutdown_signal`
  are not public; ADR 0027.
- Fifth review: `run_async`; no store feature by default; the router is built before the node starts; the sample
  refuses its old variables; what "never printed" covers; the small cases of `node_command`, `--types` and a stop
  while the node starts.

## Open Questions

None.
