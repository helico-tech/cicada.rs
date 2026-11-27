# App: run and operate a binary

Use `cicada-app` when you want actor hosting, HTTP and operator commands in one binary.
Enable `sqlite` or `postgres` for a store and `tui` for the terminal UI; none is enabled by default.

## Run it as a binary

Host your actor and call `run()`. This example uses the actor from [sync.rs](../../examples/src/sync.rs);
your application hosts its own actor from the [core tutorial](core.md#define-an-actor-type).

```rust,no_run
use cicada_app::App;
use cicada_examples::sync::ReportsActor;

fn main() -> std::process::ExitCode {
    App::new().host(ReportsActor).run()
}
```

| Command | Behavior |
| --- | --- |
| `myapp serve --store actors.db` | Run actors and the HTTP API at `127.0.0.1:8080` |
| `myapp serve --store actors.db --types none` | Serve the API without running actors |
| `myapp worker --store actors.db --types reports` | Run selected actors; serve no HTTP |
| `myapp worker --server https://actors.internal` | Run through the HTTP API, with no database access |
| `myapp get actors --server http://127.0.0.1:8080` | Run an operator command against a server |

`--types` accepts `all` (default), comma-separated type names, or `none` for serve only.
A binary without hosted types, including the standalone `cicada`, has neither serve nor worker.

**Authentication.** App supplies none. Before binding beyond loopback, use `App::http` to wrap the API in your
middleware. Apply the layer after merging routes it must protect. `/ready` is public outside that router;
a host route named `/ready` cannot replace it. See [HTTP composition](http.md#run-it-in-an-axum-service) and the
[sample’s middleware](../../sample-service/src/lib.rs). `serve --token` is rejected; a client token does not protect a server.

**Lifecycle.** SIGTERM/SIGINT stops claims, drains turns within the grace period, then cancels them. Serve keeps HTTP
answering during the node stop, with `/ready` returning 503; afterward it drains HTTP for up to 5 s and closes the
store. Worker serves no `/ready`. See [deployment](../operations.md#deploying) for platform stop timeouts. At the stop,
background tasks see `Stopping` as claims stop and get the same grace period as the turns; one still running then is
aborted with a warning. Serve keeps answering HTTP until the node and the tasks have stopped, and closes the store
after both. A task's error after the stop is a warning, not a failure.

**Host hooks.** `on_start` runs after the local node starts and before serve listens; use it to create required actors.
Errors stop the node. `observe`, `node_config` and `version` configure the host. Use `run_async` from an async main;
`run_with` supplies arguments, environment, output and stop handling for tests. Avoid expensive setup for operator
commands by checking `cicada_app::node_command` before building dependencies.

**Background tasks.** Work that runs next to the node, such as a loop that takes jobs from outside, belongs in
`App::task` (from 0.22.0, [ADR 0064](../adr/0064-background-tasks-of-an-app.md)), not in a task spawned from
`on_start`: nothing watches that one, so a crash leaves the node running with `/ready` at 200. A task starts in `serve`
and in a worker on the store, after every `on_start`, in the order of the calls, and before serve listens. It gets
the node's `ActorSystem` and a `Stopping`, and must run until the stop: watch `stopping.wait()` and return when it
completes.

```rust,no_run
use std::time::Duration;

use cicada_app::App;
use cicada_examples::sync::ReportsActor;

fn main() -> std::process::ExitCode {
    App::new()
        .host(ReportsActor)
        .task("runner", |system, stopping| async move {
            loop {
                tokio::select! {
                    () = stopping.wait() => return Ok(()), // the stop began: end in good order
                    () = tokio::time::sleep(Duration::from_secs(5)) => {
                        let _ = system.now().await?; // take outside work here, using the node
                    }
                }
            }
        })
        .run()
}
```

A task that returns, fails or panics before the stop stops the node in good order and fails the command with exit
code 1 and `task <name> ended before the stop`, so the process supervisor restarts it; there are no restarts inside
the process. Two tasks with one name refuse `serve` and `worker` with exit code 2; operator commands run no task. A
task that ignores `Stopping` is aborted at the end of the grace period; one that never yields cannot be aborted.

### Settings

Flags override non-blank environment variables. For node settings, variables override the host’s `NodeConfig`, then
library defaults. Durations accept `250ms`, `5s`, `7d`, or ISO-8601 (`PT5S`).
`CICADA_NODE_LABEL` is read as given: empty is unset, whitespace-only is an error, and outer spaces are kept.
An invalid label from a flag or variable is a usage error; an invalid label in the host's `NodeConfig` fails startup.

| Flag | Variable | Default | Meaning |
| --- | --- | --- | --- |
| `--store` | `CICADA_STORE` | required, but for a worker over HTTP | `postgres://…` or `postgresql://…` (PostgreSQL), `sqlite:FILE`, or the path of a SQLite file. Another value with `://`, or one that starts with `postgres:`, is refused and never printed, as it may hold a password |
| `--store-schema` | `CICADA_STORE_SCHEMA` | `public` | the schema, on PostgreSQL |
| `--store-pool-size` | `CICADA_STORE_POOL_SIZE` | the larger of 20 and the concurrency plus 4 | the connections of this node, on PostgreSQL; a smaller value is used as given, and the node then warns at start |
| `--retention` | `CICADA_RETENTION` | 7 days | how long outcomes are kept, also the window in which a message id is known (S10); set one value on every node |
| `--listen` | `CICADA_LISTEN` | `127.0.0.1:8080` | the address `serve` listens on |
| `--types` | `CICADA_TYPES` | `all` | the types of the node: `all`, type names with commas, or (`serve` only) `none`; `all` and `none` are keywords only as the whole value |

| Flag | Variable | `NodeConfig` field |
| --- | --- | --- |
| `--node-id` | `CICADA_NODE_ID` | `node_id` |
| `--node-label` | `CICADA_NODE_LABEL` | `label` |
| `--concurrency` | `CICADA_CONCURRENCY` | `concurrency` |
| `--lease-ttl` | `CICADA_LEASE_TTL` | `lease_ttl` |
| `--heartbeat-interval` | `CICADA_HEARTBEAT_INTERVAL` | `heartbeat_interval` |
| `--poll-interval` | `CICADA_POLL_INTERVAL` | `poll_interval` |
| `--shutdown-grace-period` | `CICADA_SHUTDOWN_GRACE_PERIOD` | `shutdown_grace_period` |
| `--max-turn` | `CICADA_MAX_TURN` | `default_max_turn` |
| `--max-lost-claims` | `CICADA_MAX_LOST_CLAIMS` | `max_lost_claims`: a number, or `off` |

See [configuration defaults](core.md#configuration) and [generated help](../reference/cli.md) for exact options.
On an unavailable PostgreSQL database, startup retries a second after each attempt, starting no new attempt after
one minute. Individual attempts can make the total longer. Logs use `tracing` on stdout, with `RUST_LOG`
(default `info,sqlx=warn`).

**Node config from the hosted types.** Use `App::node_config_for` (from 0.25.0) when a node setting depends on the
types selected by `--types` or `CICADA_TYPES`, such as loading a placement identity only on runs that host the runner.
It gets the names in the order of the `App::host` calls, and an empty slice for a serve-only run with `--types none`.
It runs once for `serve` or `worker`, after the local startup checks and before waiting for the store or the server;
operator commands, help and version skip it. A returned error fails the command with exit code 1. The last call to
either `node_config` or `node_config_for` sets the host config; flags and variables still win over its fields, and an
HTTP worker uses the same subset of fields as with `node_config`.

<!-- doctest
@@19
-->
```rust,no_run
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use cicada::runtime::NodeConfig;
use cicada_actors::NodeIdentity;
use cicada_app::App;

fn with_runner_identity(app: App, dir: PathBuf, identity_slot: Arc<OnceLock<NodeIdentity>>) -> App {
    app.node_config_for(move |types| {
        if types.contains(&"factory.runner") {
            let identity = NodeIdentity::load_or_create(&dir)?;
            let config = NodeConfig { node_id: identity.node_id(), ..NodeConfig::default() };
            identity_slot.set(identity).map_err(|_| anyhow::anyhow!("runner identity was already set"))?;
            Ok(config)
        } else {
            Ok(NodeConfig::default())
        }
    })
}
```

Share the slot with the placement member's task of that run, and use a fresh slot for each run when a process builds a
fresh `App` to run again, so the identity is released and the next run can lock the folder. With `--node-id other`
the callback still loads and locks the identity when the runner is selected, and the node uses `other`.

Exit codes: **0** success or requested stop, **2** invalid usage/environment, **1** operational/startup failure.

### A worker over HTTP

Use the same App without `on_start` hooks or tasks:

```sh
myapp worker --server https://actors.internal --token "$TOKEN"
myapp worker --context prod --types reports
```

| Selection | Result |
| --- | --- |
| Explicit `--store` | Local store |
| Explicit `--server` or `--context` | HTTP |
| Both explicit modes | Usage error |
| No explicit mode, non-blank `CICADA_STORE` | Store, even with operator server/context variables |
| Otherwise `CICADA_SERVER` or `CICADA_CONTEXT` | HTTP |
| Only token or config’s current context | No mode selected; usage error |

A worker on the store reads neither the operator config nor `CICADA_TOKEN`; `--token` is invalid there.
For HTTP mode, resolve server/context/token as for [operator commands](#command-line-and-terminal-ui).

The server owns TTL, heartbeat, claim limit, retention and store configuration: their flags are rejected and their
variables ignored. Worker id, concurrency, poll interval, maximum turn and grace period still apply.
An HTTP worker has no node row: `--node-label` is refused, while `CICADA_NODE_LABEL` and the host's label are ignored.
`on_start` and `App::task` are refused before any request (exit code 2); bootstrap and run tasks through serve or a
local-store worker instead.

The HTTP client disables redirects and retries of its own. Requests have a 30 s total timeout, with a 10 s turn-call
timeout. Startup probes `GET /types` while unavailable, with no new probe after a minute, then registers contracts
once. A refusal or conflict fails startup. The worker needs access to type reads/registration and every lease route.

SIGTERM/SIGINT drains the worker and leaves the source running. A fatal lease error stops it with exit 1.
It has no node row or readiness endpoint. The [HTTP guide](http.md#run-a-worker-over-http) covers uncertainty and limits;
[remote_worker.rs](../../examples/src/remote_worker.rs) shows library-level construction.

**When the worker is ready.** `App::on_worker_ready` (from 0.25.0) calls the host once the worker over HTTP has
checked the server and registered its contracts. It gets a `WorkerReady` with the settled `grace_period` and
`cancel_timeout`, and from 0.26.0 `types`, the types selected by `--types` or `CICADA_TYPES` in host order
(`App::type_names()` lists every type the app hosts), and `stop_now`, a `StopNow` handle. From 0.26.0 `WorkerReady` is
`#[non_exhaustive]`: destructure it with `..` and get it only from the callback. Only a worker over HTTP reports it; `serve` and a worker on the store keep
their signal-driven stop.

<!-- doctest
@@13
# let _ = app;
-->
```rust,no_run
use std::sync::{Arc, Mutex};

use cicada_app::{App, StopNow};
use cicada_examples::sync::ReportsActor;

let stop_now: Arc<Mutex<Option<StopNow>>> = Arc::default();
let keep = stop_now.clone();
let app = App::new().host(ReportsActor).on_worker_ready(move |ready| {
    println!("running {:?} with a grace period of {:?}", ready.types, ready.grace_period);
    *keep.lock().unwrap() = Some(ready.stop_now);
});
// Later, when a user asks to stop at once:
if let Some(stop_now) = stop_now.lock().unwrap().as_ref() { stop_now.now() }
```

`stop_now.now()` ends the grace period of the worker's stop, so the stop ends within the cancel timeout. The running
turns are cancelled, and their leases are released where the worker still holds them and can, and otherwise expire; a
turn whose handler had already ended may still complete. A call that comes before the worker has begun stopping, even
before the stop signal, is kept and applies as soon as the stop begins. It never stops the worker by itself.

### A menu bar worker

`cicada-tray` (from 0.25.0, [ADR 0074](../adr/0074-a-tray-crate-next-to-the-app.md)) runs a worker over HTTP in the
host process and lets a menu bar app control it. Build a `Tray` with `Tray::new(|| App::new().host(MyActor))`: it calls
the factory again for every connection it starts. On a Tokio runtime, `connect("production")` runs an HTTP worker
against the named operator context, with the process environment by default; `with_env` passes an explicit map, such
as one that sets `CICADA_CONFIG` to a chosen config file.

<!-- doctest
# async fn example() {
@@10
# }
-->
```rust,no_run
use cicada_app::App;
use cicada_examples::sync::ReportsActor;
use cicada_tray::{Tray, TrayState};

let tray = Tray::new(|| App::new().host(ReportsActor));
tray.connect("production");
let mut state = tray.state();
while state.changed().await.is_ok() {
    if let TrayState::Failed { error, .. } = &*state.borrow() { eprintln!("worker failed: {error}") }
}
```

- **State.** `state()` moves from `Offline` through `Connecting` to `Online`, even without work. `Failed` carries the
  worker command's error, also when the worker task panics or is cancelled; connect again from there. `disconnect()`
  stops claiming and gives running turns their grace period: `Stopping` shows the stop time plus the grace period and
  cancel timeout, and a successful stop returns to `Offline`. Do not call `connect`, `disconnect` or `stop_now` while
  holding a borrow of `state()`.
- **Stop now.** From 0.26.0, `stop_now()` works while the tray stops a worker that was ready: it ends the grace period,
  as `StopNow` does, and `Stopping`'s time drops to at most now plus the cancel timeout. A turn that ends without an
  acknowledged commit is listed as abandoned; one whose handler had already ended may still commit. A turn the worker
  leaves behind, such as one whose release is still in flight at the cancel timeout, stays listed as running after
  `Offline` until it ends. In any other state it does nothing.
- **Turns.** `turns()` gives the running turns and the last 50 finished ones of this tray; watch `turn_changes()` to
  redraw. A running turn shows its actor, message type, attempt and claim time; a finished one adds its end time and
  outcome. `Abandoned` means the worker saw no commit succeed, such as when a stop cancelled its handler. The history
  lives in memory.
- The worker follows the retry and shutdown rules of the HTTP worker command, and an `App` with `on_start` or `task` is
  refused, as those need a local `ActorSystem`.

### Menu bar app on macOS

From 0.26.0, `Tray::run` drives a native menu bar app on macOS 14 or later, on the main thread: a plain `main` that
returns `std::process::ExitCode` calls `Tray::new(|| App::new().host(MyActor)).run()`. The worked example is
[menu-bar-worker.rs](../../examples/menu-bar-worker.rs), which hosts the checkout actor; run it with
`cargo run -p cicada-examples --example menu-bar-worker`. Configure and select a context with the
[command line](#command-line-and-terminal-ui) before opening the menu.

The app starts offline. Its switch connects to the config file's current context and disconnects gracefully; picking
another context drains the current worker before starting the next, and the menu reloads the context names each time
it opens (`Contexts::load` in code). Running shows each turn's actor, message type, attempt and time since claim;
Registered Types marks the types the worker runs; Recent Turns shows the last 50 finished turns, with a badge for failed
and abandoned ones. While the worker drains after a disconnect or a Quit, a Stop Now row under the header ends the
grace period, and the countdown drops to the cancel timeout. Quit, Ctrl-C and logout drain the worker before the app
exits.

**Settings.** From 0.26.0, Settings… (⌘, while the menu is open) opens a Settings window. Pick a context, or Add… a new
one (1 to 64 characters from `A-Z a-z 0-9 . _ -`), set its server and token, switch the types this worker runs (the
list scrolls past eight), and set the turns at once (1 to 1,000). Apply and Reconnect saves only what you changed and,
while the tray holds a connection, disconnects and connects to the applied context with the new settings; an offline
tray stays offline. Revert shows the saved values again, a validation error shows under its field and writes nothing,
and Esc or ⌘W closes the window without saving.

- The token is stored in the config file, as `cicada config` stores it, not in the Keychain. `CICADA_SERVER` and
  `CICADA_TOKEN` in the tray's environment win over the window's server and token, and the window says so.
- The connection is saved first, then the worker settings. When the second write fails, the window says "connection
  saved; worker settings not saved: <error>" (or "worker settings not saved: <error>" when only those changed), keeps
  your values and does not reconnect; Apply again once the cause is fixed.

**Open Inspector.** With the Cargo feature `inspector` of `cicada-tray` (from 0.26.0), Open Inspector… (⌘I while the
menu is open) opens the [inspector](#inspector-on-macos) in the tray's process, on the tray's context, also while it
connects or stops; offline, it starts on the target `cicada` would use. The inspector has its own context picker. A
second Open Inspector… brings the same window forward on the context it already shows; closing it and opening it again
starts from the tray's latest connection. When the inspector is the key window, ⌘R refreshes, ⌘W closes, and ⌘[ and
⌘] move through its history. Quit, Ctrl-C and logout close the inspector before the tray finishes quitting. Try it with
`cargo run -p cicada-examples --example menu-bar-worker --features cicada-tray/inspector`.

**`tray.toml`.** The tray keeps per-context worker settings in `tray.toml`, in the folder of the config path as given
(next to a symlink, not its target), with mode 0644 and no secrets:

```toml
[contexts.prod]
types = ["checkout"]
concurrency = 4
```

A context listed there connects with `--types` (its saved types that the binary still registers, or `all` when that is
every registered type) and `--concurrency`, which win over `CICADA_TYPES` and `CICADA_CONCURRENCY`. A context not
listed uses those variables and the defaults, and the window shows every type on with 16 turns at once, until an Apply
saves its entry. An app with no registered type, a saved selection with none of its types registered any more, or a
lone type named `all` or `none` among others ends the connection `Failed`, with an error that says to open Settings.
Two config files in one folder share one `tray.toml`.

**`ContextFile` for hosts.** `cicada_app::ContextFile` reads and writes the config file with the code of `cicada config`
(`Contexts` only reads the names and the current one). Use `ContextFile::update(&env, |file| { ...; Ok(()) })` for a
write that may run next to another: it re-reads the file under the lock and saves it when it changed.
`set_server_and_token` keeps the context's headers and leaves `current-context` alone; `set_context` replaces the whole
context and makes it current when none is.

- On macOS 14.0 to 14.3 the running turn's details go in its title.
- For a distributed `.app`, build the binary with the intended `MACOSX_DEPLOYMENT_TARGET`, put it in `Contents/MacOS`,
  give `Contents/Info.plist` its executable and bundle metadata, and set `LSUIElement` to true for a menu bar agent;
  the app uses the accessory policy, so it has no Dock icon. Signing and notarizing belong to your build.
- The factory is called at launch to list the registered types, and for each accepted connection.
- The grace period and cancel timeout can delay Quit and logout, and a second Ctrl-C does not force-quit while the
  worker drains. Stop Now appears only for a worker that was ready.

### Inspector on macOS

`cicada-inspector` (from 0.26.0, [ADR 0076](../adr/0076-a-native-macos-inspector.md)) opens a native window that
inspects a running system over the HTTP API. A small binary, or a `.app`, calls `cicada_inspector::Inspector::new().run()`
from a plain `main` that returns `std::process::ExitCode`, on the main thread; the crate's rustdoc has the example. It
owns the app until the window closes: a regular app with a Dock icon and a main menu with Quit (⌘Q), Close (⌘W),
Refresh (⌘R), Back (⌘[) and Forward (⌘]).

The sidebar lists Overview and the Browse items (Actors, Messages, Running Turns, Outcomes, Nodes, Types, Watches);
Overview and Actors have pages, and the others say "Coming soon". The Overview shows tiles for actors, pending
messages, the oldest due message and failed outcomes, the health, the nodes, the first 20 running turns and the
registered types with their actor counts. Refresh is by hand. A refresh counts the actors of at most 50 types, 8 at a
time, all within 10 seconds: a part whose call fails shows its error, one with no answer says "no answer within 10 s",
and a type past the 50 or not counted in time shows "not counted". A new refresh or a context switch cancels the one
before.

**Actors** lists the actors whose address starts with the text in the toolbar's search field, 100 at a time as the
table scrolls, with their type, version, created time and status. Typing empties the table at once and reads the new
prefix when you stop typing. The status says `running` when the actor is among the first 200 running turns under the
prefix, read once per reload, `paused` when its mailbox is paused, or both; the footer says when the running status
covers only the first 200 running turns, or that it is unknown. A page that fails shows its error with a Retry button.

The list is an outline: open a row with its disclosure triangle or the right arrow key to see its first 100 children,
choose **more…** for the next page of that parent's children, and **Retry** under the parent after a failed read.
Closing a row keeps its loaded pages. A child row shows its address, type and running status, where a dash means its
address is outside the running sample's prefix. An actor can appear in the root list and under a parent, and each row
folds and selects on its own.

Selecting an actor fills the pane on the right, read only: its type, version, created time, parent, pinned node and
paused flag, its running turn, and tabs for its state as highlighted JSON, its mailbox (the pending messages among its
first 50) and its watches. An actor that is gone shows "Not found". The toolbar's pane button hides and shows the pane
(in the standalone app also View › Hide Pane). ⌘R on the Actors page reloads the list from the top and the pane, and
reopens surviving rows as their pages load; a new search prefix or context starts with all rows folded.

- The running status is a sample: with more than 200 running turns under the prefix, an actor past them shows no
  `running` even when it runs. Search a narrower prefix to see it.
- The turn's message type and attempt come from a separate read of the mailbox: a turn reclaimed between the two reads
  can show an older attempt, and a message past the mailbox's first 50 shows no type.
- Watches lists only watches whose stored target is exactly this actor, not one on its type or on every type (`*`).
- A state longer than 256 KiB shows its first 256 KiB with a notice.
- Pages are not a snapshot: an actor created or deleted while you scroll can be missed or appear late, and a row that
  disappeared is forgotten once a page passes its address. Refresh reads again from the top.

The window starts on the context `cicada` would use, and its pop-up lists the config file's contexts; picking one makes
a new client and refreshes. A context without a usable server shows its error, and the window stays usable.

- macOS 14 or later; elsewhere `run` prints that and exits with code 1.
- To open it inside an app that already owns the main thread and a Tokio runtime, such as a menu bar app, create a
  `cicada_inspector::Embedded` with `new(mtm, env, handle)` and call `show(Some("prod"))`, or `show(None)` for the
  target `cicada` would use. Calling `show` again brings the open window forward and keeps its context; `close()`, or
  dropping the controller, closes the window and asks its refresh to stop, and late results are dropped. The tray's
  Open Inspector… is built on it.
- `CICADA_SERVER` and `CICADA_TOKEN` in its environment win over the picked context, as for `cicada`, and the window
  says so. A password in a server URL never reaches the window.
- The failed tile counts what `GET /outcomes` still lists, so what retention keeps; the window cannot show the
  retention itself. An older server without the count shows `—`.
- No `.app` bundle, signing or notarising is provided.

## Command line and terminal UI

Install the operator from the checkout:

```sh
cargo install --path crates/cicada-app --features tui
cicada config set-context local --server http://127.0.0.1:8080
cicada get actors
cicada tui
```

![Cicada TUI showing sync/acme, its persisted state, and a paused mailbox with three pending messages](../assets/tui.png)

The actor view keeps state, mailbox, outcomes and children together. This capture uses the sample service on
an isolated SQLite store; [capture instructions](../assets/README.md) explain how to refresh it.

All operator commands call HTTP. Omit `--features tui` to build without terminal libraries.
Use [generated CLI help](../reference/cli.md) for flags and command details.

| Task | Command |
| --- | --- |
| Inspect | `get actors`, `get actor ADDR`, `get mailbox ADDR`, `get message ID` |
| Inspect in full | `describe actor ADDR` (YAML); add `-o json` for JSON |
| Create/send | `create actor ADDR --state JSON`; `send ADDR --type TYPE --payload JSON` |
| Recover | `retry message ID`, `deliver-now ID`, `resume ADDR` |
| Control | `pause ADDR`, `purge ADDR`, `cancel turn ADDR`, `delete actor ADDR` |
| Diagnose | `get outcomes --status failed`, `get turns`, `get nodes`, `stats`, `health` |

Choose a pin when creating a new instance, for example:

```sh
cicada create actor reports/host-a --state '{}' --pinned-to host-a
cicada get actor reports/host-a -o wide
```

The wide table shows `PINNED-TO` after `PARENT`; `describe actor` and JSON show `pinnedTo`, and the TUI actor screen
shows `Pinned to`. An existing actor keeps its pin. A blank, over-200-character or NUL pin is a usage error (exit 2).
There is no pin filter: use the state listing and mailbox to [diagnose waiting pinned work](../operations.md#pinned-actors-that-wait).

A delete includes descendants. Retry creates a new message; purge/cancel/delete have distinct effects:
read [the incident guide](../operations.md#on-call) first. Listings show one page; `--all` traverses every page.
Missing items and API failures exit 1. Usage errors exit 2.

A usage error hides URL passwords: `postgres://u:pw@h/db` prints as `postgres://u:***@h/db`, including passwords
with a raw `'` and each address in a list. This covers only `scheme://user:password@host` in usage errors;
other output and libpq-style settings are not scrubbed. Percent-encode `/`, `?`, `#`, `"`, `` ` `` and spaces
in passwords, as URLs require.

### Server and credentials

Resolve each field independently:

| Field | Precedence |
| --- | --- |
| Server | `--server`, `CICADA_SERVER`, context server |
| Context | `--context`, `CICADA_CONTEXT`, config current context |
| Token | `--token`, `CICADA_TOKEN`, context token |
| Config path | `CICADA_CONFIG`, `$XDG_CONFIG_HOME/cicada/config.toml`, `~/.config/cicada/config.toml` |

Context headers/token still apply when its URL is overridden. A missing file is empty; an unreadable file or unknown
context fails. `cicada_app::context_client(&env, Some("prod"))` (from 0.26.0) gives an `ActorClient` for a context,
resolved exactly as the operator commands resolve their target and with their 30 s timeout, and the name of the context
it really uses (`None` when only `CICADA_SERVER` names a server); `None` as the context picks it as `cicada` does. It
checks the server URL first, so no error quotes it, and sends a user and password in the URL as Basic credentials,
which win over a token; the client's URL keeps neither. `config set-context` replaces a context; `config use-context`
switches it. Writes replace the file’s
comments, preserve symlinks and set Unix permissions to 0600.

From 0.26.0, `config set-context` and `config use-context` take a lock on `<file>.lock` next to the config file (next to
its target when it is a symlink) while they write, so they and the [menu bar app](#menu-bar-app-on-macos) keep each
other's changes; readers take no lock, and the lock covers one machine. On a file system without file locks, such as
some network mounts, the write fails rather than risk another writer's change: point `CICADA_CONFIG` at a local file.
An unreadable file is reported with its path and line, never its contents. A config path whose name, or whose symlink
target's name, is `tray.toml` or ends in `.lock`, in any letter case, is refused, and so is one that ends in `/` or
`/.`.

### Terminal keys

| Keys | What |
| --- | --- |
| `:actors`, `:messages`, `:outcomes [status]`, `:turns`, `:watches`, each with an optional prefix; `:nodes`, `:types` | A listing; Tab completes; Enter on a watch opens its watcher |
| `:actor ADDR`, `:mailbox ADDR`, `:message ID`, `:node ID`, `:type NAME` | One item |
| `enter`, `esc`, `q` | Open the selected row; clear the filter or go back; `q` on the first screen and `ctrl-c` quit |
| `j` `k` `↑` `↓`, `g` `G`, `pgup` `pgdn` | Move |
| `/`, `<` `>`, `s`, `w` | Filter rows as you type; sort column; sort order; wide columns |
| `tab` | The next tab of an overview (an actor's mailbox, outcomes and children) |
| `d` | Describe: the item as YAML; `t` switches to JSON, `w` wraps |
| `m`, `a`, `b`, `P` | An actor's mailbox; a message's actor; its cause; an actor's parent |
| `c`, `r`, `n` | Cancel, retry, deliver now a message |
| `p`, `u`, `x` | Pause, resume, purge an actor's mailbox |
| `ctrl-k`, `ctrl-d` | Stop the running turn; delete an actor or remove a type |
| `ctrl-r`, `?` | Refresh now; help |

Actions apply to the selected row, or the displayed item, after confirmation (default No).
The display refreshes every 2 s and the header every 10 s. Sending and bulk actions are CLI only.

## Sample service

The [sample service](../../sample-service/) hosts the worked examples and E2E actors:

```sh
cargo run -p cicada-sample-service -- serve --store actors.db
```

The default binary, `cicada-sample-service`, creates required singletons and the dead-letters watch for hosted
types. `GET /` and `/ready` are public. Set `API_TOKEN` to require a bearer token on the API.
Set `OTEL_EXPORTER_OTLP_ENDPOINT` for telemetry; the protocol must be unset or `http/protobuf`.
The image defaults to `0.0.0.0:8080`, `/data/actors.db`, and a 3 s shutdown grace period.
Legacy unprefixed variables are rejected; see the [0.13.0 upgrade](../how-to/upgrading.md).

Use the second binary, `cicada-sample-executor`, to run sample types through an existing API:

```sh
cargo run -p cicada-sample-service --bin cicada-sample-executor -- worker --server http://localhost:8080 --types e2e.counter
```

It has the same catalog, routes, API-token protection and full App command line, but no bootstrap in any mode:
it creates no sample singletons or wildcard dead-letter watch. A store-connected sample node hosting the needed
types, or an API caller, creates what the workload needs; registering contracts creates no actor. The original
binary still refuses `worker --server` because it has an `App::on_start` hook.

Both binaries also host the ready-made heartbeat watch, `cicada.heartbeat-watch`, selected by `--types` like any
other type. Hosting it creates no watch and sends no `Beat`: create `cicada.heartbeat-watch/<name>` with its
silence, target and alarm, make sure the target exists, and send `Beat` to start a round. See
[Heartbeat watch](actors.md#heartbeat-watch) for how it monitors handled Beats.

An HTTP worker sends `CICADA_TOKEN` (or `--token`); `API_TOKEN` protects a served API and is never the client token.
It has no listener, so its in-memory `EffectLog` cannot be read through `/e2e/effects`; inspect the actors' durable
state on the server. This path opens no store, although the sample package still builds both SQL features.
See [image deployment](../operations.md#deploying) for the entrypoint and health-check settings.

The E2E actors record `Turn::node_id()`: the node or worker that ran each turn, never the HTTP server's id for a
remote turn. App resolves `--node-id`, then `CICADA_NODE_ID`, then a random id. A differing flag now starts and wins
over the variable. The legacy node-id arguments of `SlowActor::new`, `CounterActor::new`, `RecorderActor::new`,
`Catalog::new` and `Catalog::with_fakes` remain accepted but are ignored.

## Running it

Continue with [operations](../operations.md) for sizing, deployment and incident diagnosis.
