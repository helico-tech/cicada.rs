# app Specification

## Purpose
A set of actor types becomes a binary: `serve` runs a node with the HTTP API, `worker` runs a node without it, and
the operator commands talk to a node over HTTP. The store, the address and the settings of the node come from flags
and `CICADA_` variables, and a host adds its own layer, its work after the start and its observers.

## Requirements

### Requirement: A set of actors becomes a binary that serves

`App::new().host(actor).run()` SHALL give a binary whose command `serve` runs a node that hosts the actors, serves
the HTTP API at the root and a public `/ready`, and stops in good order.

#### Scenario: Serve, create and send

- **WHEN** a binary built from `App` with a counter type runs `serve` on a SQLite file, and a client creates a
  counter and sends it a message over HTTP
- **THEN** the message ends `done`, and `GET /ready` answers `200`

#### Scenario: Stopping

- **WHEN** the binary is told to stop while a turn runs
- **THEN** the node claims no more and the turn finishes within the grace period of the node; until then the API
  still answers and `GET /ready` answers `503`; then the HTTP server stops, the store is closed, and the command
  ends with exit code 0

#### Scenario: A request that stays open

- **WHEN** the binary is told to stop while a route of the host holds a request open
- **THEN** the node stops without waiting for that request, and the command ends within five seconds after it

#### Scenario: A stop while the store is waited for

- **WHEN** the binary is told to stop while it waits for its store
- **THEN** the command ends at once with exit code 0

#### Scenario: A stop during an operator command

- **WHEN** the stop that `run_with` was given completes while `get actors` waits for its answer
- **THEN** the command goes on and prints its answer

#### Scenario: A route of the host that clashes with the API

- **WHEN** the host merges a route that the API already has
- **THEN** the command fails before the node was started

#### Scenario: A stop while `on_start` runs

- **WHEN** the binary is told to stop while a function of `on_start` runs
- **THEN** the function is dropped, the node stops in good order, the store is closed, the command ends with exit
  code 0, and it never took a request

#### Scenario: The address it listens on

- **WHEN** the binary runs `serve --listen 127.0.0.1:0`
- **THEN** it writes one line `Listening on http://127.0.0.1:<port>` with the port it bound, and answers there

#### Scenario: The address is taken

- **WHEN** the binary runs `serve` on an address that another process listens on
- **THEN** the command fails with exit code 1 and an error that names the address, before a node was started and
  before `on_start` ran

#### Scenario: The node cannot start

- **WHEN** the binary runs `serve` on a store that holds another contract for one of its types
- **THEN** the command fails with the problem of the start, exit code 1, and nothing listens

### Requirement: A worker hosts types and serves nothing

The command `worker` SHALL host selected types through either a direct-store ActorSystem or an HTTP-backed Worker
and SHALL NOT listen on any address. HTTP mode SHALL work without a store feature or database access.

#### Scenario: A worker next to a serving node

- **WHEN** one binary runs `serve --types none` and another runs `worker` on the same store, and a client sends a
  message over HTTP to the first
- **THEN** the worker handles it

#### Scenario: A worker that hosts nothing

- **WHEN** the binary runs `worker --types none`
- **THEN** the command fails with exit code 2 and an error that says a worker must host a type

### Requirement: `--types` picks what a node hosts

`serve` and `worker` SHALL host every type of the binary unless `--types` or `CICADA_TYPES` names a list of them.
`serve` SHALL also accept `none`. A name that the binary does not host SHALL be an error that lists the hosted
types. `all` and `none` SHALL be keywords only when they are the whole value.

#### Scenario: A list

- **WHEN** a binary with the types `a` and `b` runs `worker --types a`
- **THEN** the node hosts `a` and not `b`

#### Scenario: None

- **WHEN** the binary runs `serve --types none`
- **THEN** it serves the HTTP API and claims no message

#### Scenario: A type the binary does not have

