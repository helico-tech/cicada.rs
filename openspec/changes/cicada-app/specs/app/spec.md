# app

## ADDED Requirements

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

The command `worker` SHALL run a node that hosts types and SHALL NOT listen on any address.

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

`serve` and `worker` SHALL take the store from `--store` or `CICADA_STORE`, and `serve` its address from `--listen`
or `CICADA_LISTEN`. A flag SHALL win over its variable, and a blank variable SHALL count as unset. Without a store
the command SHALL fail and name both ways to give one.

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
the default of the store and the concurrency of the node plus 4.

#### Scenario: The retention

- **WHEN** the binary runs `serve --retention 1h`, and then `serve --retention 14d`
- **THEN** the store is opened with a retention of one hour, and then of fourteen days

#### Scenario: The pool

- **WHEN** the binary runs `serve --store-pool-size 5` with a PostgreSQL store, and then without the flag at a
  concurrency of 40
- **THEN** the store is opened with 5 connections, and then with 44

### Requirement: `serve` listens on the loopback address by default

Without `--listen` and `CICADA_LISTEN`, `serve` SHALL listen on `127.0.0.1:8080`.

#### Scenario: The default address

- **WHEN** the binary reads the settings of `serve` with only a store
- **THEN** the address is `127.0.0.1:8080`

### Requirement: A store that is not there yet is waited for

On PostgreSQL, `serve` and `worker` SHALL try again while the store answers that it is unavailable, for up to 60
seconds, and then fail.

#### Scenario: The database comes up late

- **WHEN** the binary runs `serve` and the database starts to answer after five seconds
- **THEN** the node starts

#### Scenario: The database stays away

- **WHEN** the database does not answer for 60 seconds
- **THEN** the command fails with an error that says the store is unavailable

### Requirement: The settings of the node come from flags, variables and the host

The flags `--node-id`, `--concurrency`, `--lease-ttl`, `--heartbeat-interval`, `--poll-interval`,
`--shutdown-grace-period`, `--max-turn` and `--max-lost-claims`, each with a variable `CICADA_…` of the same name,
SHALL set the fields of `NodeConfig`. A flag SHALL win over its variable, the variable over the `NodeConfig` the
host gave to `App`, and that over the default of the library.

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

- **WHEN** `CICADA_LEASE_TTL` is `soon`
- **THEN** the command fails with exit code 2, and the error names `CICADA_LEASE_TTL`

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
`worker`. An error SHALL stop the node and fail the command.

#### Scenario: An actor that must exist

- **WHEN** the host creates the actor `dead-letters/_` in `on_start`
- **THEN** the actor exists when the first request is served

#### Scenario: In a worker

- **WHEN** a binary with an `on_start` function runs `worker`
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
the binary as it was called. `serve` and `worker` SHALL refuse the flags `--context`, `--server`, `--token` and
`-o`.

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
