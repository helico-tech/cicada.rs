# Docs handoff for tray-menu-mockup

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

The tray's menu follows the owner's mockup (`openspec/changes/tray-menu-mockup/mockup/`, boards Main, Disconnected,
Disconnecting, Running and Registered), with the eight departures its design lists.

- **Rows.** Running turns, contexts, recent turns and the connection failure are rows with a tinted circle icon, a
  title, a second line and, on the right, a badge: the elapsed time (m:ss) of a running turn, the finish time (HH:MM)
  of a recent turn. On macOS 14.0 to 14.3 the second line joins the title with " · ". Text too long for the menu's
  width ends in "…" instead of widening the menu.
- **Connected.** The header shows "<app> worker" over "Connected to <context>"; the last item is "Quit <app>". "Running" with "N of M" (running turns of the worker's
  concurrency), each turn as its actor over "<message type> · attempt N". "Context": each context's name over its
  server's host (host and port only). Registered Types with "K of T" (types the worker runs of those registered),
  Recent Turns with "N failed" when any failed. Then Open Inspector… (with the `inspector` feature), Settings…, Quit
  <app>.
- **Connecting.** "Connecting to <context>…", the contexts and the submenus, no Running section yet.
- **Not connected.** "Not connected" (with no contexts at all, "Not connected · Run cicada config"). After a failed
  connection, a row "Couldn't reach <context>" over "<cause> at HH:MM". The cause is the innermost cause of the
  error, such as "Connection refused (os error 61)", cut at 200 characters. A cause that might hold an address or
  credentials (one with `@`, `:/`, `:\`, or `http:`, `https:`, `ws:` or `wss:` in any case) is not shown: the row
  says "Connection failed at HH:MM" instead. Picking a context now selects it (the blue circle) without connecting,
  also after a failure; "Connect to <context>" (⌘K) and the switch connect to the selected one. The pick starts from the config file's `current-context`, lasts while the app runs and that context exists, and is never
  written to the config file. No Running section, no submenus. A turn a stop left behind (Stop Now while its release
  is still in flight) stays listed under "Finishing" until it ends.
- **Disconnecting.** "Disconnecting from <context>…", the switch dimmed. Once the worker was ready, a bar from the
  disconnect to the tray's deadline (the grace period plus the cancel timeout) with "Taking no new turns" and "N s
  left" to that deadline: the worker starts its grace period only once its polling loop stopped, which the tray cannot
  see. After Stop Now, with a grace period of zero, or past the deadline, the bar is full and says "Cancelling
  turns…". "Finishing" with the count and the running turns,
  Stop Now with the help "Stopping now cancels these turns. Their messages stay and run again later.", Quit. No
  contexts, submenus, Inspector or Settings. A stop before the worker was ready shows no bar and no Stop Now.
- **Registered Types.** "Actor Types": a checkmark for each type the worker runs, its name, and "N running" or "Not on
  this worker" once the worker is ready; "Background Tasks" with "Not available over HTTP"; Choose Types… opens
  Settings.
- **Recent Turns.** "Finished on this worker", newest first: done ("<type> · done in m:ss"), failed ("<type> ·
  <error's first line>"), retried after a failure ("<type> · <error>, retry HH:MM" or "retry now"), cancelled and
  abandoned; "Kept until the app quits" at the end. "N failed" counts failed, retried and abandoned turns. An error
  line that might hold an address or credentials (the check of the failure row) shows as "failed" ("failed, retry
  HH:MM" for a retried turn).
- **Status item.** A hexagon with a dot: filled while connected, hollow and dimmed otherwise (also while connecting
  and disconnecting).

Public items: `cicada_app::Contexts::host(name) -> Option<&str>`, the host of the context's server URL with its port
when the URL names one (`127.0.0.1:8080`, also a default port such as `actors.internal:443`), never its user, password,
path or query; `cicada_app::WorkerReady::
concurrency`, the most turns the started worker runs at once, from its config.

Tests: `crates/cicada-app/src/config.rs` (`contexts_keep_only_the_host_and_port_of_each_server`),
`crates/cicada-app/src/node.rs` (`worker_ready_runs_once_after_a_retry_and_registration_with_the_settled_settings`
checks `concurrency`), `crates/cicada-tray/src/menu.rs` (one test per board and state, per recent outcome, the pick,
the bar, the turn left behind while offline), `crates/cicada-tray/src/tests.rs` (`a_failed_connection_records_when_it_failed`, the concurrency and Stop
Now data). The AppKit menu has a manual check list next to the boards.

## 1. The menu bar app's menu

**Where.** `docs/guide/app.md`, `Menu bar app on macOS`, the paragraph that starts "The app starts offline."
**Text.** Replace it with the substance above, shorter: the switch connects to the selected context (the config
file's current context until you pick another; picking one while not connected selects it, ⌘K or the switch
connects); picking another context while connected drains the current worker first; each context shows its server's
host; Running shows "N of M" and each turn's actor, message type, attempt and elapsed time; after a failed connection
the menu shows the error and when; Registered Types shows each type's state and Choose Types… opens Settings; Recent
Turns shows the last 50 finished turns and their outcomes, with "N failed" on its row; while the worker drains, the
header shows the grace period's bar and Stop Now ends it. Quit, Ctrl-C and logout drain the worker before the app
exits.
**Caveats.** The bar's "N s left" counts to the end of the cancel timeout, not of the grace period alone. The pick
is not saved: `cicada config use-context` still sets the CLI's default, and the tray starts from
it again at the next launch. The text on the right of a row is a native badge. On macOS 14.0 to 14.3 the second line
of each row joins its title (the guide's existing bullet about the running turn's details in its title should say
"each row's second line").

## 2. `Contexts::host`

**Where.** `docs/guide/app.md`, `Menu bar app on macOS`, the sentence "(`Contexts::load` in code)".
**Text.** "`Contexts::host(name)` gives the host and port of a context's server for display, never its credentials,
path or query."
**Caveats.** None.

## Blocking

None.

## Status

Ran `cargo test -p cicada-examples --test docs` (11 passed) and `--doc` (55 passed) without changes to protected
prose. All text above is proposed; no protected file was edited.