- **WHEN** the binary runs `serve --types a,nope`
- **THEN** the command fails, and the error names `nope` and lists `a` and `b`

#### Scenario: An empty name and a name twice

- **WHEN** the binary runs `worker --types a,,b`, and then `worker --types a,a`
- **THEN** the first fails with exit code 2, and the second hosts `a` once

#### Scenario: A type called `all`

- **WHEN** a binary with the types `all` and `b` runs `worker --types all,b`, and then `worker --types all`
- **THEN** both runs host both types

### Requirement: The store and the listen address come from flags or variables

`serve` and a direct-store `worker` SHALL take the store from `--store` or `CICADA_STORE`, and `serve` its address
from `--listen` or `CICADA_LISTEN`. A flag SHALL win over its variable, and a blank variable SHALL count as unset.
Without a store, serve SHALL fail and name both ways to give one. Worker SHALL select its store/HTTP mode before
resolving mode-specific settings; an HTTP worker SHALL not read or open a store.

#### Scenario: A SQLite file

- **WHEN** the binary runs `serve --store actors.db`
- **THEN** the node runs on the SQLite file `actors.db`

#### Scenario: PostgreSQL from a variable

- **WHEN** `CICADA_STORE` is a `postgres://` address and the binary runs `worker`
- **THEN** the node runs on that database, in the schema `public`

#### Scenario: A flag wins

- **WHEN** `CICADA_LISTEN` is `127.0.0.1:9000` and the binary runs `serve --listen 127.0.0.1:9100`
- **THEN** it listens on port 9100

#### Scenario: No store

- **WHEN** the binary runs `serve` without `--store` and without `CICADA_STORE`
- **THEN** the command fails, and the error names `--store` and `CICADA_STORE`

### Requirement: The value of the store says which store it is

A store that starts with `postgres://` or `postgresql://` SHALL be PostgreSQL, with the schema of `--store-schema`.
A value that starts with `sqlite:` SHALL be the SQLite file named after it. Another value that starts with
`postgres:` or `postgresql:`, or that holds `://`, SHALL be refused. Any other value SHALL be the path of a SQLite
file. A value that is not taken as a SQLite file SHALL NOT be printed, in an error or in a log line.

#### Scenario: An address of another kind

- **WHEN** the binary runs `serve --store postgress://user:secret@host/db`
- **THEN** the command fails, the error names the kinds of store it knows, and it does not hold `secret`

#### Scenario: A PostgreSQL address without its slashes

- **WHEN** the binary runs `serve --store postgres:db`
- **THEN** the command fails, and no file `postgres:db` is created

#### Scenario: A SQLite address

- **WHEN** the binary runs `serve --store sqlite:actors.db`, and then `serve --store sqlite://actors.db`
- **THEN** both runs use the SQLite file `actors.db`

#### Scenario: A backend that is not built in

- **WHEN** a binary built without the feature `postgres` runs `serve` with a `postgres://` store
- **THEN** the command fails with exit code 1, and the error says that the binary was built without PostgreSQL and
  names the feature

### Requirement: The retention and the pool of the store can be set

`--retention` SHALL set how long the store keeps outcomes, and `--store-pool-size` the connections of the node on
PostgreSQL, each with a variable `CICADA_…` of the same name. Without the second, the pool SHALL be the larger of
the default of the store and the concurrency of the node plus 4. A given pool below the concurrency plus 4 SHALL be
used as given; the node's own check warns about it.

#### Scenario: The retention

- **WHEN** the binary runs `serve --retention 1h`, and then `serve --retention 14d`
- **THEN** the store is opened with a retention of one hour, and then of fourteen days

#### Scenario: The pool

- **WHEN** the binary runs `serve --store-pool-size 5` with a PostgreSQL store, and then without the flag at a
  concurrency of 40
- **THEN** the store is opened with 5 connections, and then with 44

#### Scenario: A given pool that is too small

- **WHEN** the binary runs `serve --store-pool-size 5 --concurrency 16` with a PostgreSQL store
- **THEN** the store is opened with 5 connections, not raised to 20

