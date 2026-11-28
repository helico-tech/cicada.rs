# Run containers

`cicada-containers` is an optional crate for applications that run containers. It has two levels:

- **The runtime**, `ContainerRuntime`: calls on containers, networks and volumes, found by names you choose. Call it
  from host code or a task of its own, for work such as following logs or collecting files.
- **The container actor**, `ContainerActor`: one durable actor per container, which creates, starts, watches, stops
  and removes it, and reports its end once to a target you name.

No other Cicada crate depends on it ([ADR 0056](../adr/0056-shared-external-resource-crates.md)). Add it from the
same tag as the other crates ([install](../how-to/install.md#choose-the-crates)): `features = ["docker"]` for
`DockerRuntime`, and `features = ["testing"]` in dev-dependencies for `FakeRuntime`.

The crate's rustdoc has more examples (generate it with `cargo doc -p cicada-containers --all-features --no-deps`),
and [`tests/actor.rs`](../../crates/cicada-containers/tests/actor.rs) tests the container actor end to end.

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
host's Tokio runtime. For a setup `connect()` does not take, such as an endpoint the host knows, use `connect_to`:

<!-- doctest
@@5
# Ok::<(), Box<dyn std::error::Error>>(())
-->
```rust,no_run
use cicada_containers::DockerRuntime;

let local = DockerRuntime::connect()?; // as the docker CLI finds it
let remote = DockerRuntime::connect_to("tcp://build-host:2375")?; // an endpoint you give
let service = DockerRuntime::connect()?.docker_config("/srv/ci/.docker"); // the logins of a service user
```

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

From 0.25.0, `RuntimeInfo` also has `id`: Docker's daemon id from `/info`, the same for every node that talks to that
daemon, or `None` when the runtime gives none. Use it as a placement member's
[resource key](actors.md#placement-pool), so two nodes on one daemon do not offer its places twice. Docker does not
promise the id's format: compare it, never parse it. Fetching it is a second call, to `/info`, which can take seconds on
a busy daemon. The fake has no id until `set_info(RuntimeInfo::new("arm64", "linux").with_id("daemon-1"))`.

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

**Isolate a network from the host.** Set both `NetworkSpec::internal` and `NetworkSpec::isolated` (from 0.26.0) when
containers must not reach the Docker host and its published ports through this bridge. Docker 28 or later (API 1.48)
then leaves the bridge without a host address, with its IPv4 and IPv6 gateway modes set to `isolated`; the option is off
by default. `ensure_network` returns `InvalidSpec` for `isolated` without `internal`, `Unsupported` when the Docker API
is too old, and `Conflict` when either flag differs on an existing network: Docker cannot change them in place, so
remove the old network first.

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

<!-- doctest
# #[tokio::main(flavor = "current_thread", start_paused = true)]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
@@28
# use cicada_containers::{ContainerSpec, FakeRuntime};
# let fake = FakeRuntime::new();
# fake.create(&ContainerSpec::new("ci-run-7", "rust:1")).await?;
# fake.start("ci-run-7").await?;
# fake.set_logs("ci-run-7", "compiling\ntest result: ok\n");
# fake.exit_with("ci-run-7", ContainerStatus::Exited { code: 0 });
# let lines = follow(&fake, "ci-run-7", None).await?;
# assert_eq!(lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["compiling", "test result: ok"]);
# Ok(())
# }
-->
```rust
use std::time::Duration;

use cicada_containers::{ContainerError, ContainerRuntime, ContainerStatus, LogCursor, LogLine};

/// Follows the container `name` to its end from `cursor`, `None` the first time, and returns its lines.
async fn follow(
    runtime: &dyn ContainerRuntime,
    name: &str,
    mut cursor: Option<LogCursor>,
) -> Result<Vec<LogLine>, ContainerError> {
    let mut lines = Vec::new();
    let mut ended = false;
    loop {
        let page = runtime.logs_since(name, cursor, 500).await?;
        let short = page.lines.len() < 500;
        lines.extend(page.lines); // a host stores these and `page.next` in one write of its own
        cursor = Some(page.next);
        if short && ended {
            return Ok(lines); // a short page read after the end: every line is in
        }
        if short {
            ended = !matches!(runtime.inspect(name).await?, ContainerStatus::Created | ContainerStatus::Running);
            if !ended {
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }
    }
}
```

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
into `skipped` with its size, and the copy goes on, though its bytes still count as read; a file is never cut.
`max_files` bounds `files` and `skipped` together: the next entry past it stops the copy with one last `SkippedFile` for
`path` ("the rest was not read"). From 0.23.1, `max_read` bounds what a copy reads: by default four times `max_bytes`,
it counts the content bytes of every regular file, kept or skipped, as Docker sends a skipped file's data all the same.
A file that would take the count past it stops the copy before its data, with one last `SkippedFile` for `path` ("the
rest was not read: over the read limit of N bytes"); the files before it are kept. Change it with
`CopyLimits::new(1000, 64 << 20).with_max_read(1 << 30)`. So a huge file early in a folder ends the copy there, and the
smaller files after it are not kept, which 0.23.0 still kept; the order of entries is Docker's.
Links are never followed: symbolic and hard links, devices, pipes and entries outside `path` are skipped with a reason.

A relative path, a `..` part, `/` or a zero limit, `max_read` included, is `InvalidSpec`; a missing container is `Conflict`; a missing path
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
- `outputs(request)`, optional (from 0.23.0), names the output folder the actor keeps at the end; see
  [Outputs at the end](#outputs-at-the-end).
- `const MAX_TURN: Option<Duration>`, optional (from 0.22.0), is the turn limit of its actors; `None` keeps the node's
  default. Set it above the two long turns: the create step (a pull of a missing image, the create, the folders and
  the copies) and a stop, which waits up to `stop_grace`; with [outputs](#outputs-at-the-end), the copy at the end is a
  third. It wins over `--max-turn` and `CICADA_MAX_TURN`, so an
  operator cannot lower it, and zero is refused at registration. It is not part of the contract.

`spec` and `inputs` are sync and run at each create; their results are never stored. **Put secrets there, never in the
request**: the request is in the state. Put secret files on the container's own file system, not on a volume, as
`Remove` deletes the container's files while a named volume outlives it.

A job that runs a commit's tests, gives the program a writable `/out` and the commit as a file, and lets a slow pull
fit one turn:

```rust
use std::time::Duration;

use cicada::prelude::*;
use cicada_containers::{ContainerJob, ContainerSpec, CopyDir, CopyFile, Inputs, Outputs};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct Test {
    /// The commit to test.
    commit: String,
}

/// Holds what builds the specs, such as a token; never stored.
struct Tests {
    token: String,
}

impl ContainerJob for Tests {
    const TYPE: &'static str = "ci.tests";
    const MAX_TURN: Option<Duration> = Some(Duration::from_secs(35 * 60)); // a slow pull fits one turn
    type Request = Test;

    fn spec(&self, name: &str, test: &Test) -> Result<ContainerSpec, String> {
        let mut spec = ContainerSpec::new(name, "rust:1");
        spec.command = Some(vec!["sh".into(), "-c".into(), "cargo test > /out/report.txt".into()]);
        spec.env.insert("COMMIT".into(), test.commit.clone());
        spec.secret_env.insert("CARGO_REGISTRY_TOKEN".into(), self.token.clone());
        Ok(spec)
    }

    fn inputs(&self, _name: &str, test: &Test) -> Result<Inputs, String> {
        let mut out = CopyDir::new("/out");
        out.mode = 0o777; // the program does not run as root
        let mut inputs = Inputs::default();
        inputs.dirs.push(out);
        inputs.files.push(CopyFile::new("/in/commit.txt", test.commit.clone()));
        Ok(inputs)
    }

    fn outputs(&self, _test: &Test) -> Option<Outputs> {
        Some(Outputs::new("/out")) // kept in the actor's store at the end: 64 MiB in 1000 files, 256 MiB read
    }
}
```

### Host and start

1. Host `ContainerActor::new(job, runtime, settings)`, with the runtime as `Arc<dyn ContainerRuntime>` and
   `ContainerSettings::new("ci-")?`, beside the target type.
2. Create each actor `Container::<Job>::named(key)` with `ContainerState::new(request)`.
3. Send `ContainerMsg::start(&target, "Ended")?`. The target's family takes `Ended(ContainerEnded)`. Or send
   `ContainerMsg::start_with_progress(&target, "Ended", "Progress")?` to get [progress](#progress) too; the family then
   also takes `Progress(ContainerProgress)`.
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

### Progress

A job's program can report its progress while it runs (from 0.23.0,
[ADR 0065](../adr/0065-container-progress-through-a-file.md)). Ask for it in the `Start`; the owner's family takes the
progress under its own variant:

<!-- doctest
# use cicada::prelude::*;
# use serde::{Deserialize, Serialize};
# #[derive(Debug, Clone, Serialize, Deserialize)]
# struct Ci { run: String }
# impl ActorRef for Ci { const TYPE: &'static str = "ci"; type Message = CiMsg; }
@@11
# assert_eq!(serde_json::to_value(&start)?["progressType"], "TestProgress");
# Ok::<(), Box<dyn std::error::Error>>(())
-->
```rust
use cicada_containers::{ContainerEnded, ContainerMsg, ContainerProgress};

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
enum CiMsg {
    TestEnded(ContainerEnded),
    TestProgress(ContainerProgress), // { key, seq, text }
}

let owner = Ci { run: "run-7".into() };
let start = ContainerMsg::start_with_progress(&owner, "TestEnded", "TestProgress")?;
```

- **The file.** The actor sets `CICADA_PROGRESS_FILE` in the container's environment to the job's `PROGRESS_FILE`
  (default `/tmp/cicada-progress`; override the constant in your `impl ContainerJob`), after the job's `spec`, which
  cannot change or hide it. The program writes its progress there in any text format; the actor does not parse it. A
  `PROGRESS_FILE` that is relative, has a `..` part or names no file ends the container `notStarted`.
- **Write it atomically.** Write a temporary name in the same folder, then rename it onto the path; a file read while
  it is written can come back half written:

  ```sh
  printf '{"done": %d, "total": %d}' "$done" "$total" > "$CICADA_PROGRESS_FILE.tmp" && mv "$CICADA_PROGRESS_FILE.tmp" "$CICADA_PROGRESS_FILE"
  ```

- **Not on a tmpfs.** The actor reads the file with `copy_out`, which cannot see a tmpfs. With a read-only root and a
  tmpfs on `/tmp`, set `PROGRESS_FILE` to a path on a volume.
- **When it is read.** At each poll of the running container, and once in the turn that records an end for which the
  container ran (`exited`, `stopped`, `timedOut`, and `failed` with `runtimeEnded` or `outOfMemory`). Each running
  container with progress costs one `copy_out` per poll.
- **What is sent.** When the text changed since the last progress sent, the owner gets a `ContainerProgress
  { key, seq, text }`, with `seq` counting from 1. At most 64 KiB is read, as UTF-8 with each invalid sequence and NUL
  replaced by U+FFFD. An empty file is sent only after a progress that was not empty. The state keeps only `seq` and
  a hash, not the text.
- **Best effort.** A missing file, one over 64 KiB or not a regular file, and a failed read send nothing and are not
  runtime errors: progress never ends a container. A progress the target's type refuses is not retried and does not
  hold up the end.
- **Order.** The last progress goes out in the end's commit, before the end. A progress the owner fails to handle is
  retried after a backoff, so it can arrive after a later one or after the end. **Drop a progress whose `seq` is not
  above the last one you handled, and any that arrives after the end.**

### Outputs at the end

An actor with an [output store](#keep-outputs) keeps what a container leaves behind, the rest of its log and the job's
output folder, and the owner learns with the end what was kept (from 0.23.0,
[ADR 0066](../adr/0066-container-outputs-at-the-end.md)). Give the actor the store with `with_store`, and the job an
`outputs` folder, as `Tests` does in [Define a job](#define-a-job):

<!-- doctest
# use cicada::prelude::*;
# use cicada_containers::{ContainerJob, ContainerSpec};
# use serde::{Deserialize, Serialize};
# #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
# struct Test {}
# struct Tests;
# impl ContainerJob for Tests {
#     const TYPE: &'static str = "ci.tests";
#     type Request = Test;
#     fn spec(&self, name: &str, _: &Test) -> Result<ContainerSpec, String> { Ok(ContainerSpec::new(name, "rust:1")) }
# }
@@7
# Ok::<(), Box<dyn std::error::Error>>(())
-->
```rust
use std::sync::Arc;

use cicada_containers::{ContainerActor, ContainerSettings, FakeRuntime, MemoryOutputStore, OutputStore};

let store: Arc<dyn OutputStore> = Arc::new(MemoryOutputStore::new()); // your own store in production
let runtime = Arc::new(FakeRuntime::new());
let actor = ContainerActor::new(Tests, runtime, ContainerSettings::new("ci-")?).with_store(store.clone());
```

- **What happens.** At an end the container ran for (`exited`, `stopped`, `timedOut`, and `failed` with
  `runtimeEnded` or `outOfMemory`), the actor holds the end as `pending` in the phase `collecting`. Its next polls
  store the rest of the log (at most 10,000 lines per turn, the next poll at once while more is left), then copy the
  outputs folder out within its limits and put each file into the store, keyed by the container's name. The poll that
  finishes reads the last progress, records the end with `outputs` and sends it. A saved end still means a sent end.
- **The report.** The end's `outputs` is an `OutputsReport`:
  `{"stored": 12, "bytes": 40960, "skipped": 1, "firstSkipped": [{"path": "/out/core", "reason": "…"}], "error": null}`.
  `skipped` counts every entry the limits or the link rule of `copy_out` left out and every file the store refused,
  each with its reason; `firstSkipped` lists the first 10.
- **Without an outputs folder** only the log is stored, and the counts are 0. **Without a store** nothing changes:
  the end is recorded and sent in one commit, without `outputs`. Ends the container did not run for (`notStarted`,
  `missing`, `runtimeErrors`) never collect.
- **Errors.** A runtime or store error a retry can cure counts as a runtime error and backs off; at `max_errors` in a
  row the end goes out with its outcome unchanged and `error: "(no outputs: failed N times in a row: …)"`, with a
  warning in the log. A collecting turn past the job's `MAX_TURN` commits nothing and is not a runtime error, but
  after 3 failed tries of the same poll, timeouts included, the next try makes no copy and sends the end with
  `error: "(no outputs: the collection failed 3 times; last: …)"` and the counts of what the store already holds; so a
  copy that always takes too long still ends, after about three turn limits (from 0.23.1). A relative
  outputs path, a runtime without `copy_out` and a container removed by someone else end the collection at once with
  the error; a runtime without `logs_since` skips the log. What was stored before an error stays stored.
- **Messages meanwhile.** A second `Start` changes nothing: the end goes to the first target. A `Stop` stops nothing
  more. A `Remove` removes the container after the end is sent.
- **Next to a `LogArchiver`.** Give both the same store: the guarded append stores each line once, and the actor
  stores the lines the archiver had not read when the container ended.
- **Turn limit and memory.** The copy runs in one turn: set the job's `MAX_TURN` above the time a copy of the read
  limit takes (`max_read`, four times the byte limit by default; `Outputs::new` reads at most 256 MiB). A collecting container holds up to its byte limit in memory: size the limits against the node's
  concurrency. Each actor collects in its own turns, so a slow copy holds up only its own container.

The store is yours: the actor deletes stored outputs only with a [retention](#retention).

### Retention

With a store and `ContainerSettings::output_retention`, the actor deletes a container's stored log and files a set
time after the container's removal (from 0.23.0). The default, `None`, keeps them until you delete them:

<!-- doctest
# use cicada::prelude::*;
# use cicada_containers::{ContainerJob, ContainerSpec};
# use serde::{Deserialize, Serialize};
# #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
# struct Test {}
# struct Tests;
# impl ContainerJob for Tests {
#     const TYPE: &'static str = "ci.tests";
#     type Request = Test;
#     fn spec(&self, name: &str, _: &Test) -> Result<ContainerSpec, String> { Ok(ContainerSpec::new(name, "rust:1")) }
# }
# let store: std::sync::Arc<dyn cicada_containers::OutputStore> = std::sync::Arc::new(cicada_containers::MemoryOutputStore::new());
# let runtime = std::sync::Arc::new(cicada_containers::FakeRuntime::new());
@@7
# Ok::<(), Box<dyn std::error::Error>>(())
-->
```rust
use std::time::Duration;

use cicada_containers::{ContainerActor, ContainerSettings};

let mut settings = ContainerSettings::new("ci-")?;
settings.output_retention = Some(Duration::from_secs(7 * 24 * 3600)); // keep a week after the removal
let actor = ContainerActor::new(Tests, runtime, settings).with_store(store);
```

- **When.** The clock starts at the container's removal (`Remove`), not at its end, so a container its owner keeps
  keeps its outputs. The removal saves the due time in the state (`outputsUntil`), and a poll then calls
  `OutputStore::remove` for the container's name. Zero deletes in the poll right after the removal. The container is
  removed from the runtime first, so a `LogArchiver` does not store its log again.
- **The saved time decides.** A retention changed or unset after the removal does not move it; a restart keeps it.
  `Check` brings back a lost poll at the saved time, or at once when that passed.
- **Errors.** A store that is `Unavailable` or fails with `Other` is tried again with the backoff, without limit; when
  the errors in a row reach `max_errors`, the actor logs one warning. A
  name the store refuses (`Invalid`), and a node without a store at the due time, clear the due time with a warning in
  the log; the outputs then stay.
- **What stays.** The actor's own state, with its end, stays after the deletion.

Mind these limits:

- An actor deleted before its due time leaves its outputs: call `OutputStore::remove` with the container's name
  (prefix plus key) when you delete it. The actor does not list outputs whose actor is gone; list your own store.
- A container removed before 0.23.0, or by an actor without a store or a retention, gets no due time.
- The name is computed from the settings' prefix at the due time: do not change the prefix while outputs wait.

### Rules for hosts

- **Keep the prefix label.** The actor adds the label `cicada.container.prefix` with its prefix to every spec, after
  `spec`; a job's own labels stay, and a job that sets this label is overruled. The log archiver finds containers by
  it.
- **Never reuse a key**, also after a delete: a new actor under an old key adopts the old container and reports its
  old end.
- **Send `Remove` before you delete the actor**, or the container stays. Use the spec's labels and `sweep` for
  leftovers. With a store, also remove its stored outputs, or wait for its [retention](#retention) first.
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

## Keep outputs

A host keeps each container's log lines, log cursor and output files outside the container, in a store it
implements: `OutputStore` (from 0.22.1), keyed by the container's name, so one store serves every job, such as a
folder per run or a database table. Use it as `Arc<dyn OutputStore>`. A [`LogArchiver`](#archive-the-logs) fills it
with the logs of a job's containers.

| Method | What it does |
| --- | --- |
| `log_end(container)` | The stored cursor (`None` when nothing is stored) and the number of stored lines; a writer resumes from the cursor |
| `append_logs(container, after, page)` | Stores a `LogPage`'s lines and its `next` cursor in one write; see below |
| `read_logs(container, from, limit)` | Up to `limit` `StoredLine`s from line number `from` (0-based, stored order); none past the end |
| `put_file(container, file)` | Stores a `CopyFile`, as `copy_out` gives them, replacing one at the same path |
| `files(container)` | The path, size and mode of each stored file, in path order |
| `read_file(container, path)` | The file, or none |
| `remove(container)` | Deletes lines, cursor and files; afterwards the cursor is `None` again. Nothing stored is `Ok` |

This follows a container's log into the in-memory store of the tests, `MemoryOutputStore`, and checks the store's
rules:

<!-- doctest
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
@@17
# Ok(())
# }
-->
```rust
use cicada_containers::{
    ContainerRuntime, ContainerSpec, FakeRuntime, MemoryOutputStore, OutputErrorKind, OutputStore, check_output_store,
};

let runtime = FakeRuntime::new();
runtime.create(&ContainerSpec::new("job-1", "alpine:3")).await?;
runtime.set_logs("job-1", "hello\nworld");

let store = MemoryOutputStore::new();
let end = store.log_end("job-1").await?;
let page = runtime.logs_since("job-1", end.cursor, 500).await?;
assert_eq!(store.append_logs("job-1", end.cursor.as_ref(), &page).await?, 2);
let again = store.append_logs("job-1", end.cursor.as_ref(), &page).await.unwrap_err();
assert_eq!(again.kind, OutputErrorKind::Conflict);
assert_eq!(store.read_logs("job-1", 1, 10).await?[0].line.text, "world");

check_output_store(&store).await; // the rules of the trait hold
```

### Archive the logs

You do not write the log pages yourself: a `LogArchiver` follows the log of every container of a container job and
keeps it in your `OutputStore`, so the log survives the container's removal and your route can serve it with
`read_logs`. It runs as a task of its own and writes to the store, not to actor state. Run it as an
[app task](app.md#run-it-as-a-binary), with the runtime you give the container actor and the actor's prefix:

<!-- doctest
@@13
-->
```rust,no_run
use std::sync::Arc;

use cicada_app::App;
use cicada_containers::{ArchiveSettings, ContainerRuntime, DockerRuntime, LogArchiver, OutputStore};

fn app(store: Arc<dyn OutputStore>) -> Result<App, Box<dyn std::error::Error>> {
    let runtime: Arc<dyn ContainerRuntime> = Arc::new(DockerRuntime::connect()?); // also given to ContainerActor
    let archiver = LogArchiver::new(runtime, store, "ci-", ArchiveSettings::new())?;
    Ok(App::new().task("logs", move |_system, stopping| async move {
        archiver.run(async move { stopping.wait().await }).await;
        Ok(())
    }))
}
```

A host without `cicada-app` spawns `archiver.run(stop)` on Tokio itself. On the fake, with a stop after one second:

<!-- doctest
# #[tokio::main(flavor = "current_thread", start_paused = true)]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
@@18
# Ok(())
# }
-->
```rust
use std::sync::Arc;
use std::time::Duration;

use cicada_containers::{
    ArchiveSettings, ContainerRuntime, ContainerSpec, FakeRuntime, LogArchiver, MemoryOutputStore, OutputStore,
    PREFIX_LABEL,
};

let runtime = FakeRuntime::new();
let mut spec = ContainerSpec::new("ci-1", "alpine:3");
spec.labels.insert(PREFIX_LABEL.into(), "ci-".into()); // as the container actor does
runtime.create(&spec).await?;
runtime.set_logs("ci-1", "hello\nworld");

let store = MemoryOutputStore::new();
let archiver = LogArchiver::new(Arc::new(runtime), Arc::new(store.clone()), "ci-", ArchiveSettings::new())?;
archiver.run(tokio::time::sleep(Duration::from_secs(1))).await;
assert_eq!(store.read_logs("ci-1", 0, 10).await?[1].line.text, "world");
```

- **Which containers.** The container actor labels every container it creates with `cicada.container.prefix`
  (`PREFIX_LABEL`) and its prefix. The archiver lists the containers with that label, so give it the same prefix as
  the job's `ContainerSettings`.
- **Rounds that never wait.** Every `interval` (2 s) a round lists the containers and starts a read for each one that
  is neither drained nor still being read, up to `max_parallel` (16) at once, without waiting for them: a slow log
  holds up only itself. A read appends each page of `logs_since` (`page_lines`, 500 lines) from the stored cursor. A
  running container is read again each round; an ended one is drained once, then skipped.
- **Once and in order.** Every line is stored once and in order, also after a restart, which reads on from the stored
  cursor, and with two archivers on one store, where the second writer of a page gets `Conflict` and reads again.
- **Errors never end it.** Only `stop` ends `run`. A log that is `Unsupported` (Docker's log driver `none`) is logged
  once and not read again; a container that is gone is dropped; a name the store refuses (`Invalid`) skips the
  container. Every other error, and a failed `list`, is tried again in a later round; from the fifth failure in a row
  the wait doubles, up to 60 s. Errors are logged through `tracing`.
- **One per Docker daemon.** Each archiver repeats every log scan of its daemon, and Docker scans a log from the
  cursor's second on each read. Run one archiver per daemon, not one per node; two stay correct but cost twice.

`ArchiveSettings::new()` gives the defaults, with public fields to change; `LogArchiver::new` refuses a `page_lines`
outside 1 to `LOG_PAGE_MAX`, a zero `interval` and a zero `max_parallel` with `InvalidArchive`. Mind these limits:

- A container made before 0.22.1 has no label and is not archived, also when the actor adopts it.
- Lines a container writes after the last read are lost when the container is removed before the next round.
- Do not `remove` a container's outputs while the container exists: its cursor would be `None` again, and the archiver
  would store the log from the start once more.

### One write for lines and cursor

`append_logs` stores the lines and the cursor together or not at all, and only when the stored cursor equals `after`
(`None` for a container with nothing stored). Otherwise it fails with `Conflict` and stores nothing. So a writer that
crashed after a write reads `log_end` again and goes on without a repeated or lost line, and two writers of one
container cannot interleave: the second gets `Conflict`. An empty page stores its cursor alone. Implement it as one
transaction (a database) or one atomic replace (a folder). No test can prove atomicity across a crash or the guard
under writers in parallel on several nodes: your store must promise both.

### Names and paths

A container name and a file path can come from a user, such as through your HTTP route, and a folder store turns them
into file paths. Every method checks them first and fails with `Invalid` before it touches anything:
`check_container_name` (a letter or digit first, then only `A-Za-z0-9_.-`: no `/`, never `..`) and `clean_output_path`
(absolute, no `..` part, not `/`; empty and `.` parts dropped, so `/a`, `//a` and `/./a` are one file, `/a`). Call both
in your store and use the cleaned path as the key; `files` and `read_file` give the cleaned path, not the one put.
From 0.23.0, a container name over 200 characters and a path with a control character (NUL included) are `Invalid`
too. A store on a case-insensitive file system (the defaults of macOS and Windows) must keep `/out/Job` and `/out/job`,
and the containers `Job` and `job`, apart.

`OutputError` has a `kind`: `Invalid` (a name or path outside the rules), `Conflict` (a stale `after`), `Unavailable`
(the store could not be reached; the only kind `is_retryable` calls retryable) and `Other`.

### Test a store

`MemoryOutputStore` (feature `testing`) is an in-memory store; clones share their outputs. `check_output_store(&store)`
runs every rule against any store and panics naming the rule it broke, so you check your own store the way Cicada
checks its store backends. Call it in a `#[tokio::test]`. It uses container names of its own
(`cicada-output-check-a` and `-b`) and removes them before and after, so it can run on a store that holds other data.
It does not check atomicity after a crash, the guard under parallel writers, or a file at `/a` next to one at `/a/b`.

## Find containers without their actor

A container that carries a job's prefix label but has no container actor (left by a lost store, a test, an actor
deleted by hand) runs on unnoticed. `OrphanWatch` (from 0.23.1) lists the job's containers every `interval` (5 minutes)
and reads the actor of each one, `J::TYPE` keyed by the name without the prefix. A container whose actor is missing in
two rounds in a row is reported once: a warning in the log and one call of your `on_orphan` with its name. It never
removes a container: decide with `sweep` or by hand.

<!-- doctest
# use std::sync::{Arc, Mutex};
# use std::time::Duration;
# use cicada::prelude::*;
# use cicada_containers::{ContainerJob, ContainerSpec};
# use serde::{Deserialize, Serialize};
# #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
# struct Test {}
# struct Tests;
# impl ContainerJob for Tests {
#     const TYPE: &'static str = "ci.tests";
#     type Request = Test;
#     fn spec(&self, name: &str, _: &Test) -> Result<ContainerSpec, String> { Ok(ContainerSpec::new(name, "rust:1")) }
# }
# #[tokio::main(flavor = "current_thread", start_paused = true)]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
@@17
# Ok(())
# }
-->
```rust
use cicada_containers::{ContainerRuntime, FakeRuntime, OrphanSettings, OrphanWatch, PREFIX_LABEL};
use cicada_test::TestActors;

let runtime = FakeRuntime::new();
let mut spec = ContainerSpec::new("ci-left", "rust:1");
spec.labels.insert(PREFIX_LABEL.into(), "ci-".into()); // as the container actor does
runtime.create(&spec).await?; // but no actor ci.tests/left exists

let orphans = Arc::new(Mutex::new(Vec::new()));
let seen = orphans.clone();
let watch = OrphanWatch::<Tests>::new(Arc::new(runtime.clone()), "ci-", OrphanSettings::new(), move |name: &str| {
    seen.lock().unwrap().push(name.to_owned()); // or your own alert
})?;
let test = TestActors::builder().build().await?;
watch.run(test.system().clone(), tokio::time::sleep(Duration::from_secs(6 * 60))).await;
assert_eq!(*orphans.lock().unwrap(), ["ci-left"]); // at the second round, 5 minutes in
assert!(runtime.inspect("ci-left").await.is_ok()); // and not removed
```

In a host, run it as an [app task](app.md#run-it-as-a-binary) like the [log archiver](#archive-the-logs), with the node's
`ActorSystem` and `stopping.wait()` as the stop, once per Docker daemon. `on_orphan` is a plain function: to send a
message, spawn a task or push into a channel from it. `OrphanWatch::new` refuses a zero `interval`. Mind these limits:

- What it reported is kept in memory: after a restart each orphan is reported once more. An orphan whose actor
  appears is dropped, and reported again only after its actor is gone for two rounds again; a removed one is forgotten.
- A failed `list` or actor read is logged and does not count as a round.
- Two stores on one Docker daemon with the same prefix (staging and production) see each other's containers as
  orphans: give each its own prefix.
- Containers made before 0.22.1 carry no label and are never seen. A labelled name without the prefix is skipped with
  one warning.

## Write or wrap a runtime

Implement `ContainerRuntime` for another engine. `logs_since`, `copy_out`, `make_dirs` and `info` have defaults that
fail with `Unsupported`, so a runtime written for an older version compiles unchanged; the container actor ends a
container `notStarted` only when `make_dirs` is unsupported. Report a container killed for lack of memory as
`ContainerStatus::Failed` with the reason `OUT_OF_MEMORY` exactly, or the actor cannot report `outOfMemory`. Make
`ensure_network` honour `NetworkSpec::isolated` or return `Unsupported`: a runtime written before it may ignore the flag
and create a weaker network.

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

This test hosts the container actor on the fake, with a target that keeps the ends, and drives one container to its
exit on virtual time (the `Tests` job of [Define a job](#define-a-job)):

<!-- doctest
# use std::time::Duration;
# use cicada::prelude::*;
# use cicada_containers::{ContainerEnded, ContainerJob, ContainerSpec};
# use serde::{Deserialize, Serialize};
# #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
# struct Test { commit: String }
# struct Tests { token: String }
# impl ContainerJob for Tests {
#     const TYPE: &'static str = "ci.tests";
#     type Request = Test;
#     fn spec(&self, name: &str, _: &Test) -> Result<ContainerSpec, String> { Ok(ContainerSpec::new(name, "rust:1")) }
# }
# #[derive(Debug, Clone, Serialize, Deserialize)]
# struct Ci;
# impl ActorRef for Ci { const TYPE: &'static str = "ci"; type Message = CiMsg; }
# #[derive(Debug, Serialize, Deserialize, JsonSchema)]
# #[serde(tag = "type")]
# enum CiMsg { Ended(ContainerEnded) }
# #[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
# struct Ends { ends: Vec<ContainerEnded> }
# struct CiActor;
# impl Actor for CiActor {
#     type Ref = Ci;
#     type State = Ends;
#     async fn handle(&self, turn: &mut Turn<Self>, message: CiMsg) -> HandlerResult {
#         let CiMsg::Ended(ended) = message;
#         turn.state_mut().ends.push(ended);
#         Ok(())
#     }
# }
# #[tokio::main(flavor = "current_thread", start_paused = true)]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
@@25
# Ok(())
# }
-->
```rust
use std::sync::Arc;

use cicada_containers::{
    Container, ContainerActor, ContainerMsg, ContainerRuntime, ContainerSettings, ContainerState, ContainerStatus,
    FakeRuntime, Outcome,
};
use cicada_test::TestActors;

let fake = FakeRuntime::new();
let runtime = Arc::new(fake.clone());
let settings = ContainerSettings::new("ci-")?;
let job = Tests { token: "t0ken".into() };
let test = TestActors::builder().with(move |node| node.host(ContainerActor::new(job, runtime, settings)).host(CiActor));
let test = test.build().await?;
test.create(&Ci, &Ends::default()).await;

let tests = Container::<Tests>::named("run-7");
test.create(&tests, &ContainerState::new(Test { commit: "abc123".into() })).await;
test.send(&tests, ContainerMsg::start(&Ci, "Ended")?).await;
test.run_until_idle().await;
assert_eq!(fake.inspect("ci-run-7").await?, ContainerStatus::Running);

fake.exit_with("ci-run-7", ContainerStatus::Exited { code: 0 });
test.advance_time_by(Duration::from_secs(2)).await;
assert_eq!(test.state::<_, Ends>(&Ci).await.ends[0].end.outcome, Outcome::Exited);
```

The fake has no links and holds only regular files. On an ended container it leaves out files whose deepest mount is
a tmpfs, as Docker does. See [`tests/actor.rs`](../../crates/cicada-containers/tests/actor.rs) for a node with the
fake, a job and a target.
