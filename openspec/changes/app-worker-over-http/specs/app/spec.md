## MODIFIED Requirements

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

### Requirement: The settings of the node come from flags, variables and the host

The flags `--node-id`, `--concurrency`, `--lease-ttl`, `--heartbeat-interval`, `--poll-interval`,
`--shutdown-grace-period`, `--max-turn` and `--max-lost-claims`, each with a variable `CICADA_…` of the same name,
SHALL set the fields of `NodeConfig` for serve and direct-store workers. A flag SHALL win over its variable, the
variable over the `NodeConfig` the host gave to `App`, and that over the default of the library. HTTP workers SHALL
map only applicable settings to WorkerConfig and refuse explicit server-owned settings as specified below.

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

## ADDED Requirements

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
