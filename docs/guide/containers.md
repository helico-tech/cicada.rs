# Run containers

`cicada-containers` is an optional crate for applications that run containers. It has two levels:

- **The runtime**, `ContainerRuntime`: calls on containers, networks and volumes, found by names you choose. Call it
  from host code, outside actor turns, for work such as following logs or collecting files.
- **The container actor**, `ContainerActor`: one durable actor per container, which creates, starts, watches, stops
  and removes it, and reports its end once to a target you name.

No other Cicada crate depends on it ([ADR 0056](../adr/0056-shared-external-resource-crates.md)). Add it from the
same tag as the other crates ([install](../how-to/install.md#choose-the-crates)): `features = ["docker"]` for
`DockerRuntime`, and `features = ["testing"]` in dev-dependencies for `FakeRuntime`.

This page has no compiled examples of its own yet. The crate's rustdoc has tested examples (generate it with
`cargo doc -p cicada-containers --all-features --no-deps`), and
[`tests/actor.rs`](../../crates/cicada-containers/tests/actor.rs) hosts the container actor end to end.

## Choose a runtime

| Runtime | Use it for |
| --- | --- |
| `DockerRuntime` (feature `docker`) | A Docker Engine, local or over plain TCP |
| `FakeRuntime` (feature `testing`) | Tests on virtual time; see [Test with the fake](#test-with-the-fake) |
| Your own `impl ContainerRuntime` | Another engine; see [Write or wrap a runtime](#write-or-wrap-a-runtime) |

The trait is an `async_trait` and works as `Arc<dyn ContainerRuntime>`. No public item names a Docker client type.

### Connect to Docker

`DockerRuntime::connect()` uses the Docker the `docker` CLI uses, in this order: `DOCKER_HOST` (`unix://`, `tcp://` or
`http://`); else the context `DOCKER_CONTEXT` names; else `/var/run/docker.sock` when it exists; else the CLI's current
context (`docker context use`). So on a Mac with OrbStack, Colima or Docker Desktop it works without a link at
`/var/run/docker.sock`. Context lookup arrives in 0.22.0; up to 0.21.1, `connect()` reads only `DOCKER_HOST` and the
local socket.

`connect()` does not contact the daemon: the API version is agreed on the first call, so a host starts while Docker is
down and its calls fail with the retryable `DaemonUnavailable` until it is up. It needs Docker 25 or later and the
host's Tokio runtime. For a setup `connect()` does not take, such as an endpoint the host knows, pass it to
`DockerRuntime::connect_to("tcp://build-host:2375")`.

- A machine with a working local socket keeps using it whatever the current context is; only `DOCKER_CONTEXT` or
  `DOCKER_HOST` move it. Set `DOCKER_CONTEXT` when `docker` and your host must agree.
- There is no TLS. A TLS context, an `https://` or `ssh://` endpoint and a context that cannot be read fail with
  `Other`, naming the context; nothing falls back to the socket. Set `DOCKER_HOST` to a plain endpoint, or use
  `connect_to`. A context whose socket is missing is `DaemonUnavailable`.
- The context is read once, at `connect()`: switching contexts later does not move a runtime. `docker_config(dir)`
  sets only where pulls find logins, not where `connect()` finds contexts.

**Keep secrets out of logs.** bollard logs request bodies, secret environment included, at `debug` under the target
`bollard`. Keep that target at `info` or above, for example `RUST_LOG=debug,bollard=info`. The backend itself logs
nothing, and its error messages never hold a secret value. The Docker socket gives root on the machine: never mount it
into a container you do not trust.

### Private images

`create` pulls a missing image with the machine's `docker login`, so `docker login <registry>` on the machine is
enough. At each pull it reads `config.json`, as the Docker CLI does, from the folder set with
`DockerRuntime::connect()?.docker_config(dir)`, else `DOCKER_CONFIG`, else `$HOME/.docker`; a login made after
`connect` counts. For the image's registry it takes the helper that `credHelpers` names, else the `credsStore` helper
(Docker Desktop and OrbStack keep logins there), else the `auths` entry. A helper runs as
`docker-credential-<name> get` from `PATH` and is killed after 10 seconds.

A missing or broken file, a registry without an entry or a failing helper gives a pull without credentials, so a
public image still works. The message of a failed pull ends with where its credentials came from or why there were
none, such as `(no credentials for registry.example.com)`; never with a credential. A refused login keeps its kind:
Docker Hub and `registry:2` say "pull access denied" (`ImageNotFound`), others "unauthorized" (`Other`, which the
container actor retries).

- Run `docker login` as the user the host runs as. A service whose `$HOME` is another user's reads that user's
  `~/.docker`: give it the folder with `docker_config` or `DOCKER_CONFIG`.
- On Windows, set one of the two: without them the runtime reads only `$HOME/.docker`, not the profile folder.
- A keychain helper that waits for a prompt is killed after 10 seconds, and the pull runs without credentials.
- Cicada never logs in itself. Credentials never go into the spec, the store, a log line or an error.

### The runtime's machine

`runtime.info().await` returns a `RuntimeInfo` with `arch` (`amd64`, `arm64`, `arm`) and `os` (`linux`), in the
spelling of image platforms, so you can compare them with an image's platform as they are. These are Go's names, not
Rust's `std::env::consts::ARCH` (`x86_64`, `aarch64`); `arm` does not say `v6` or `v7`. The call changes nothing and
also shows that the runtime answers, so a host that offers work only while its runtime answers can call it each round.

Treat every error but `Unsupported` as "not here this round": an unreachable Docker is `DaemonUnavailable`, but one
that answers and is not healthy (an engine that is starting, a Docker below version 25) is `Other`. The call has no
time limit of its own and bollard gives up after 120 seconds, so wrap it in `tokio::time::timeout` for a fast answer.

## Runtime calls

Every call is safe to repeat, so a turn replayed after a crash can call the runtime again without harm:

| Call | What it does |
| --- | --- |
| `create(&spec)` | Creates the container `spec.name`, pulling a missing image; keeps an existing container of that name without comparing it |
| `copy_in(name, files)` | Writes files into a created container; see [Copy files in](#copy-files-in) |
| `make_dirs(name, dirs)` | Makes missing folders in a created container; see [Make folders](#make-folders) |
| `start(name)` | Starts a created container; a running or ended one is left as it is, so a container runs at most once |
| `inspect(name)` | `Missing`, `Created`, `Running`, `Exited { code }` or `Failed { code, reason }` |
| `logs(name, tail_lines)` | The last lines as one string, for a short report |
| `logs_since(name, cursor, max_lines)` | One page of timed lines; see [Follow a log](#follow-a-log) |
| `copy_out(name, path, limits)` | The files at or below a path; see [Copy files out](#copy-files-out) |
| `stop(name, grace)` | Stops a running container; a created one stays `Created`, a missing one is `Ok` |
| `remove(name)` | Removes it by force, with its anonymous volumes; a missing one is `Ok` |
| `list(&label)` | The containers with that label whose status it can report |
| `ensure_network(&spec)`, `ensure_volume(&spec)` | Makes a named network or volume unless it exists |
| `sweep(&label)` | Removes the containers, then networks, then volumes with that label, best effort |
| `info()` | The machine; see [The runtime's machine](#the-runtimes-machine) |

`ContainerStatus::Exited` is the program's exit; a stopped container is `Exited` too. `Failed` is the runtime's
failure, such as a dead container or one killed for lack of memory (the reason `OUT_OF_MEMORY`). On Docker, `stop`
sends the image's stop signal as `docker stop` does: exit code 143 after SIGTERM, 130 after SIGINT (`postgres`), 137
when the grace period runs out.

`ContainerSpec::new(name, image)` has public fields. Put secrets in `secret_env`: its `Debug` shows only the names,
and Docker gets the values only in the body of the create call. The hardening fields (`read_only_root`,
`drop_all_capabilities`, `no_new_privileges`, `tmpfs`, `init`, `pids_limit`) are off by default. A memory limit allows
no swap. A `tmpfs` is `noexec` unless its `exec` is set.

### Errors

A `ContainerError` has a `kind` and a message (`<action> <name>: <Docker's message>` on Docker). Only
`DaemonUnavailable` is worth a retry (`is_retryable()`).

| `ErrorKind` | Meaning |
| --- | --- |
| `DaemonUnavailable` | No connection or a timeout |
| `Conflict` | The container is in the wrong state or missing for this call, or a path holds a file where a folder is asked |
| `ImageNotFound` | The registry has no such image; Docker Hub and `registry:2` say so for a refused login too |
| `InvalidSpec` | The request is refused before anything is done, such as a bad path or limit |
| `Unsupported` | The runtime does not implement this call, or cannot do it for this container, such as the logs of a container whose log driver keeps none. A retry cannot help |
| `Other` | Anything else, such as a daemon error |

`ErrorKind` is `non_exhaustive`: give a match a wildcard arm.

### Copy files in

Copy files only into a created container, before `start`. A path must be absolute, without a `..` part, and name a
file; `/work/./a` is `/work/a`. Every refusal is `InvalidSpec` and writes none of the files:

- onto a volume or bind mount target (`/work` when a volume sits there), as a file cannot replace a mount point;
  write below it instead;
- at or below a tmpfs with no deeper volume or bind mount, as the tmpfs is mounted at start, over the file;
- on a read-only mount, and with `read_only_root`, anywhere but below a writable volume or bind mount.

Files are owned by root with the mode of each `CopyFile`. `CopyFile`'s `Debug` shows its path, mode and length, never
its contents.

### Make folders

A program that does not run as root cannot write to a folder root made, or to one that does not exist. Before `start`,
`make_dirs(name, dirs)` makes the folders it needs. `CopyDir::new(path)` gives mode 0o755; set `mode` on the binding
(`let mut out = CopyDir::new("/out"); out.mode = 0o777;`) to let any user write.

Only what is missing is made: a missing named folder with its mode, each missing parent with 0o755, owned by root. An
existing folder keeps its owner and mode, also when you name it with another mode: `/tmp` keeps 1777. So calling it
again makes nothing. A symbolic link at a named path or a parent counts as the folder it points to (in `postgres:18`,
`/var/run/app` makes `/run/app`); the walk of one folder follows at most 8 links. Every check runs before anything is
made:

- `Conflict`: a file, or a link to anything but a folder, at a named path or a parent; more than 8 links; a container
  that is not `Created`;
- `InvalidSpec`: a mode above 0o7777, one path named with two modes, and every path `copy_in` refuses;
- `Other` on Docker: a link loop, and Docker's other refusals.

You cannot set the mode of a volume's own root, as a mount target is refused: make a folder inside the volume, or set
the mode in the image.

### Follow a log

To keep a run's output outside the container while it runs, call `logs_since(name, cursor, max_lines)` with the cursor
of the page before, `None` the first time. A `LogPage` holds up to `max_lines` lines (1 to `LOG_PAGE_MAX`, 1000), each
a `LogLine` with its `time`, its `stream` (`Out` or `Err`) and its `text` as lossy UTF-8 without the newline, and
`next`, the cursor for the next call. Paging to the end gives every line once, in log order.

1. Load the cursor you stored, or start with `None`.
2. Read a page and store its lines and its `next` in one write of your own table, so a restarted follower goes on
   where it stopped. Keep the cursor as a string (`Display` and `FromStr`, or serde); never build or parse it.
3. A page with fewer than `max_lines` lines has read everything there is now. Wait a few seconds and read again; an
   empty page keeps the cursor.
4. Once `inspect` shows the container ended, read once more and stop after that short page, so no last line is lost.

A missing container is `Conflict`, a created one gives an empty page, an out-of-range `max_lines` or a string that is
not a cursor is `InvalidSpec`. A log the runtime can never read (Docker's log driver `none`) is `Unsupported` from
0.22.0, and `logs` gives the same kind: stop following then, as no retry will read it. Retry the other kinds, counting
errors in a row. On Docker each read scans
the log from the cursor's second, so poll every few seconds, not in a tight loop. A line over 16 KiB comes back in
parts. A line stamped before the cursor's second but written after it is missed, log rotation of that second shifts
the cursor, and a cursor after January 2038 is `Other`.

### Copy files out

`copy_out(name, path, limits)` returns the regular files at or below `path` of a created, running or ended container,
until `remove`. Collect a run's artifacts after its end and before you send `Remove`. Each `CopyFile` is whole, with
its absolute path, contents and mode, ready for `copy_in` of another container.

`CopyLimits::new(max_files, max_bytes)` bounds memory whatever the folder holds. A file larger than the bytes left goes
into `skipped` with its size, and the copy goes on; a file is never cut. `max_files` bounds `files` and `skipped`
together: the next entry past it stops the copy with one last `SkippedFile` for `path` ("the rest was not read").
Links are never followed: symbolic and hard links, devices, pipes and entries outside `path` are skipped with a reason.

A relative path, a `..` part, `/` or a zero limit is `InvalidSpec`; a missing container is `Conflict`; a missing path
gives an empty result. An error during Docker's stream fails the whole call. Mind these limits:

- Files on a tmpfs are gone once the container ended: put artifacts on the container's own file system or a volume.
- A file read while it is written can come back short. A program that updates a progress file writes a temporary
  name in the same folder and renames it into place; read it with small limits, such as `CopyLimits::new(1, 64 << 10)`.
- Dropping the future does not stop Docker's read; the archive is read to its end on a blocking thread.

### List and sweep

`list` leaves out a container removed while it lists, and on Docker one whose state Docker does not report; a missing
name in `list` is no proof the container is gone, so `inspect` it. `Swept` names what this sweep removed, per kind,
and what it skipped with Docker's reason (a network with endpoints, a volume in use). Something another caller removed
first is in no list, so two racing sweeps never both name it.

## The container actor

The container actor reconciles a container toward a goal ([ADR 0057](../adr/0057-a-reconciled-container-actor.md)): a
message only records the goal, and one armed poll at a time asks the runtime what is true and takes one step. The
container's name is the settings' prefix plus the actor's key, so a step that runs again finds what it made. The
[design guide](design.md#reconcile-an-outside-resource) explains the pattern.

### Define a job

A `ContainerJob` names the actor type, as a flow does, and builds the spec from a small typed request:

- `const TYPE` and `type Request` (serde and `JsonSchema`; its schema is part of the contract, so the HTTP door checks
  it).
- `spec(name, request)` returns the `ContainerSpec`. The actor sets the spec's name to `name`.
- `inputs(name, request)`, optional, returns `Inputs { dirs, files }`: folders and files the container gets between
  create and start, so a file can go into a fresh folder. `Inputs` is `non_exhaustive`: start from
  `Inputs::default()`.
- `timeout(request, settings)`, optional, returns the request's time limit; the default returns the settings'
  `timeout`, and `None` is no limit.
- `const MAX_TURN: Option<Duration>`, optional (from 0.22.0), is the turn limit of its actors; `None` keeps the node's
  default. Set it above the two long turns: the create step (a pull of a missing image, the create, the folders and
  the copies) and a stop, which waits up to `stop_grace`. It wins over `--max-turn` and `CICADA_MAX_TURN`, so an
  operator cannot lower it, and zero is refused at registration. It is not part of the contract.

`spec` and `inputs` are sync and run at each create; their results are never stored. **Put secrets there, never in the
request**: the request is in the state. Put secret files on the container's own file system, not on a volume, as
`Remove` deletes the container's files while a named volume outlives it.

The rustdoc of `ContainerJob` and `ContainerJob::timeout` in
[`src/actor/mod.rs`](../../crates/cicada-containers/src/actor/mod.rs) has tested examples.

### Host and start

1. Host `ContainerActor::new(job, runtime, settings)`, with the runtime as `Arc<dyn ContainerRuntime>` and
   `ContainerSettings::new("ci-")?`, beside the target type.
2. Create each actor `Container::<Job>::named(key)` with `ContainerState::new(request)`.
3. Send `ContainerMsg::start(&target, "Ended")?`. The target's family takes `Ended(ContainerEnded)`.
4. Later send `ContainerMsg::Stop` to stop it early, and `ContainerMsg::Remove` to remove the container.

`Start`, `Stop` and `Remove` only move the goal forward (run, then stop, then remove). `Check` brings back a lost poll;
a repeated goal does the same. The settings are `non_exhaustive`, with public fields to set on the binding:

| Field | Default | Meaning |
| --- | --- | --- |
| `prefix` | (given) | Starts with a letter or digit; only `A-Za-z0-9_.-` |
| `poll_interval` | 2 s | Time between polls of a running container |
| `max_backoff` | 60 s | Longest wait after runtime errors |
| `max_errors` | 10 | Runtime errors in a row before the container ends `failed` |
| `timeout` | none | How long a container may run before it is stopped and ends `timedOut` |
| `stop_grace` | 10 s | The grace period of a stop |
| `log_lines`, `log_bytes` | 200, 16 KiB | The tail of the log kept in the end |

The deadline is asked once, when the container starts, and saved; it never moves when settings or code change. Each
running container costs one turn per `poll_interval`.

### The end

A container ends once: `exited`, `failed`, `timedOut` or `stopped`. The `ContainerEnd` holds the `outcome`, the exit
`code`, a `reason`, the tail of the `logs` and, for `failed`, a `kind`. It is saved and sent to the first `Start`'s
target in the commit that records it, before any removal. An end the target's type refuses (not registered, or not
taking the message) is saved as `undelivered`, and the actor goes on.

| `FailureKind` | When |
| --- | --- |
| `notStarted` | The key cannot name a container; `spec` or `inputs` gave an `Err`; an input path is bad, or a file is at or above a folder; the runtime refused the spec, image, start or inputs; the runtime has no `make_dirs` |
| `outOfMemory` | The runtime reported `Failed` with the reason `OUT_OF_MEMORY` |
| `runtimeEnded` | The runtime reported another failure of a started container, such as a dead one |
| `missing` | A started container was gone |
| `runtimeErrors` | `max_errors` runtime errors in a row |

Match `FailureKind` with a wildcard arm, as it is `non_exhaustive`. A `failed` end saved before 0.21.0 has no kind.

**Treat a repeated end as one.** A `Start` sent after the end sends the saved end again to that `Start`'s target, so a
target can get the same end more than once. Keep the first end per key.

Runtime errors never fail a turn: they count, and the next poll waits `min(poll_interval × 2^errors, max_backoff)`. A
start refused with `InvalidSpec` ends the container as `notStarted` at once; another refused start shows on Docker as
`Failed` and ends it `runtimeEnded`. A log read the runtime answers with `Unsupported` does not count: the end is
recorded at once with the logs `(no logs: …)` and the runtime's message. A container that ended after its create
stays until `Remove`.

A replayed create step first inspects the container. One past `Created`, started by a step whose commit was lost, is
adopted with a fresh deadline, without calling `spec`, `inputs`, `create`, the copies or `start`. One still `Created`
gets its inputs again, which is safe.

### Rules for hosts

- **Never reuse a key**, also after a delete: a new actor under an old key adopts the old container and reports its
  old end.
- **Send `Remove` before you delete the actor**, or the container stays. Use the spec's labels and `sweep` for
  leftovers.
- **Keep the target until the end.** A target deleted before it makes the poll's commit fail, which blocks the actor
  until the target exists again. A target whose type is not registered does not block: the end is `undelivered`.
- **Set the job's `MAX_TURN` above an image pull and the stop grace**, which run inside one turn; without it the
  node's default applies. A step past the limit ends `failed` (`timeout`) and does not count as a runtime error: the
  poll's retry policy runs it again up to 7 times (1 to 32 s apart), finding by name what the cut-off step made. Then
  the container waits for the next `Check`, `Start`, `Stop` or `Remove`.
- **Every node that can claim the actors must reach the same Docker.** See
  [Operations](../operations.md#containers-and-docker-placement).
- **Stop before start creates nothing.** A `Stop` before any `Start` makes the container `stopped`; a later `Start`
  gets that end.

## Write or wrap a runtime

Implement `ContainerRuntime` for another engine. `logs_since`, `copy_out`, `make_dirs` and `info` have defaults that
fail with `Unsupported`, so a runtime written for an older version compiles unchanged; the container actor ends a
container `notStarted` only when `make_dirs` is unsupported. Report a container killed for lack of memory as
`ContainerStatus::Failed` with the reason `OUT_OF_MEMORY` exactly, or the actor cannot report `outOfMemory`.

**A wrapper must forward every method.** A runtime that wraps `DockerRuntime` to add logging, metrics or a pin must
forward `info`, `logs_since`, `copy_out` and `make_dirs` too; otherwise the trait's defaults answer `Unsupported`.

## Test with the fake

`FakeRuntime` is an in-memory runtime without timers, so it runs under paused time with
[`TestActors`](core.md#testing-with-testactors). Clones share one state, and it refuses the same paths and pages
the same way as the Docker backend.

| Method | Effect |
| --- | --- |
| `exit_with(name, status)` | Ends a container with that status |
| `set_logs(name, text)` | Replaces its log with `Out` lines at the Unix epoch |
| `push_log(name, LogLine::new(time, stream, text))` | Adds one timed line |
| `put_file(name, CopyFile)` | Adds a file as if the program wrote it |
| `set_info(RuntimeInfo)` | Changes the machine; `amd64` on `linux` before |
| `fail_next(Method, kind)`, `fail_always(Method, kind)`, `clear_failures()` | Scripts failures per call |
| `calls()` | Every call, as `Call` values |

The fake has no links and holds only regular files. On an ended container it leaves out files whose deepest mount is
a tmpfs, as Docker does. See [`tests/actor.rs`](../../crates/cicada-containers/tests/actor.rs) for a node with the
fake, a job and a target.