#### Scenario: The default pool is large enough

- **WHEN** the binary runs `serve` with a PostgreSQL store at any concurrency, without `--store-pool-size` or
  `CICADA_STORE_POOL_SIZE`
- **THEN** the pool is at least the concurrency plus 4

### Requirement: `serve` listens on the loopback address by default

Without `--listen` and `CICADA_LISTEN`, `serve` SHALL listen on `127.0.0.1:8080`.

#### Scenario: The default address

- **WHEN** the binary reads the settings of `serve` with only a store
- **THEN** the address is `127.0.0.1:8080`

### Requirement: A store that is not there yet is waited for

On PostgreSQL, `serve` and `worker` SHALL try again while the store answers that it is unavailable, SHALL start no new
try after 60 seconds, and then fail.

#### Scenario: The database comes up late

- **WHEN** the binary runs `serve` and the database starts to answer after five seconds
- **THEN** the node starts

#### Scenario: The database stays away

- **WHEN** the database does not answer for 60 seconds
- **THEN** the command fails with an error that says the store is unavailable

### Requirement: The settings of the node come from flags, variables and the host

Node flags and their CICADA variables SHALL set NodeConfig for serve and direct-store workers: node-id,
node-label, concurrency, lease-ttl, heartbeat-interval, poll-interval, shutdown-grace-period, max-turn and
max-lost-claims. Precedence SHALL be flag, nonempty variable, host config, then library default. HTTP workers SHALL
map only applicable settings and refuse explicit settings they cannot honor as specified below.

#### Scenario: A variable over the host's value

- **WHEN** the host gave a `NodeConfig` with a concurrency of 4 and `CICADA_CONCURRENCY` is 8
- **THEN** the node runs at most 8 turns at once

#### Scenario: The host's value over the default

- **WHEN** the host gave a `NodeConfig` with a concurrency of 4 and neither the flag nor the variable is set
- **THEN** the node runs at most 4 turns at once

#### Scenario: The limit of lost claims is turned off

- **WHEN** the binary runs `serve --max-lost-claims off`
- **THEN** the node has no limit of lost claims

#### Scenario: A value that cannot be read

- **WHEN** a direct-store command reads `CICADA_LEASE_TTL` as `soon`
- **THEN** the command fails with exit code 2, and the error names `CICADA_LEASE_TTL`

#### Scenario: Label precedence

- **WHEN** a label is supplied by flag, variable and host config
- **THEN** --node-label wins, else CICADA_NODE_LABEL, else the host's label, else None
- **AND** an empty environment value is unset, while whitespace-only is invalid
- **AND** valid leading/trailing spaces are preserved from the variable as from the flag or host configuration
- **AND** invalid explicit values are usage errors (exit 2); invalid host config follows start validation (exit 1)

### Requirement: The host can change the HTTP side

`App::http` SHALL give the host the router of the API and the node, and SHALL serve the router the host returns.
`App` SHALL answer `/ready` itself, outside that router. Functions given in several calls SHALL apply in the order
of the calls.

#### Scenario: A layer of the host

- **WHEN** the host puts a layer on the API that refuses a request without a token
- **THEN** a request to the API without the token gets the layer's refusal, and `GET /ready` answers `200`

#### Scenario: A route of the host

- **WHEN** the host merges a route `/info` after its layer
- **THEN** `GET /info` answers without the token

#### Scenario: A route `/ready` of the host

- **WHEN** the host merges a route `/ready` of its own
- **THEN** the binary starts, and `GET /ready` gives the answer of `App`

### Requirement: The host can work after the start

`App::on_start` SHALL run the host's function once after the node started, in `serve` before it listens and in
direct-store `worker`. An error SHALL stop the node and fail the command. HTTP mode with any on_start hook SHALL
fail as a usage error before reading config or performing network/store access, rather than silently skipping hooks.

#### Scenario: An actor that must exist

- **WHEN** the host creates the actor `dead-letters/_` in `on_start`
- **THEN** the actor exists when the first request is served

#### Scenario: In a worker

- **WHEN** a binary with an `on_start` function runs a direct-store `worker`
- **THEN** the function runs once, after the node started

#### Scenario: The function fails

- **WHEN** the function returns an error
- **THEN** the command fails with that error, the node is stopped and nothing listens

### Requirement: The host's observers and node settings reach the node

An observer given to `App::observe` SHALL be told the turns of the node.

#### Scenario: An observer

- **WHEN** the host gave an observer and the node runs a turn
- **THEN** the observer is told its start and its end

### Requirement: The operator commands of the binary go over HTTP

A binary built from `App` SHALL have the operator commands of `cicada`, with the same rules for the server and the
token. They SHALL talk to a server over HTTP, also in a binary that can serve. The help and the hints SHALL name
the binary as it was called. `serve` SHALL refuse `--context`, `--server`, `--token` and `-o`. Worker SHALL accept
HTTP target/auth flags before or after the subcommand and SHALL refuse `-o`/`--output`; its store mode SHALL also
refuse an explicit `--token`. Existing operator behavior SHALL remain unchanged.

#### Scenario: A command of the host's binary

- **WHEN** a host's binary runs `get actors --server` with the address of a serving node
- **THEN** it lists the actors of that node

#### Scenario: No server

- **WHEN** it runs `get actors` with no server from a flag, a variable or a context
- **THEN** the command fails with exit code 1 and the hint of the spec `cli` on how to name a server

#### Scenario: The name in a hint

- **WHEN** a binary called `myapp` runs `get actors` with no server configured
- **THEN** the hint names `myapp config set-context`, not `cicada`

#### Scenario: The version

- **WHEN** the host gave `App::version("myapp 1.2.3")` and the binary runs `--version`
- **THEN** it prints `myapp 1.2.3`; without that call it prints `cicada-app` and the version of the crate

#### Scenario: An operator flag on `serve`

- **WHEN** the binary runs `serve --token secret`
- **THEN** the command fails before anything starts, and the error says that `serve` has no authentication of its
  own

### Requirement: A binary without types has only the operator commands

A binary built from an `App` that hosts no type SHALL have neither `serve` nor `worker`. `cicada-app` SHALL build
the binary `cicada`, which is such a binary.

#### Scenario: An operator command of the plain binary

- **WHEN** `cicada get actors` is run against a node
- **THEN** it lists the actors of that node, as the spec `cli` says

#### Scenario: No serve without types

- **WHEN** `cicada serve --store actors.db` is run
- **THEN** the command line refuses `serve` as a command it does not know, with exit code 2

#### Scenario: The version of the plain binary

- **WHEN** `cicada --version` is run
- **THEN** it prints `cicada` and the version of the crate

### Requirement: A host can run the command on its own runtime

`App::run_with` SHALL run the command with the arguments, the environment, the output and the stop it is given. It
SHALL NOT end the process or set up logging. It SHALL write help and the version to its output and return without
an error, and SHALL say for an error whether the command line could not be used or the command failed.
`App::run_async` SHALL do what `run` does, on the runtime of its caller. `node_command` SHALL say whether arguments
would start a node.

#### Scenario: Help

- **WHEN** `run_with` gets the arguments `myapp --help`
- **THEN** it writes the help to its output, with the name `myapp`, and returns without an error

#### Scenario: A command line that cannot be read

- **WHEN** `run_with` gets the arguments `myapp serve --lease-ttl soon`
- **THEN** it returns the error of a command line that cannot be used, which `run` turns into exit code 2

#### Scenario: Whether a node runs

- **WHEN** a host asks `node_command` about `myapp serve --store a.db`, `myapp worker`, `myapp get actors`,
  `myapp --help`, `myapp --context serve get actors` and `myapp serve --help`
- **THEN** the answers are yes, yes, no, no, no and no

#### Scenario: A host with an async main

- **WHEN** a host calls `run_async` from its `#[tokio::main]` function with the arguments of `serve`
- **THEN** the node serves and stops on SIGTERM as with `run`, and the call returns the exit code

### Requirement: Worker mode is selected before mode-specific configuration

Explicit --store SHALL select store mode; --server or --context SHALL select HTTP mode. Combining those mode flags
SHALL be a usage error. Without mode flags, nonblank CICADA_STORE SHALL select store mode before HTTP target
variables; otherwise nonblank CICADA_SERVER or CICADA_CONTEXT SHALL select HTTP. Tokens and current-context alone
SHALL not select a mode. No selected mode SHALL fail with exit 2 and a store/server hint.

#### Scenario: Explicit selection overrides ambient settings

- **WHEN** worker has --server or --context and an invalid CICADA_STORE
- **THEN** it selects HTTP without parsing the store value
- **WHEN** worker has --store and ambient HTTP target/auth variables
- **THEN** it selects the store and ignores the HTTP variables/config file
- **WHEN** --store is combined with --server or --context, including flags before worker
- **THEN** it fails with exit 2 before opening a store or issuing an HTTP request

#### Scenario: Existing deployments with operator variables

- **WHEN** worker has no mode flag and both CICADA_STORE and CICADA_SERVER/CICADA_CONTEXT are set
- **THEN** it remains a direct-store worker
- **WHEN** only HTTP target variables select a mode
- **THEN** it selects HTTP
- **AND** whitespace-only mode variables count as unset, including during worker HTTP target resolution

#### Scenario: Context and token alone

- **WHEN** no mode selector is given but the config file has current-context or a token is set
- **THEN** worker fails with exit 2 rather than silently selecting that context's server
- **WHEN** --token is explicitly given in store mode
- **THEN** it fails with exit 2 naming the need for HTTP mode

#### Scenario: HTTP target resolution

- **WHEN** HTTP mode is selected
- **THEN** it loads the existing operator config and resolves server/token/context headers with existing precedence
- **AND** a context's token/headers may apply even when --server overrides its URL, as documented for operators
- **AND** a config read/resolve failure is exit 1
- **WHEN** store mode has a broken CICADA_CONFIG file
- **THEN** that file is not read and cannot fail the worker

### Requirement: HTTP workers use local execution settings and server lease policy

HTTP mode SHALL map node id, concurrency, poll interval, max turn and shutdown grace to WorkerConfig with flag over
environment over host over default precedence. Host shutdown_cancel_timeout SHALL be retained. Types and observers
SHALL reach the Worker. Store configuration and lease policy SHALL remain server-owned.

#### Scenario: Applicable settings

- **WHEN** --concurrency overrides CICADA_CONCURRENCY and the host's NodeConfig value
- **THEN** the Worker uses the flag's value, with equivalent precedence for id/poll/max-turn/grace
- **AND** the selected --types/CICADA_TYPES and every App observer are used
- **AND** --types none remains a usage error and node_command recognizes an HTTP worker invocation

#### Scenario: Explicit server settings

- **WHEN** HTTP mode includes --store-schema, --store-pool-size, --retention, --lease-ttl,
  --heartbeat-interval or --max-lost-claims
- **THEN** it fails with exit 2 naming the offending server-owned setting
- **AND** it does not silently accept a flag the remote worker cannot honor

#### Scenario: Ignored settings cannot break HTTP execution

- **WHEN** the corresponding environment variables contain malformed values, or host NodeConfig has invalid unused
  lease/heartbeat/sweep/claim-limit fields, or CICADA_LISTEN is invalid
- **THEN** HTTP mode does not read/validate those values
- **AND** applicable WorkerConfig validation still applies, with parse failures as exit 2 and start failures as exit 1

#### Scenario: No database dependency

- **WHEN** an HTTP worker has an invalid PostgreSQL address in CICADA_STORE, or a SQLite path that does not exist
- **THEN** it completes work through HTTP without opening that database or creating that file
- **AND** the HTTP execution path compiles without sqlite/postgres features of cicada-app

#### Scenario: Startup hooks require an ActorSystem

- **WHEN** the App has any on_start hook and HTTP worker mode is selected
- **THEN** the command fails with exit 2 before config-file/network/store access, explaining the incompatible hook
- **AND** it does not skip the hook silently or create a surrogate ActorSystem
- **AND** App::http remains serve-only and does not create a listener for either worker mode

### Requirement: HTTP worker startup waits for temporary unavailability and remains stoppable

After configuration, HTTP mode SHALL probe TurnSource::list_types, retrying only Unavailable after one second and
starting no probe at or after 60 seconds from the first attempt. Each probe SHALL use the client's 30s bound, so
the final in-flight attempt may finish after the retry horizon. Stop SHALL interrupt pending probes and waits.

#### Scenario: The server comes up late

- **WHEN** type listing is temporarily Unavailable and succeeds before the retry horizon
- **THEN** the App proceeds to Worker start/registration
- **AND** no handler runs merely because a probe succeeded

#### Scenario: Retry horizon

- **WHEN** the server remains Unavailable
- **THEN** the command fails with exit 1 without starting a probe at/after 60s, including after a retry sleep
- **AND** a probe begun before the horizon may finish later within its HTTP timeout
- **AND** documentation does not promise a strict 60s total startup bound
- **AND** the approximately 90s preflight bound does not include subsequent sequential contract registration or its
  possible conflict-diagnostic reads

#### Scenario: Permanent refusal or stop

- **WHEN** a probe returns Invalid or Fatal, such as authentication refusal or an unknown route
- **THEN** it fails with exit 1 without retrying for a minute
- **WHEN** stop arrives during a request or retry wait
- **THEN** the command stops waiting and returns success without starting a Worker

#### Scenario: Registration fails after successful preflight

- **WHEN** the subsequent Worker start fails, including a conflicting contract or temporary registration failure
- **THEN** the App stops the Worker for cleanup and fails with exit 1 and start diagnostics
- **AND** it does not retry by interpreting StartError text

### Requirement: HTTP worker lifecycle preserves Worker completion and failure semantics

The App SHALL share hosted actor definitions between ActorSystem and Worker without a core API change, build the
Worker with its settings/observers and run until stop or automatic Worker failure. Shutdown SHALL call Worker::stop
and retain its recorded error. The App SHALL never close the remote source or introduce another lease release policy.

#### Scenario: Stop during registration

- **WHEN** stop wins the race with Worker start
- **THEN** the App drops start and calls worker.stop, with no late dispatch
- **AND** any contract registration already persisted may remain

#### Scenario: A stopped worker and a failed worker

- **WHEN** a signal stops a healthy running Worker
- **THEN** it drains/cancels using its configured waits and exits 0
- **WHEN** Worker stops automatically with a recorded error
- **THEN** the command exits 1 with that error
- **WHEN** a signal races with an already-recorded automatic failure
- **THEN** the App checks stopped after cleanup and preserves exit 1 rather than masking the failure

#### Scenario: Uncooperative or finished work

- **WHEN** work outlives cooperative stop waits
- **THEN** App follows Worker's abandonment rules and does not release beneath running work
- **WHEN** a handler has already ended
- **THEN** App does not release or rerun it; completion/uncertainty retains the core Worker's rules
- **AND** HTTP worker creates no readiness listener or node-table membership

### Requirement: The App worker client preserves authentication and disables redirects

The internal worker-client constructor SHALL use existing context headers and sensitive bearer authentication,
disable redirects and add no application retry policy. Its 30s total HTTP timeout SHALL coexist with ActorClient's
10s turn timeout. Existing operator clients SHALL retain their behavior.

#### Scenario: Redirect refusal

- **WHEN** the server answers a lease call with 307
- **THEN** the client does not follow it or resend the POST, and Worker's Fatal handling stops the command with exit 1

#### Scenario: Credentials and diagnostics

- **WHEN** the host layer requires the resolved bearer token or context header
- **THEN** the worker sends it to type registration/listing and lease routes
- **AND** documented permissions include GET /types/{name}, used to obtain an existing contract after a conflict
- **AND** Target/settings Debug omits token/header values and hides URL passwords
- **AND** new App diagnostics do not intentionally print raw credentials or request headers

#### Scenario: Documentation and generated CLI

- **WHEN** CLI reference and examples are checked
- **THEN** docs/reference/cli.md matches the new worker help and a compiled app-guide example shows HTTP execution
- **AND** the guide states mode precedence, ignored/refused settings, startup-hook incompatibility, auth route coverage,
  timeouts and stop/error behavior

### Requirement: App-produced usage errors hide URL passwords

App-produced usage errors SHALL replace the password in recognized scheme://user:password@host text with ***,
including raw apostrophes, and SHALL process adjacent addresses separately. Redaction SHALL be idempotent and retain
exit code 2. This scoped scanner SHALL NOT promise arbitrary secret scrubbing, libpq/non-// forms, or credentials
truncated by whitespace, slash, question mark, hash, double quote or backtick. Ambiguous punctuation MAY be hidden.

#### Scenario: Apostrophe in a clap-rejected argument

- **WHEN** an App command rejects `postgres://u:S3'CRET@h/db` as an unexpected argument
- **THEN** its Usage error exits with code 2 and prints the address as `postgres://u:***@h/db`
- **AND** neither secret fragment appears elsewhere in the rendered error, and the application has not started

#### Scenario: Apostrophes in a settings error

- **WHEN** CICADA_LISTEN is `postgres://o'neil:S3'CRET@h/db` and settings reject it
- **THEN** the error retains the CICADA_LISTEN context and address `postgres://o'neil:***@h/db`
- **AND** no password fragment is printed and the application has not started

#### Scenario: Adjacent addresses

- **WHEN** a usage error contains `'a://u:p'q@h','b://v:r's@k'`
- **THEN** it prints `'a://u:***@h','b://v:***@k'`
- **WHEN** --types rejects `a://h,,b://u:secret@k` for an empty name
- **THEN** the error retains the first address and prints the second as `b://u:***@k`

#### Scenario: Credential characters and existing forms

- **WHEN** a recognized password contains sub-delimiters including apostrophe, percent escapes or Unicode
- **THEN** the whole password is replaced without decoding escapes or corrupting text
- **AND** an empty password is replaced, multiple at-signs use the last at-sign as today, and clear host/port/path context remains

#### Scenario: No password and repeated redaction

- **WHEN** diagnostic text contains ordinary text, user@host, host:port or bracketed IPv6 without a password
- **THEN** those forms remain unchanged
- **AND** applying redaction again to any already-redacted diagnostic produces the same output

#### Scenario: An incomplete authority

- **WHEN** a candidate authority has no at-sign
- **THEN** it is left unchanged because the scanner cannot distinguish a port from incomplete credentials
- **AND** the documented guarantee does not claim to hide such truncated input

### Requirement: HTTP workers refuse explicit node labels

An App worker over HTTP SHALL refuse --node-label because it has no node row. It SHALL ignore CICADA_NODE_LABEL and
the host's NodeConfig.label without validating them. This SHALL NOT change WorkerConfig or remote lease identity.

#### Scenario: Label flag in HTTP mode

- **WHEN** worker --server or --context selects HTTP execution with --node-label
- **THEN** it fails with exit 2 and a diagnostic explaining that a worker over HTTP has no node row

#### Scenario: Unused label cannot break HTTP execution

- **WHEN** HTTP execution has a malformed CICADA_NODE_LABEL or an invalid host label but no --node-label flag
- **THEN** it does not read or validate either label, and applicable worker settings still apply normally
