# Changelog

Releases of Cicada for Rust. The crates share one version. They are used as git dependencies, so pin a tag (README,
Using a release in another project).

Each section is headed `## X.Y.Z — YYYY-MM-DD`; the next version is `## X.Y.Z — unreleased` until it is released. While
the version is 0.x, a breaking change raises the minor number and anything else the patch number. A published tag never
moves: a correction is a new version.

## 0.26.0 — unreleased

`Tray::run` opens a native menu bar app on macOS 14 or later. Use the switch or pick a configured context to
connect, watch running and recent turns, and let the worker drain when you disconnect or quit. While it drains, Stop
Now cancels its running turns at once instead of waiting out the grace period. Its Settings window picks or adds a
context, sets its server and token, and chooses the types the worker runs and how many turns run at once, kept per
context in `tray.toml` next to the config file.
`cicada_app::Contexts` reads the configured names and current selection for the menu, and `cicada_app::ContextFile`
reads and writes the config file with the code of `cicada config`. A new crate `cicada-inspector` (ADR 0076) opens a
native inspector window on macOS 14 or later with `Inspector::new().run()`: an Overview of the store's counts, the
health, the nodes, the running turns and the types with their actor counts, an Actors page that lists the actors
under an address prefix with a read-only pane of the selected actor, a Messages page of the pending messages with
Show Actor, an Outcomes page of the finished messages by status with their cause chain and the failed count in the
sidebar, a Types page with registered schemas, and a Watches page with a filter and Show Actor, for a context of
`cicada config`.
`cicada_app::context_client` makes the client for it. Enable the tray's `inspector` feature to open that window from
its menu.

**BREAKING** for `cicada_app::WorkerReady`: it reports the selected actor types in `types`, gains `stop_now`, loses
`Copy`, `PartialEq` and `Eq`, and is now non-exhaustive. Read its fields directly or match with `..`, and compare
fields instead of whole values. Use `clone()` when another owned value is needed. Only `App::on_worker_ready` supplies
the value. `cicada::store::StoreStats` and `cicada::api::StatsView` gain a `failed` field and become non-exhaustive.
Replace external struct literals with stats returned by `ActorSystem::stats` or `ActorClient::stats`. Read fields
directly or use `..` in matches.

### Added

- `cicada-inspector`: a new crate, a native macOS inspector over the HTTP API. ADR 0076 allows the tray's optional
  dependency and supersedes ADR 0011 in part for a native macOS console. `Inspector::new().run()` owns the main
  thread with a Dock icon and a main menu (Quit, Close ⌘W, Refresh ⌘R, Back ⌘[, Forward ⌘]) until the window closes;
  elsewhere, or before macOS 14, it prints an error and returns exit code 1. The window has a sidebar (Overview;
  Browse: Actors, Messages, Running Turns, Outcomes, Nodes, Types, Watches; those without a page say "Coming soon"), a
  toolbar with back and forward, the title, a context pop-up and Refresh, and the Overview: tiles for actors, pending,
  oldest due and failed (kept by retention; `—` from an older server), the health, the nodes, the first 20 running
  turns with a mark when there are more, and the types with their actor counts. A refresh is by hand and bounded: at
  most 50 types counted, 8 counts at a time, 10 s in all; a part that fails shows its error alone, and a type past
  the 50 or the deadline shows as not counted, and a count still queued at the deadline never starts. A refresh
  superseded by another or by a context switch is cancelled, also while in flight, and its result discarded.
  Covered by the tests in `crates/cicada-inspector/src/tests.rs`; the window has a manual check list.
- `cicada-inspector`: the Actors page. Its table lists the actors under the prefix typed in the toolbar's search field,
  100 at a time as it scrolls, by the server's cursor, with their address, type, version, created time and status:
  running when the actor is among one sample of the first 200 running turns under the prefix, read once per reload,
  and paused from the actor, both when both hold. The footer says when the status covers only the first 200 running
  turns, or that it is unknown when that read failed. A failed page keeps its place and shows a Retry row. Typing drops
  the rows at once and reads the new prefix 300 ms after the last key. Selecting an actor fills the inspector pane,
  read only: its type, version, created time, parent, pinned node and paused flag, its running turn (message, node,
  lease, cancel requested) with the message's type and attempt when it is among the first 50 of the mailbox, the
  state as highlighted JSON (cut at 256 KiB with a notice), and tabs for the pending messages among the mailbox's
  first 50 and for the watches where it is the watcher and where it is the exact target. A missing actor shows "not
  found", an idle one "idle", and each part shows its own error. A toolbar button shows and hides the pane, and so
  does View › Hide Pane in the standalone app; ⌘R reloads the list from the top and the pane. A read that no longer
  matches the prefix, the selection or the context is dropped, a search that leaves out the selected actor unselects
  it, and a context switch or a closed window cancels the pending reads. Covered by the tests in
  `crates/cicada-inspector/src/tests/actors.rs` and `tests/json.rs`; the window has a manual check list.
- `cicada-inspector`: Actors rows fold open to show their children. Open a row to read its first 100 children, then
  choose `more…` for the next page. An actor shown both at the root and under its parent has separate selection and
  fold state for each row. Refresh reopens surviving rows as their pages load. A different search prefix or context
  starts with all rows folded.
- `cicada-inspector`: the Messages page. Its table lists the pending messages to the actors under the prefix in the
  search field, which it shares with Actors and keeps per page, 100 at a time as it scrolls, with their id, target,
  type, delivery time and attempt. Pages are not a snapshot: a message retried between two pages shows once, as first
  read, and a page of only such messages still moves on to the next. The footer shows the count, an empty prefix, or a
  failed page with Retry. Selecting a message fills the pane from its row, with no read of its own: the target,
  sender, type, delivery time, correlation id, headers, attempt and failures, and the payload as highlighted JSON
  (cut at 256 KiB), formatted only then; a long envelope scrolls above Show Actor and the payload. A selected
  message that a new search's pages no longer hold, once all are read, is unselected. ⌘R reloads the list from the
  top and clears the selection. Show Actor in the
  pane goes to the Actors page, searches for the message's exact target address and selects that root row once the
  first page has loaded; a gone actor shows "not found", and a failed page shows its Retry row with nothing
  selected. The jump is recorded in back and forward: each Messages place keeps the prefix and message it had when
  you left it, so Back returns to them (reading that prefix again when it changed since), and Forward goes to the
  actor again. A context switch clears back and forward. Leaving the Actors page or typing a search cancels a jump
  still waiting for its page. Messages shares the Actors list's pager. Covered by the tests in
  `crates/cicada-inspector/src/tests/messages.rs` and, unchanged, `tests/actors.rs`; the window has a manual check
  list.
- `cicada-inspector`: the Types and Watches pages. Types filters the registered types by name from one read per
  reload; it shows the contract hash with the full hash in a tooltip and the message type count. The pane lets you
  choose State or a message schema and formats that choice as highlighted JSON, cut at 256 KiB. Watches loads 100
  rows at a time in target and watcher order. It pages on scroll with an empty filter. A watcher or target prefix
  filters the loaded rows, and **Load more** reads a page even when no rows match. Both pages show **Retry** on a
  failed read. From a watch, **Show Actor** goes to the watcher or a one-actor target and records the jump in back
  and forward. Both pages keep their filter text when you navigate and clear their selection on ⌘R.
- `cicada-inspector`: the Outcomes page. Its table lists the finished messages of the actors under the prefix in the
  search field, 100 at a time as it scrolls, by a status switch in the toolbar: Failed (the default), Cancelled, Done
  or All, which adds a status column and includes purged outcomes. Rows show the message type, actor, attempt,
  finished time and the error's first line, with the whole error in a tooltip. Pages are not a snapshot: a message on
  two pages shows once, as first read, and a page of only such messages still moves on. Switching the status starts
  over from the first page and clears the selection, a search still waiting and the pane. The sidebar's Outcomes
  item shows the store's failed count from `/stats`, read with the Overview's refresh (shown as soon as `/stats`
  answers; a refresh that a later one replaced shows nothing) and with ⌘R on Outcomes: "…" until a read answers and
  after a context switch, the last count kept with the error in its tooltip when a read
  fails, and "—" from an older server that sends no count or when the first read fails. Selecting an outcome fills
  the pane from its row, with no read of the message: the status and error, attempts and failures, the last retried
  error, the retry policy, the envelope (id, target, sender, correlation id, finished time, headers) and the payload
  as highlighted JSON, formatted only then; Show Actor goes to its actor. The cause chain is one step at a time and
  follows the recorded `causedBy`: Caused by links to the message recorded as this one's cause (the message whose
  turn sent it, the original of an operator retry, or the message whose turn stopped the actor of a `Stopped`
  notice), and says "No recorded cause" when there is none, as for a send from outside an actor or a `Failed`
  notice. Caused lists the messages that record this one as their cause, one level down: pending or running ones and
  finished ones whose outcome is retained; its first 50 with "more…", Retry on a failed page, "None" when there are
  none and "not found" when the message is unknown or past retention. Following a link is a place in back and
  forward that reads that message fresh and shows it pending, running or finished, "not found" past retention, or
  its error with Retry; a status switch leaves those places as they were. Back to an outcome shows it as it was
  read, also when its row is no longer listed, and selects its row when it is listed or once a page brings it. A
  context switch clears back and forward and drops pending reads. Covered by the tests in
  `crates/cicada-inspector/src/tests/outcomes.rs`; the window has a manual check list.
- `cicada-tray`: the optional `inspector` feature adds Open Inspector… (⌘I while the menu is open). The window opens
  on the tray's connected context or resolves the target as `cicada` does when offline. Another pick brings the
  window forward with its own context. Quit, Ctrl-C and logout close it and request cancellation of its refresh before
  the tray exits.
- `cicada-inspector`: `Embedded::new(mtm, env, handle)` opens an inspector in a host's AppKit application with
  `show(context)`. The host supplies its environment and Tokio runtime handle. `close()` and dropping the controller
  close the window and request cancellation of its refresh. Reopening starts a fresh session.
- `cicada-app`: `context_client(&Env, Option<&str>)`, a client for a context of the config file resolved as the
  operator commands resolve it (`CICADA_SERVER` and `CICADA_TOKEN` first, then the named context, `CICADA_CONTEXT`,
  the file's current context), with their 30 s timeout, and the name of the context in use. It checks the server URL
  first, so no error quotes it, and moves the URL's user and password into Basic credentials (which win over a token,
  as before), so the client's URL, its errors and its `Debug` never hold them. They are decoded as reqwest decoded
  them: a user that is not UTF-8 sends no Basic credentials and a token stays. Covered by the `a_context_client_…`
  tests in `crates/cicada-app/src/config.rs`.
- `cicada`, `cicada-client`, `cicada-app`: `GET /stats` and `cicada stats` report the number of failed outcomes
  currently visible under the address prefix, using the visibility rules of `GET /outcomes`. The CLI table shows
  `-` when an older server omits the count.
- `cicada`: `Worker::end_grace()` ends the grace period of a stop that has begun, as if it had run out: the running
  turns are cancelled at once, their leases are released where the worker still holds them and can, and otherwise
  expire, and the stop returns within the cancel timeout (a wait for the polling loop ends too). Before a stop begins or
  after it ended it does nothing and is not remembered, so it never shortens a later stop. Covered by
  `end_grace_cancels_the_running_turns_at_once_and_the_stop_returns_within_the_cancel_timeout`,
  `end_grace_before_the_stop_reaches_its_grace_wait_still_ends_it` and
  `end_grace_before_a_stop_is_not_remembered_and_the_later_stop_keeps_its_full_grace_period`.
- `cicada-app`: `StopNow`, a cloneable handle in `WorkerReady::stop_now`. `StopNow::now()` ends the grace period of
  the worker over HTTP's stop, also of the stop after a panicking `App::on_worker_ready` function; a request before
  the stop begins is kept until it does, and one after the stop does nothing. It does not stop the worker by itself.
  Covered by the `over_http_stop_now_…` tests and `a_panicking_worker_ready_function_still_takes_stop_now` in
  `crates/cicada-app/src/node.rs`.
- `cicada-tray`: `Tray::stop_now()`, and a Stop Now row in the menu while the tray stops a worker that was ready. It
  ends the grace period, so the running turns are cancelled, and moves the countdown's `until` to now plus the cancel
  timeout when that is earlier; the tray is offline within the cancel timeout. A turn that ends without an acknowledged
  commit, also after a failed release, is listed as abandoned; one whose handler had ended may still commit; one whose
  release is still in flight stays listed as running until it ends. It does nothing in any other state. Covered by the
  `stop_now_…` tests in `crates/cicada-tray/tests/control.rs`.
- `cicada-app`: `ContextFile`, the config file of `cicada config` with its path rules, parser and atomic save (mode
  0600): `names`, `current`, `context` (a `ContextEntry` with `server`, `token` and `headers`), `set_context`,
  `set_server_and_token` (keeps the headers), `use_context`, `save`, `path` and `resolved_path`.
  `ContextFile::update` holds an exclusive lock on `<target>.lock` next to the file a save writes while it reads,
  changes and saves that same file, so two writers keep each other's changes; `cicada config use-context` and
  `set-context` use it. `check_server_url` is the URL check of a worker over HTTP. Covered by
  `two_updates_from_two_threads_are_both_kept` and the `ContextFile` tests in `crates/cicada-app/src/config.rs`.
- `cicada-tray`: Settings… (⌘,) opens a native Settings window: Connection (a context pop-up with Add…, the server,
  the token in a secure field) and Worker (a switch per registered type, scrolling past eight, and the turns at
  once from 1 to 1,000), with Revert and Apply and Reconnect. Apply writes only the fields the user changed, under the
  config file's lock and then `tray.toml`'s, and reconnects while the tray holds a connection; a context without a
  `tray.toml` entry gets the shown types and turns at once as its entry. Esc and ⌘W close the window. Covered by
  the `SettingsForm` tests in `crates/cicada-tray/src/settings.rs`, the `apply_…` tests in
  `crates/cicada-tray/src/menu.rs` and `apply_while_online_restarts_the_worker_with_the_new_arguments`.
- `cicada-tray`: `tray.toml` next to the config file keeps, per context, `types` and `concurrency`. A context with
  an entry connects with `--types` (`all` when every registered type is picked) and `--concurrency`, which win over
  `CICADA_TYPES` and `CICADA_CONCURRENCY`; a context without one inherits them. No registered type, a saved selection
  with no registered type left, or a lone type named `all` or `none` among others, ends the connection `Failed` with
  an error that says to open Settings. Covered by the `saved_settings_…` tests in
  `crates/cicada-tray/tests/control.rs` and the tests in `crates/cicada-tray/src/tray_file.rs`.

### Changed

- `cicada-app`: an unreadable config file is reported as `cannot read <path>: not a valid config file at line N` by
  every command, the worker included, without the parser's text, which can quote a token. `ContextEntry`, the config
  and `ContextFile` print the token, header values and a password in the server URL as `***` in `Debug`.
- `cicada-app`: `cicada config use-context` and `set-context` create `<config>.lock` next to the config file (next to
  its target when it is a symlink) and wait while another writer holds it. On a file system without file locks they
  fail instead of writing unlocked.
- `cicada-app`: `cicada config` and the menu bar app refuse a config path whose name, or whose symlink target's name,
  is `tray.toml` or ends in `.lock`, in any letter case, so that a config write never replaces a lock file or the
  tray's file.

## 0.25.0 — 2026-11-27

A placement member notices a second machine under its node id (ADR 0071): two machines started from a copied identity
folder write one node row in turn, so a member that finds another host name there sends no offers and reports
`Duplicate` to the pool, which marks the node with that host, alerts once and gives it no new places. A node can set the
host name it writes to its row (`NodeConfig::hostname`), and `NodeIdentity` locks its folder, so one machine runs one
process per identity. An offer can name a resource key, such as the Docker daemon's id that `RuntimeInfo::id` now
reports, and one live node holds a key (ADR 0072), so two nodes on one daemon count its places once. An operator changes
a live pool's silence and limits with `Configure` instead of deleting the pool. A full pool ignores a new node's offer
and sends one `full` notice per full period (ADR 0075) instead of refusing the offer for good. A host chooses its node
config once the run's types are known (`App::node_config_for`), and a new crate `cicada-tray` runs a worker over HTTP in
the host's process with its state and turns (ADR 0074; `App::type_names`, `App::on_worker_ready`). **BREAKING** for
placement pools: the contract of `cicada.placement` changes (two new messages, optional fields of the offer, the node
and the conflict mark, the state's `contested` and `full`, and the notice kind `full`), so hosts of 0.24.0 pools follow
Upgrading below; `NodeConfig`, `PlacementConflict`, `PlacementMsg`, `PlacementNode`, `PlacementState`,
`PlacementNoticeKind` and `Capacity` gain a field or a variant, and `PlacementNoticeKind` is `#[non_exhaustive]`. The
store schema, the HTTP API, the other actors' contracts and the guarantees are those of 0.24.0.

### Added

- `cicada-tray`: a portable model for a menu bar worker (ADR 0074). `Tray` builds a fresh `App` on each connect
  and runs its HTTP worker against a named context. It watches connection state, drains the worker on disconnect
  and shows its running turns and the last 50 finished turns, including turns abandoned without a confirmed commit.
- `cicada-app`: `App::type_names()` returns the hosted names in registration order, and `App::on_worker_ready`
  reports the settled grace period and cancel timeout once an HTTP worker starts.
- `cicada-app`: `App::node_config_for` chooses a host's node config from the types selected for `serve` or
  `worker`. Flags and environment variables still override the returned config. A worker over HTTP now checks its
  server URL before that call: a malformed one fails with `invalid server URL` or `the server URL must use http or
  https` instead of `the server is not ready: …`, still with exit code 1.
- `cicada`: `NodeConfig::hostname`, the host name a node writes to its row in the node table, such as a container's
  stable name; `None` (the default) keeps the machine's own, looked up once when the node is built. A set value is
  checked like `label` (not blank, no NUL, at most 200 characters) at the start. `ActorSystem::hostname()` returns the
  value the node writes; a machine renamed while the node runs keeps writing the old one. Covered by
  `a_configured_hostname_is_written_to_the_row_and_returned_by_the_node` and
  `a_blank_a_nul_or_a_too_long_hostname_fails_start`.
- `cicada-actors`: the duplicate check of `PlacementMember` (ADR 0071). Each round first reads the node's own row
  (`ActorSystem::node`); a host name there that is set and differs from `ActorSystem::hostname()` makes the round skip
  its probe and offer, send the pool `Duplicate { node, host }` and log an error, once for each other host name in a
  row; a round that finds no other host ends that run, so a later one is logged again. No host name of its own or in the
  row, or a failed read (a warning), checks nothing. A redeploy under a new host name trips nothing, as the old node
  removes its row when it stops. Covered by `crates/cicada-actors/tests/placement_duplicate.rs`.
- `cicada-actors`: the pool's `Duplicate { node, host }`, from anyone. For a live node it sets the conflict mark of
  ADR 0069 with the new optional `host` (and the node's own instance for a new mark, which the state's check now
  accepts only with a `host`), and a new mark is told `conflict` once. While its mark has a `host` the node gets no new
  places and keeps its holders. A mark set only by reports ends at the first turn one silence time after the last
  report; one set by offers and reports ends as an offer's mark, counted from the later of the two; going silent ends
  every mark. Nothing for a silent node; refused for good for an unknown node and a host out of bounds. Covered by
  `a_reported_duplicate_alerts_once_gives_no_new_places_and_ends_one_silence_time_after_the_last_report`,
  `a_report_and_a_conflicting_offer_share_one_mark` and
  `a_report_of_a_silent_node_changes_nothing_and_of_an_unknown_node_is_refused` in
  `crates/cicada-actors/tests/placement.rs`.
- `cicada-actors`: `NodeIdentity::load_or_create` first takes an exclusive OS lock on `placement-identity.lock` in its
  folder and keeps it while the identity or a clone of it lives; `PlacementMember` keeps the identity, so the lock
  lasts as long as the member. A second load fails with `InvalidIdentity::kind()` `IdentityErrorKind::Locked`; the
  other errors are `IdentityErrorKind::Invalid` (`#[non_exhaustive]`). Two processes starting at once on a missing file
  can no longer both write one. On a file system without locks, and in a read-only folder that holds the identity file
  but no lock file (it cannot be created), it logs a warning and goes on without one; a lock file that cannot be
  opened for writing is opened read-only and locked the same way. Covered by
  `a_second_load_fails_while_the_identity_lives_and_succeeds_after_all_are_dropped` and
  `a_read_only_folder_loads_the_identity_and_locks_a_read_only_lock_file`.
- `cicada-containers`: `RuntimeInfo::id`, the id of the runtime instance, and `RuntimeInfo::with_id`.
  `DockerRuntime::info` fills it from the `ID` of Docker's `/info` (empty: `None`), the same for every node on that
  daemon, in a second call next to `/version`; an error of either fails `info()`. `/info` gathers the daemon's whole
  state and can take seconds on a busy daemon, at every probe. The fake reports no id until `set_info` sets one. Covered
  by `runtime_info_takes_the_daemon_id_of_info_and_an_empty_one_is_none`,
  `info_answers_amd64_on_linux_until_a_test_sets_another_machine` and the E2E step
  `info_gives_linux_and_the_arch_the_cli_prints`.
- `cicada-actors`: resource keys in the placement pool (ADR 0072). `Capacity::resource(key)` makes the member send it
  as the new optional `resource` of `Offer` (1 through 200 characters without NUL; out of bounds the member sends no
  offer and warns, the pool refuses it for good), and the pool records it on the node (`PlacementNode::resource`). One
  live node holds a key: an offer whose key a different live node holds changes nothing for the offering node (a new
  node is not added, a known one keeps its last entry and goes silent if it keeps offering only that key). The pool
  lists the contest in the state's new `contested` (`PlacementContest { resource, node, at }`, one per key and offering
  node, `at` the store time of its last such offer, at most `maxNodes`; a new one past that is refused for good) and
  sends one `conflict` notice about the offering node when an entry is added; the check removes an entry one silence
  time after its `at` and now runs while a contest is listed, also without a live node. When the holder is silent or
  gone, the next offer of the key takes it. A decoded state with two live holders of one key is refused. Covered by
  the resource-key tests of `crates/cicada-actors/tests/placement.rs`,
  `a_capacity_with_a_resource_key_offers_it` and `tests/wire_formats.rs`.
- `cicada-actors`: the pool's `Configure { silence, max_nodes, max_waiting, max_holders }`, from anyone, each field
  optional (on the wire `silence` an ISO-8601 duration, `maxNodes`, `maxWaiting`, `maxHolders`): it changes the
  settings it names within the bounds of the create and keeps the others. A message without a setting (a misspelled
  name reads as absent), a value out of bounds, or a silence after which a node's or contest's silence would end past
  the last representable instant, is refused for good and changes nothing; a misspelled field next to a valid one is
  ignored. A limit below what the pool has removes nothing: the nodes, holders, waiting asks and contests stay, and new
  ones are refused or queued until the count is below it; a decoded state may now hold more than its limits, up to
  1,000 each. A larger `maxHolders` grants waiting asks in the same turn; a larger `maxNodes` admits a new node at its
  next offer. A new silence with a check armed arms the check at the new earliest end, due at once when that passed, so
  a shorter one marks the quiet nodes silent in the next turn; an equal one arms nothing. Recipe:
  `cicada send cicada.placement/runners --type Configure --payload '{"maxHolders": 8, "silence": "PT2M"}'`. Covered
  by the `Configure` tests of `crates/cicada-actors/tests/placement.rs`, the unit test
  `configured_replaces_the_given_settings_within_the_bounds_of_the_create` and `tests/wire_formats.rs`.

### Changed

- `cicada-actors`: a placement pool that knows `maxNodes` nodes or more ignores an offer of a node it does not know
  (ADR 0075) instead of refusing it for good, so a member past the limit no longer makes a dead letter at every
  interval. The turn commits and changes no entry. The pool keeps one mark in the state's new optional `full`
  (`PlacementFull { node, at }`: the node of the last ignored offer and its store time, written only while set) and
  sends the alert target one notice of the new kind `full` (`PlacementNoticeKind::Full`) about the first ignored node
  when the mark is set; later ignored offers, from any new node, only move it. The check removes the mark one silence
  time after its `at`, and an offer that adds a node (after `Configure` raised `maxNodes` or a `Gone` freed a slot)
  removes it at once, so a pool that fills up again alerts again. An ignored offer arms the check when none is armed;
  `armed` is now set while a node is live, a contest is listed or the pool is marked full, and a decoded state with a
  mark and no armed check is refused. The resource check of ADR 0072 comes first: a new node offering a held key is a
  contest with its `conflict` alert. A known node offers as before. `PlacementNoticeKind` becomes `#[non_exhaustive]`,
  so a later kind does not break hosts again; a host's `match` on it needs a wildcard arm once. During a rolling
  upgrade an alert target on an older node does not know `full` and fails that alert for good, and nothing else.
  Covered by the full-pool tests of `crates/cicada-actors/tests/placement.rs`, the unit test
  `a_decoded_full_mark_is_checked_and_keeps_the_check_armed` and `tests/wire_formats.rs`.

### Changed (breaking)

- The contract of `cicada.placement` changes: `PlacementMsg` gains `Duplicate` and `PlacementConflict` the optional
  `host`, so its hash differs from 0.24.0's. Rust code that matches `PlacementMsg` exhaustively gains `Duplicate`, and
  code that builds `PlacementConflict` gains `host: None`. `PlacementMsg` also gains `Configure`, which an exhaustive
  match gains too. The resource keys add the optional `resource` to `Offer` and to the node entry and the state field
  `contested`: code that builds or matches `PlacementMsg::Offer` without `..` or builds `PlacementNode` or
  `PlacementState` gains them (`resource: None`, `contested: vec![]`). The full mark adds the state field `full` (code
  that builds `PlacementState` gains `full: None`) and the notice kind `full`: a `match` on `PlacementNoticeKind`
  gains a wildcard arm, as the enum is now `#[non_exhaustive]`.
- `Capacity` gains the public field `resource`: a struct literal or a pattern without `..` no longer compiles; use
  `Capacity::new` and `Capacity::resource`.
- A state of a placement pool is now written with `contested` (`[]` when empty).
- `NodeConfig` gains the public field `hostname`: a struct literal without `..NodeConfig::default()` no longer
  compiles; add `hostname: None`.
- `NodeIdentity` no longer derives `Debug`, `PartialEq` and `Eq` but implements them: equality compares the node id
  and the instance, and the lock is left out. A second `load_or_create` of one folder in one process fails with
  `Locked` while the first identity lives: drop it first, such as in a test of a restart.

### Upgrading

- Hosts with placement pools on 0.24.0 follow "Changing a contract" in the operations guide for `cicada.placement`,
  whose type name is fixed, so in a maintenance window: stop every host of the type, delete its pools and then its
  registration, and start the new hosts, which create the pools again.
- A node that shares an identity folder with another process on the same machine, such as a rolling deploy's new
  container on a shared volume, fails to start with `Locked` until the old process stops.

## 0.24.0 — 2026-11-26

A placement pool (ADR 0067, ADR 0068), a new ready-made actor: nodes offer places with labels to a pool, a node that
stops offering is marked silent, with one notice, and work asks for a place with its needs and gets a node to pin its
actors to; a holder that is deleted frees its place; an operator drains a node and declares a silent node gone, whose
holders are told; a second machine under a live node's id is ignored and reported once (ADR 0069); a member offers a
node's places while its runtime answers, under an identity kept across restarts; operators drive it with `cicada send`.
Additive: a new ready-made actor with its own contract; the contracts of the other actors, the store schema, the HTTP
API and the guarantees are those of 0.23.1.

### Added

- `cicada-actors`: the placement pool `cicada.placement/<pool>` (`Placement`, `PlacementActor`, `PlacementMsg`,
  `PlacementState`, `PlacementNode`, `PlacementNodeStatus`, `PlacementAlerts`, `PlacementNotice`,
  `PlacementNoticeKind`, `InvalidPlacement`). The host creates each pool with `PlacementState::new(silence,
  max_nodes, max_waiting, max_holders)` (silence 1 second through 1 day, 1 through 1,000 nodes; the line and holder
  limits are asks, below). A member sends
  `PlacementMsg::offer(node, instance, labels, places)` at an interval shorter than the silence: up to 32 labels,
  sorted and kept once, and 0 through 1,000 places; each offer replaces the node's entry and records the store time of
  its turn as `heardAt`. The pool's own `Check`, armed at the earliest end of a live node's silence (its time in
  `armed`, its id in `armedId`; made due early, it arms itself again at the same time), marks a node not heard from for the silence as `silent`; each change is sent as a `PlacementNotice`
  (`silent` once per silence, `back` at the next offer) to the target of the last `Alerts`, in an `Alert` turn of its
  own, so a failing target fails only that alert. `Rearm` replaces a lost check at its time. Out-of-bounds messages, a new node
  past `maxNodes`, and a `Check` or `Alert` from another sender are refused for good. Its contract is in
  `docs/reference/contracts/cicada.placement.json`. Covered by `crates/cicada-actors/tests/placement.rs` and
  `tests/wire_formats.rs`.
- `cicada-actors`: asks in the placement pool (ADR 0068; `PlacementAnswer`, `PlacementAnswerKind`, `PlacementRefusal`,
  `PlacementHolder`, `PlacementAsk`). `PlacementMsg::ask(holder, needs, &target, message_type)` asks a place on a node
  with every one of up to 32 needs; the `PlacementAnswer` goes to the target in the ask's commit: `granted` with a
  grant number that only goes up and the node (the live node with every need and a free place that has the most free
  places, ties to the smaller node id), `queued`, or `refused` with `noNode` (no node of the pool has the needs),
  `lineFull` (the line holds `maxWaiting`, 0 through 1,000) or `poolFull` (a place fits, but the pool holds
  `maxHolders`, 1 through 1,000, and the line is full). An ask only silent or full nodes could meet, or one past the
  holder limit, waits in the line; after every message the pool grants waiting asks in line order, skipping those it
  cannot place now, and tells each in a `Notify` turn of its own; a grant whose answer never reached its holder is
  freed through the pool's watch on itself. `PlacementMsg::release(holder, grant)` frees a place only for the matching
  grant; `PlacementMsg::cancel(holder)` withdraws a waiting ask. A repeated ask is answered again with the first ask's
  grant, node and target. Fewer offered places or a silent node keep their holders. Covered by
  `crates/cicada-actors/tests/placement.rs` and `tests/wire_formats.rs`.
- `cicada-actors`: a deleted holder frees its place in the placement pool. The pool watches the target of each holder
  and waiting ask with `Turn::watch_existing`, from the commit that first records the ask, and unwatches a target
  that holds and waits for nothing at the end of the turn. When a target is deleted, before or after that commit, its
  one `Stopped` frees every place held for it, once, and takes its waiting asks out of the line; freed places go to the
  line as after a `Release`. So a holder need not release before it stops. A `Stopped` from a client, for the pool
  itself or for a target that holds nothing changes nothing; a `Failed` of a watched target's message costs the pool a
  turn and changes nothing. An ask whose target is the pool itself is refused for good. No new message or state field.
  Covered by `crates/cicada-actors/tests/placement.rs`.
- `cicada-actors`: drain and gone in the placement pool. `PlacementMsg::drain(node)` stops new places on a known node:
  it keeps its holders, renews its offers and can go silent and come back, an offer keeps the drain (the node field
  `draining`), and an ask only it could meet waits instead of being refused. `PlacementMsg::undrain(node)` clears it
  and grants the waiting asks that fit. `PlacementMsg::gone(node)` of a node whose status is `silent` removes it, frees
  its places and tells each of its holders a `PlacementAnswer` of the new kind `gone`, with its grant and node, in a
  `TellGone` turn of its own, so a target that is gone fails only its answer; a later `Release` of those grants frees
  nothing, waiting asks stay, and the node offering again is a new node. `Gone` of a live node, `Drain`, `Undrain` and
  `Gone` of an unknown node, and a `TellGone` from another sender are refused for good. A `gone` answer means the place
  is no longer counted; what ran there may still run. Covered by `crates/cicada-actors/tests/placement.rs` and
  `tests/wire_formats.rs`.
- `cicada-actors`: the conflict alert of the placement pool (ADR 0069; `PlacementConflict`). An offer whose instance
  differs from the recorded one while the node is `live` changes no entry field; its turn succeeds and sets the node
  field `conflict` to `{instance, at}`, the other instance and the store time of its turn. A new mark sends one
  `PlacementNotice` of the new kind `conflict` in an `Alert` turn of its own; later conflicting offers, from any other
  instance, only update the mark. The mark ends when the node goes `silent`, or when the recorded instance offers one
  silence time or more after the mark's time; a later conflict alerts again. A silent node's entry is taken over by the
  next offer, whatever its instance. A decoded `conflict` on a node that is not live, or naming the recorded instance,
  is refused. The pool only reports: two machines under one node id also share leases and the node row in the store,
  and a cloned disk with the identity copied looks like one machine. An instance drawn at each process start makes a
  restart within the silence time count as a conflict; the member's `NodeIdentity` keeps it (below). Covered by
  `crates/cicada-actors/tests/placement.rs` and `tests/wire_formats.rs`.
- `cicada-actors`: the placement member (`PlacementMember`, `MemberSettings`, `Capacity`, `NodeIdentity`,
  `InvalidIdentity`). `NodeIdentity::load_or_create(dir)` reads `dir/placement-identity.json`
  (`{"node": …, "instance": …}`), or creates the folder and the file with the node id `node-<16 hex digits>` and a
  random instance, written to a temporary file and renamed; a file that cannot be read or parsed is an
  `InvalidIdentity` and stays as it is, never a new identity. `node_id()` is the default for `NodeConfig::node_id`;
  `instance()` goes into each offer, so a restart is the same machine to the pool. `PlacementMember::new(pool,
  identity, settings, probe)` and `run(system, stop)`, run as an app task: a round at the start and then every
  `interval` (`MemberSettings`, 15 s; `with_interval` refuses zero) calls the host's probe; a `Capacity { labels,
  places }` sends an `Offer` with the node's own id (`ActorSystem::node_id`, so `--node-id` and `CICADA_NODE_ID` still
  win) and the identity's instance, `None` sends nothing, so the node goes silent at the pool. An offer out of bounds
  and a failed or refused send are logged and tried next round; `stop` ends `run`, also while a probe hangs. The
  rustdoc of `Offer::instance`, and so the pool's contract reference, says the instance is kept across restarts.
  Covered by `crates/cicada-actors/tests/placement_member.rs`.

## 0.23.1 — 2026-11-26

An `OrphanWatch` of `cicada-containers` reports the containers of a job that have no container actor, without removing
them. A collection of outputs that keeps failing, timeouts included, still ends, and `copy_out` reads at most
`CopyLimits::max_read` bytes. Additive: the contracts, wire formats, the store schema, the HTTP API and the guarantees
are those of 0.23.0.

### Added

- `cicada-containers`: `OrphanWatch<J: ContainerJob>`, which reports the containers of a job that have no container
  actor (left by a lost store, a test, an actor deleted by hand), outside actor turns.
  `OrphanWatch::new(runtime, prefix, settings, on_orphan)` takes the job's container prefix, `OrphanSettings`
  (`interval` 5 minutes) and a plain `Fn(&str)`; a zero interval fails with `InvalidOrphanWatch`. `run(system, stop)`
  watches until `stop` completes: each round lists the containers labelled with the prefix and reads the actor of
  `J::TYPE` keyed by the name without the prefix with `ActorSystem::get_actor`. A container whose actor is missing in
  two rounds in a row is an orphan: the watch logs a warning and calls `on_orphan` with its name once, and never
  removes it. What it reported is kept in memory, pruned to the listed names: a container whose actor appears is
  dropped, a removed one is forgotten, and after a restart each orphan is reported once more. A labelled name without
  the prefix, or without a valid key, is skipped with one warning. No error ends `run`: a failed `list` or actor read
  is logged and tried next round, and that round does not count. A host runs it as an `App::task`, one per Docker
  daemon. Limits: two stores on one daemon with the same prefix see each other's containers as orphans, so give each
  its own prefix; a container made before 0.22.1 carries no label and is never seen. Covered by the tests of
  `src/orphans/`.
- `cicada-containers`: a cap on what `copy_out` reads. `CopyLimits` gains `max_read`, the most content bytes of
  regular files one copy reads, kept and skipped alike (a skipped file's data passes through Docker's archive all the
  same); `CopyLimits::new` sets it to 4 × `max_bytes` (saturating), so 256 MiB for the default `Outputs`, and
  `with_max_read(bytes)` changes it. A zero `max_read` fails with `InvalidSpec`, as the other limits do. A regular file
  that would take the bytes read past it stops the copy before its data: the files before it are kept, and one last
  skipped entry for the path says `the rest was not read: over the read limit of N bytes`. Docker and the fake stop at
  the same entry. So a huge file in an output folder is no longer read in full on every try of a collection.
- `cicada-containers`: a collection that keeps failing still ends. A turn past the job's `MAX_TURN` commits nothing,
  so a copy that always takes too long used to be retried until the poll's retry policy gave up, and the end was never
  sent. Now a collecting step whose message failed 3 times, timeouts included, makes no copy, log read or progress
  read on its next try, and records and sends the pending end with `outputs.error` =
  `(no outputs: the collection failed 3 times; last: <the last error>)` and the counts of the files the store holds.
- `cicada-containers`: warnings when the output store keeps failing. While collecting, the errors in a row that reach
  `ContainerSettings::max_errors` log a warning as the end goes out without its outputs; while deleting stored
  outputs, a warning when the errors reach `max_errors`, once, and the deletion keeps trying (each retry before and
  after logs at `debug`).

### Changed

- `cicada-containers`: a file of an output folder past the read limit ends the copy there, so smaller files after a
  huge one are no longer kept: where 0.23.0 skipped the huge file and kept going, 0.23.1 keeps the files before it
  and reports the rest as not read. Raise the limit with `CopyLimits::with_max_read` to keep the old behaviour for
  folders with a few large files.

## 0.23.0 — 2026-11-25

A container job's program reports its progress through a file, and the container actor sends it to the owner as numbered
messages (ADR 0065). With an output store, the container actor keeps the rest of a container's log and its job's output
folder there at the end, in its own turns, and the end tells the owner what was stored and skipped (ADR 0066). With
`output_retention`, the actor deletes those outputs a set time after the container's removal. A breaking release:
`Start` and `EndTarget` gain `progressType`, `ContainerState` two progress fields, a `pending` end, the phase
`collecting` and `outputsUntil`, and `ContainerEnd` an optional `outputs`, so the contract hash of every container job
type, and of every owner type whose messages take a `ContainerEnded`, changes. Hosts on 0.22.x follow "Changing a
contract" in the operations guide for those types. The store schema, the HTTP API, the ready-made actors' contracts and
the guarantees are those of 0.22.1.

### Added

- `cicada-containers`: progress through a file. `ContainerMsg::Start` and `EndTarget` take an optional `progressType`
  (not empty, at most 200 characters, no NUL, as `messageType`; one out of bounds is refused for good), kept from the
  first `Start` as the end's target is; `ContainerMsg::start_with_progress(target, message_type, progress_type)` builds
  it. With it, the actor sets `CICADA_PROGRESS_FILE` to the job's new `ContainerJob::PROGRESS_FILE` (default
  `/tmp/cicada-progress`) in the spec, after the job's `spec`, which cannot change or hide it; a path that is relative,
  has a `..` part or names no file ends the container `failed` with the kind `notStarted` before anything is made. The
  actor reads the file with `copy_out` (at most 64 KiB, one file) at each poll of the running container and once in the
  turn that records an end for which it ran (`exited`, `failed` with `runtimeEnded` or `outOfMemory`, `stopped`,
  `timedOut`), and when its text changed sends `ContainerProgress { key, seq, text }` to the end's target as
  `progressType`, with `seq` counting from 1 and the end's retry policy. The text is lossy UTF-8 with NUL replaced by
  U+FFFD, sent as it is; an empty text only after one that was not. The last progress goes in the end's commit, before
  the end, but a retried progress can arrive after a later one or after the end: drop a progress whose `seq` is not
  above the last one handled. Best effort: a missing file, one over 64 KiB or not a regular file, and a failed read send
  nothing and are not runtime errors; a send the target's registry refuses is not retried, still moves `seq` on and
  does not hold up the end. The program writes the file to a temporary name and renames it into place; `copy_out`
  cannot see a tmpfs, so with a read-only root and a tmpfs on `/tmp` set `PROGRESS_FILE` on a volume. The state keeps
  `progressSeq` and `progressHash` (the SHA-256 of the last text sent); `sha2` is no longer optional in the crate.
  Covered by the progress tests in `crates/cicada-containers/tests/actor.rs`.
- `cicada-containers`: outputs at the end (ADR 0066). `ContainerActor::with_store(Arc<dyn OutputStore>)` gives the
  actor a store, and `ContainerJob::outputs(&request) -> Option<Outputs>` (default `None`) names a folder with its
  limits (`Outputs::new(path)`: 1000 entries and 64 MiB, the `CopyLimits` of `copy_out`). With a store, an end the
  container ran for (`exited`, `failed` with `runtimeEnded` or `outOfMemory`, `stopped`, `timedOut`) is first held in
  the state's new `pending` in the new phase `Collecting`, with `end` still empty, so a saved end still means a sent
  end. The next polls store the rest of the log with the guarded append (at most 20 pages of 500 lines per turn, an
  immediate next poll while more is left; next to a `LogArchiver` of the same store each line is stored once), then one
  turn copies the folder out and puts each file into the store. The poll that finishes reads the last progress, records
  the end with `outputs: OutputsReport { stored, bytes, skipped, firstSkipped, error }` (the first 10 skipped as
  `SkippedFile { path, reason }`, which gains serde and `JsonSchema`; a file the store refuses is skipped with its
  reason), sets `Ended` and sends it. Without an outputs folder only the log is stored and the counts are 0. A runtime
  or store error a retry can cure counts as a runtime error and backs off; at `max_errors` in a row the end goes out with
  its outcome and `error: "(no outputs: …)"`; a bad outputs path (`InvalidSpec`), a missing `copy_out` (`Unsupported`)
  and a gone container (`Conflict`) end the collection at once with the error; a missing `logs_since` skips the log. A
  `Start` during the collection changes nothing: the end goes to the first target when it finishes; a `Stop` stops
  nothing more; a `Remove` removes the container after the end is sent. Ends the container did not run for, actors
  without a store and ends saved by 0.22 never collect and carry no `outputs`. Memory per collecting container is at most
  its byte limit, and the copy runs in one turn under the job's `MAX_TURN`. Covered by
  `crates/cicada-containers/tests/outputs.rs`.
- `cicada-containers`: the store's input rules are tighter (from the review of the output store): a container name
  over 200 characters and a control character (NUL included) in a path give `Invalid` in `check_container_name`,
  `clean_output_path`, `MemoryOutputStore` and `check_output_store`; the rustdoc of `OutputStore` warns that a store on a
  case-insensitive file system must keep `Job` and `job` apart.
- `cicada-containers`: output retention. `ContainerSettings::output_retention: Option<Duration>` (default `None`: kept
  until the host deletes them). With a store and a retention, the step that removes the container saves
  `outputsUntil`, now plus the retention capped at the largest instant, in the state and arms a poll then; that poll
  calls `OutputStore::remove` with the container's name and clears it. The container is removed from the runtime first,
  so no `LogArchiver` stores it again. The saved time decides: a retention changed or unset later does not move it, and
  zero deletes in the poll after the removal. `Unavailable` and `Other` are tried again with the backoff up to
  `max_backoff`, without limit; `Invalid`, and an actor without a store at the due time, clear `outputsUntil` with a
  warning. While it is set, a removed container has work left, so `Check` re-arms a lost poll at the saved time. The
  actor's own record stays. An actor deleted before the due time leaves its outputs in the store: call
  `OutputStore::remove` then. Covered by the retention tests in `crates/cicada-containers/tests/outputs.rs`.

### Changed

- **BREAKING** for container job types: their contract changes ("Changing a contract"), as `Start` and `EndTarget`
  gain `progressType` and the state gains `progressSeq` and `progressHash`. Rust code that builds or matches
  `ContainerMsg::Start` or `EndTarget` gains the field `progress_type`, and code that builds `ContainerState` by its
  fields gains `progress_seq` and `progress_hash`. A target type's contract changes only when it adds a variant that
  takes a `ContainerProgress`.
- **BREAKING** for container job types, the same contract change: the state gains `pending` and the phase
  `collecting`, and `ContainerEnd` the optional `outputs`, written only when there is one. `ContainerEnd` is not
  `#[non_exhaustive]`: Rust code that builds it gains `outputs: None`; code that matches `Phase` exhaustively gains
  `Collecting`, and code that builds `ContainerState` by its fields gains `pending: None`. An owner's message family
  that takes `ContainerEnded` changes its contract too, as the end's schema gains `outputs`.
- **BREAKING** for container job types, the same contract change: the state gains `outputsUntil`, set only in the phase
  `removed`; a state saved by 0.22 has none, and its outputs are not deleted. Code that builds `ContainerState` by its
  fields gains `outputs_until: None`.
- `cicada-containers`: `check_container_name` and `clean_output_path` refuse what they took before: a name over 200
  characters and a path with a control character. A host's own store that uses them refuses them too.

### Upgrading

- Hosts on 0.22.x with container job types follow "Changing a contract" in the operations guide for each of them: a
  versioned type name for new runs while the old actors finish, or a maintenance window (stop every host of the type,
  delete its actors and then its registration, start the new hosts). Send `Remove` to a container actor before
  deleting it, or its container stays. Add `progress_type: None` to every `ContainerMsg::Start` and `EndTarget`
  literal, such as in a host's tests, and `outputs: None` to every `ContainerEnd` literal.
- Do not roll a node back to 0.22 while a container actor with a store is in the phase `collecting`: 0.22 cannot read
  that state.

## 0.22.1 — 2026-11-25

A host keeps a container's log and output files in its own `OutputStore` of `cicada-containers`, with one guarded write
for lines and cursor and a conformance check to test it, and a `LogArchiver` follows its containers' logs into it.
Additive: the contracts, wire formats, the store schema, the HTTP API and the guarantees are those of 0.22.0.

### Added

- `cicada-containers`: `OutputStore`, an async trait a host implements to keep a container's outputs outside it, keyed
  by container name and used as `Arc<dyn OutputStore>`: `log_end` (the stored `LogCursor`, `None` when nothing is
  stored, and the number of stored lines, as `LogEnd`), `append_logs(container, after, page)`, `read_logs(container,
  from, limit)` (up to `limit` lines from the 0-based line number `from`, each a `StoredLine` with its number),
  `put_file` (a `CopyFile`, replacing one at the same path), `files` (`StoredFile { path, size, mode }` in path
  order), `read_file` and `remove` (lines, cursor and files; nothing is `Ok`). `append_logs` stores a page's lines and
  its `next` cursor in one write, and only when the stored cursor equals `after`; otherwise it fails with `Conflict`
  and stores nothing, so a writer that crashed after a write resumes from `log_end` without a repeated or lost line,
  and two writers cannot interleave. An empty page stores its cursor alone. After `remove` a container starts afresh.
  Errors are `OutputError { kind, message }` with `OutputErrorKind::{Invalid, Conflict, Unavailable, Other}`;
  `is_retryable` is true only for `Unavailable`. Every method checks its input first with the new public
  `check_container_name` (the container actor's rule: a letter or digit first, then only `A-Za-z0-9_.-`) and
  `clean_output_path` (the rule of `copy_out`: absolute, no `..` part, not `/`; empty and `.` parts dropped, so `/a`,
  `//a` and `/./a` are one file), and fails with `Invalid` before it touches anything. Behind `testing`:
  `MemoryOutputStore`, an in-memory store whose clones share their outputs, and `check_output_store(&store)`, an async
  conformance check that runs every rule against any store under container names of its own and panics naming the
  rule it broke; it cannot prove that a write is all or nothing after a crash, or that the guard holds under writers
  in parallel. Covered by the tests of `src/outputs/` (the check passes on the memory store and fails on four broken
  ones).
- `cicada-containers`: `LogLine` and `LogStream` have serde: `{"time": "<RFC 3339>", "stream": "out" | "err", "text"}`.
- `cicada-containers`: `LogArchiver`, which follows the log of every container of a container job and keeps it in an
  `OutputStore`, outside actor turns. `LogArchiver::new(runtime, store, prefix, settings)` takes the job's container
  prefix and `ArchiveSettings` (`interval` 2 s, `page_lines` 500, `max_parallel` 16 reads at once); a `page_lines`
  outside 1 to `LOG_PAGE_MAX` fails with `InvalidArchive`. `run(stop)` archives until `stop` completes: each round
  lists the containers labelled with the prefix and starts a read for each one not drained and not still being read,
  without waiting for them, so a slow log holds up only itself. A read appends each page of `logs_since` from the
  stored cursor with the guarded `append_logs`, so every line is stored once and in order, also after a restart and
  with two archivers on one store; an ended container is drained once and then skipped. No error ends `run`: a log
  that is `Unsupported` is drained and logged once, a container gone during its read is dropped, a store's `Invalid`
  skips the container, a store's `Conflict` reads the cursor again, and every other error, and a failed `list`, is
  tried again in a later round, with a wait that doubles from the fifth failure in a row, at most 60 s. A host runs it
  as an `App::task` or spawns it, one per Docker daemon. Limits: a container made before the label is not archived,
  and lines a container writes after the last read are lost when it is removed before the next round. Logging goes
  through `tracing`, a new dependency of the crate, and `tokio` is no longer optional. Covered by the tests of
  `src/archive/`.
- `cicada-containers`: the container actor puts the label `PREFIX_LABEL` (`cicada.container.prefix`) with its
  settings' prefix on every container it creates, after the job's `spec`, which cannot drop or change it. Covered by
  `the_actor_labels_its_containers_with_its_prefix_after_the_job_spec` in `tests/actor.rs`.

## 0.22.0 — 2026-11-25

A turn can watch an actor that must exist (ADR 0063): `Turn::watch_existing` tells the watcher at once when the actor is
already gone, so a pool that gives a place to a run and watches it frees the place also when the run went before its
turn. `DockerRuntime::connect()` of `cicada-containers` finds Docker through the Docker CLI's context, so it works on a
Mac with OrbStack, Colima or Docker Desktop and no `/var/run/docker.sock`. A container job sets its actor's turn limit,
so a slow image pull fits one turn without raising the node's default. `App::task` ties a host's background task to its
node (ADR 0064): an early end of the task stops the node with an error, so a supervisor restarts it. A container whose
log can never be read (Docker's log driver `none`) gives `Unsupported`, so a log follower stops, and the container actor
ends it without retries. **BREAKING**: the public `WatchChange` and `WatchChangeView` gain a field. The HTTP field is
optional and every older request and fixture keeps its bytes. No store schema, actor contract, contract hash or
guarantee changes.

### Added

- `Turn::watch_existing(target)`: one actor (a typed ref, an address or its text). The commit checks the target where
  it applies the watches, in the order of the calls and before the turn's deletes and creates. When the actor is
  there, a plain watch, as `Turn::watch` sets it. When it is not, the watcher gets one `Stopped` of it in the same
  transaction, with the cause `deleted` and `by` null, as for a delete from outside a turn, the turn's message as
  `causedBy` and the absent actor as sender, numbered with the commit's other notifications; the pair holds no watch
  then, an older one included, and a later create of that address is not watched. A delete in the same turn writes
  its own `Stopped`; an actor created in the same turn is not there yet and gives a `Stopped` (use `watch` there).
  Two calls for one absent actor give one `Stopped`. A type prefix or `*` is `TurnError::Invalid`, as is a family
  without `Failed` and `Stopped`. A delete by another node is seen exactly once: on PostgreSQL the commit locks the
  target's row with its other rows and "there" means that lock found it; SQLite and the in-memory store check inside
  their write lock. Covered by `crates/cicada/src/tests/conformance/watch_existing.rs` on every backend (with a race
  of a delete against the commit), `crates/cicada/tests/watch_existing.rs` (the pool case) and
  `a_watch_existing_of_an_absent_actor_over_http_writes_the_stopped_of_a_local_commit` in
  `crates/cicada-client/tests/remote_worker.rs`.
- `existing` on a watch change of `POST /leases/complete` (`WatchChangeView`), `true` for `watch_existing`, absent or
  false otherwise and left out when false. With a type target, every type or `set: false` the report is invalid
  (`400`). Portable fixtures: a valid body with `existing`, an `existing: false` input, and both refusals at
  conversion. `docs/openapi.json` has the field.
- `cicada-containers`: `ContainerJob::MAX_TURN: Option<Duration>`, `None` by default, is the `Actor::MAX_TURN` of
  `ContainerActor<J>`: it wins over the node's default turn limit and over `--max-turn` / `CICADA_MAX_TURN`, and
  `None` keeps the node's default. Set it above the create step (a pull of a missing image, the create, the folders and
  copies in) and the settings' `stop_grace`, which each run inside one turn. Zero is refused when the type is
  registered. A step past it ends `failed` (`timeout`), not counted as a runtime error; the poll's retry policy runs
  it again (7 attempts, 1 to 32 s), and a container the cut-off step made is found, not made twice. Not part of the
  contract. Covered by `the_actors_turn_limit_is_the_jobs` in `src/actor/mod.rs` and the three turn-limit tests of
  `tests/actor.rs`.
- `cicada-app`: `App::task(name, f)` and the public `Stopping` (`Clone`, `async fn wait`, `fn is_set`), ADR 0064. In
  `serve` and `worker` on the store, `f` gets the node's `ActorSystem` and a `Stopping` once every `on_start` function
  has finished, in the order of the calls, and is polled once before `serve` writes `Listening on`. A task that
  returns `Ok`, returns `Err` or panics before the stop stops the node in good order and fails the command (exit code
  1) with `task <name> ended before the stop: <error>`. At a stop (a signal, an early end, a failed write of
  `Listening on` or a failed HTTP server), `Stopping` is set when the node's stop begins; the tasks share the node's
  `shutdown_grace_period`, and a task still running at its end is aborted with a warning naming it. `serve` keeps
  answering, with `/ready` at 503, until the node and the tasks have stopped; the store closes after both, so a stop's
  worst case is unchanged. An error or panic after the stop is a warning and leaves exit code 0. Two tasks with one
  name refuse `serve` and `worker` (exit code 2, `two tasks named <name>`); a worker over HTTP refuses an `App` with a
  task as with `on_start` (exit code 2), and the `worker` help names both (`docs/reference/cli.md`). Covered by
  `crates/cicada-app/tests/tasks.rs`, `tests/task_logs.rs` and
  `on_start_and_a_task_are_refused_over_http_before_any_request` in `tests/worker_http.rs`.

### Changed

- `cicada-containers`: `DockerRuntime::connect()` finds the daemon as the `docker` CLI does, in this order:
  `DOCKER_HOST`; else the context `DOCKER_CONTEXT` names (not empty), even when the local socket exists, with
  `default` meaning the local socket; else `/var/run/docker.sock` when it exists; else the `currentContext` of
  `config.json` in `DOCKER_CONFIG`, else `$HOME/.docker` (none, `default` or no `config.json`: the local socket, as
  before). A context's endpoint is `Endpoints.docker.Host` of `contexts/meta/<sha256 of its name>/meta.json`, and is
  connected as `connect_to` connects it; `docker_config(dir)` does not steer it. A server with its socket keeps it,
  whatever `config.json` names. A context that cannot be read (no meta file, no docker endpoint, broken JSON, no config
  folder) or used (`SkipTLSVerify`, TLS files under `contexts/tls`, or a scheme other than `unix`, `tcp` and `http`,
  such as `ssh`) fails with `Other`, naming the context and whether its name came from `DOCKER_CONTEXT` or
  `currentContext`; it never falls back to the socket or plain `tcp`: set `DOCKER_HOST` or use `connect_to`. A context
  whose unix socket is missing fails with the retryable `DaemonUnavailable`, naming the context and the endpoint.
  Covered by the unit tests in `src/docker/context.rs` and `src/docker/mod.rs`; the E2E class of
  `e2e/tests/suite/container_docker.rs` now connects with `connect()` alone.
- `cicada-containers`: `logs` and `logs_since` of a container whose log the runtime can never read fail with
  `Unsupported`, no longer `Other`, so a follower can tell "stop, there will never be logs" from a passing daemon
  error (Factory issue 0021). `DockerRuntime` maps Docker's 501 to a log read ("configured logging driver does not
  support reading", log driver `none`) to `Unsupported` with Docker's message; every other error keeps its kind. The
  0.21.0 notes and the rustdoc of `logs_since` and `DockerRuntime` promised `Other` for this case: a caller that
  matched `Other` there now matches `Unsupported`. `ErrorKind::Unsupported` now also means "cannot do it for this
  container". The container actor records an end at once with `(no logs: …)` when its log read is `Unsupported`,
  without counting an error or retrying the read; other kinds keep the count. The fake scripts such a container with
  `fail_always(Method::Logs, ErrorKind::Unsupported)` (or `Method::LogsSince`). Covered by the unit test in
  `src/docker/map.rs`, the actor tests in `tests/actor.rs` and the E2E step
  `a_log_driver_none_is_unsupported_created_and_after_it_ran`.

### Changed (breaking)

- `WatchChange` and `WatchChangeView` have the new public field `existing`: a struct literal or an exhaustive
  destructuring of them no longer compiles; add `existing: false`. A store commit refuses a `WatchChange` with
  `existing` on a type, every type or without `set` as `StoreError::Invalid`.

### Upgrading

- Upgrade every HTTP server before a worker runs a turn that calls `watch_existing`: a 0.21 server refuses the field
  `existing` as unknown, and the turn's report is refused. A 0.21 worker keeps working against an upgraded server.

## 0.21.1 — 2026-11-25

A container runtime of `cicada-containers` reports its processor kind and operating system, which also shows that it
answers, and `DockerRuntime` pulls a private image with the machine's `docker login`. Additive: the contracts, wire
formats, the store schema, the HTTP API and the guarantees are those of 0.21.0.

### Added

- `cicada-containers`: `ContainerRuntime::info()` returns a `RuntimeInfo` with `arch` and `os` in the spelling of image
  platforms (`amd64`, `arm64`, `arm`; `linux`), for a host that says it is here only while its runtime answers and
  picks the runs that fit the machine. It changes nothing and is safe to repeat; a runtime that cannot be reached
  fails with the retryable `DaemonUnavailable`. `RuntimeInfo` is `#[non_exhaustive]` with public fields and
  `RuntimeInfo::new(arch, os)`. The method has a default that fails with `Unsupported`, so a runtime written outside
  this repo compiles unchanged. `DockerRuntime` asks `GET /version` and gives its `Arch` and `Os` as they are (no
  variant: `arm` does not say `v6` or `v7`); a missing or empty one is `Other`. It has no time limit of its own
  (bollard's 120 s apply; wrap the call in `tokio::time::timeout` for a faster answer), and an answering but unhealthy
  Docker (a 5xx, such as an engine that is starting, or a Docker below the API floor) fails with `Other`: a liveness
  check treats every error but `Unsupported` as "not here this round". The fake answers `amd64` on `linux` until
  `set_info(RuntimeInfo)`; `Call::Info` and `Method::Info` record and script the call. Covered by the unit tests in
  `src/fake.rs`, `src/docker/map.rs` and `src/docker/mod.rs`, and by `info_gives_linux_and_the_arch_the_cli_prints`
  in `e2e/tests/suite/container_docker.rs`.
- `cicada-containers`: `DockerRuntime::create` pulls a missing image with the machine's `docker login`, read as the
  Docker CLI reads it, so a private image no longer has to be on the machine first. At each pull it reads
  `config.json` from the folder set with the new `DockerRuntime::docker_config(dir)`, else `DOCKER_CONFIG`, else
  `$HOME/.docker`, and takes the credentials for the image's registry from the helper `credHelpers` names, else the
  `credsStore` helper (`docker-credential-<name> get`, killed after 10 s), else the `auths` entry. A missing or broken
  file, a registry without an entry or a failing helper gives a pull without credentials, so a public image still
  works. The message of a failed pull ends with where its credentials came from or why there were none (such as
  `(no credentials for registry.example.com)`), never with a credential or a helper's output; the error kinds stay
  as they were. Adds `base64` 0.22 to the `docker` feature (already in the tree through bollard). Covered by the unit
  tests in `src/docker/auth.rs` and `src/docker/mod.rs`, and by
  `a_private_image_is_pulled_with_the_login_of_config_json` in `e2e/tests/suite/container_docker.rs`, against a
  `registry:2` with basic auth.

## 0.21.0 — 2026-11-24

The fake and the Docker backend of `cicada-containers` give the same answers, so a host's tests on `FakeRuntime`
predict Docker, a host can follow a container's log in pages while it runs, it can copy a run's files out of a
container within limits, and it can make writable folders in a container before start; a container job gives its
container folders and files before start, and a time limit per request; a `failed` container end says why in a
machine-readable `kind`; and a node on PostgreSQL warns at start when its pool is smaller than its concurrency plus 4.
A breaking release (ADR 0062): `ContainerEnd` gains the public field `kind`, so the contract
hash of every container job type, and of every target type whose messages take a `ContainerEnded`, changes. Hosts on
0.20.x follow "Changing a contract" in the operations guide for those types. The store schema, the HTTP API, the
ready-made actors' contracts and the guarantees are those of 0.20.8.

### Added

- `cicada-containers`: `ContainerRuntime::logs_since(name, cursor, max_lines)` reads a container's log in pages, for a
  host that follows a run's output and keeps it outside the container; call it outside actor turns. A `LogPage` holds
  up to `max_lines` `LogLine`s (`time`, `stream`: `LogStream::Out` or `Err`, and `text` as lossy UTF-8 without the
  newline) and `next`, the `LogCursor` to call again with; `None` reads from the first line. Paging to the end gives
  every line once, in log order, also when several lines share a second or are out of time order within one; a page
  with fewer than `max_lines` lines has read everything there is now, and an empty page keeps the cursor. `LogCursor`
  is opaque: written and read as a string by `Display`, `FromStr` and serde, with a string JSON schema; its form is
  kept across 0.21 patches and a string that is not a cursor is `InvalidSpec`. `max_lines` outside 1 to
  `LOG_PAGE_MAX` (1000) is `InvalidSpec` before any read; a missing container is `Conflict`, a created one gives an
  empty page, and a log Docker cannot read (log driver `none`) is `Other`. The method has a default that fails with
  `Unsupported`, so a runtime written outside this repo compiles unchanged. Docker reads with timestamps from the cursor's
  second and drops lines stamped before it whatever its filter let through; limits: a line stamped before the
  cursor's second but written after the cursor is missed, each read scans the log from that second (poll every few
  seconds), rotation of that second shifts the cursor, a line over 16 KiB comes back in parts, and a cursor after
  January 2038 is `Other`. The fake's `push_log(name, LogLine)` adds timed lines; `set_logs` now gives `Out` lines at
  the Unix epoch, which `logs` and `logs_since` both read; `Call::LogsSince` and `Method::LogsSince` record and script
  the call. Covered by the unit tests in `src/logs.rs`, `src/fake.rs` and `src/docker/map.rs`, and by
  `ContainerDockerTest` in `e2e/tests/suite/container_docker.rs`.
- `cicada-containers`: `ContainerRuntime::copy_out(name, path, limits)` copies the regular files at or below `path`
  out of a created, running or ended container (until `remove`), for a host that collects a run's artifacts after it
  ended or reads a progress file while it runs. It returns a `CopyOut`: `files`, each a whole `CopyFile` with its
  absolute path, contents and permission bits (ready for `copy_in` of another container), and `skipped`, each a
  `SkippedFile { path, reason }`. `CopyLimits::new(max_files, max_bytes)` bounds memory whatever the folder holds: a
  file larger than the bytes left is skipped with its size and the copy goes on; once `files` and `skipped` hold
  `max_files` entries, the next entry stops the copy with one last skipped entry for `path` ("the rest was not
  read"). No link is followed: symbolic and hard links, devices and pipes, and entries outside `path` are skipped with
  a reason. A relative path, a `..` part, `/` or a zero limit is `InvalidSpec` before any call; a missing container
  is `Conflict`, a missing path an empty result. Files written to a tmpfs are gone once the container ended. The three
  types are `non_exhaustive` with constructors; the method has a default that fails with `Unsupported`, so a runtime
  written outside this repo compiles unchanged. Docker's archive is read as a stream and a skipped file's data is
  passed over, not held; an error during the stream fails the whole call. The fake's `put_file(name, CopyFile)` adds
  a file its program "wrote", and the fake walks a folder in Docker's order; `Call::CopyOut` and `Method::CopyOut`
  record and script the call. Covered by the unit tests in `src/copy.rs`, `src/fake.rs` and `src/docker/archive.rs`,
  and by `ContainerDockerTest` in `e2e/tests/suite/container_docker.rs`.
- `cicada-containers`: `ContainerRuntime::make_dirs(name, dirs)` makes empty folders in a created container before
  start, so a program that does not run as root can write there. `CopyDir { path, mode }` (`non_exhaustive`;
  `CopyDir::new(path)` gives mode 0o755) names each folder. Only what is missing is made: a missing named folder with
  its mode, each missing parent with 0o755; an existing folder or parent is left exactly as it is, owner and mode
  included, also when named with another mode. A named folder gets its own mode also when it is a parent of another
  named one, in any order; one path named twice with one mode is made once. A symbolic link at a named path or a parent
  (`/var/run -> /run`) counts as the folder it points to, and a folder below it is made at the target; the walk of one
  folder follows at most 8 links (`Conflict` beyond), and Docker resolves a chain itself, so it counts as one link
  there, while a link loop fails with `Other` from Docker. Before any write: a file, or a link to anything but a
  folder, at a named path or a parent is `Conflict`, as is a container that is not `Created`; a mode above 0o7777, one
  path with two modes, and every path `copy_in` refuses (a mount target, at or below a tmpfs, on a read-only mount, on
  the read-only root) are `InvalidSpec`. The method has a default that fails with `Unsupported`, so a runtime written
  outside this repo compiles unchanged. Docker asks its archive stat for each part of the path from the root and
  uploads a tar of only the missing folders, owned by uid and gid 0, so `/tmp` keeps 1777 and an image's folder keeps
  its owner. The fake keeps the folders it makes; `Call::MakeDirs` and `Method::MakeDirs` record and script the call.
  Covered by the unit tests in `src/dirs.rs`, `src/fake.rs` and `src/docker/map.rs`, and by `ContainerDockerTest` in
  `e2e/tests/suite/container_docker.rs`.
- `cicada-containers`: `ContainerJob::inputs(name, request) -> Result<Inputs, String>` gives the folders and files of
  a container: `Inputs { dirs: Vec<CopyDir>, files: Vec<CopyFile> }` (`non_exhaustive`, `Default`, exported from the
  crate root). The default gives none, so a job compiles unchanged. Like `spec`, it is sync, built at each create and
  never stored, so secret files never reach the store. The container actor now puts the inputs in between create and
  start: it makes the folders (`make_dirs`) and then writes the files (`copy_in`), so a file can go into a fresh
  folder; a container that an earlier step started is adopted without them (see Changed), and one still `Created`
  gets them again. The container ends `failed` with the kind `notStarted` before anything is made for an `Err` from
  `inputs`, a path the shared path rule refuses, and a file at a folder's path or above it (dirs `/a/b`, file `/a`;
  `/out/.` and `/out` are one path), as writing it would replace the folder. It ends so too for `InvalidSpec` from
  `make_dirs` or `copy_in`, `Conflict` while a new inspect still finds the container `Created`, and `Unsupported` from
  `make_dirs` (a runtime without it); the created container stays until `Remove`. Any other error, `Other` and a
  container removed before the copies too, counts and backs off, and the next poll puts the inputs in again. Covered
  by the unit tests of the check in `src/actor/mod.rs` and by `tests/actor.rs`.
- `cicada-containers`: `ContainerEnd::kind: Option<FailureKind>` says why a container ended `failed`:
  `FailureKind::NotStarted` (its key, spec, image, start or inputs were refused, or a runtime call it needs is
  unsupported), `OutOfMemory` (the runtime reported `Failed` with the reason `OUT_OF_MEMORY`), `RuntimeEnded` (another
  failure the runtime reported for a started container, such as a dead one), `Missing` (a started container was
  gone) and `RuntimeErrors` (the settings' maximum errors in a row). Every new `failed` end has one and no other end
  has one; on the wire it is `kind`, camelCase (`notStarted`, …), written as `null` like `code`. A `failed` end saved
  before 0.21.0 decodes with no kind, as none is guessed from its reason; decoding an end, in the state or in a
  `ContainerEnded`, refuses a `kind` on an end that is not `failed`. `FailureKind` is `non_exhaustive`. Covered by the
  unit tests in `src/actor/wire.rs` and by `tests/actor.rs`.
- `cicada-containers`: `ContainerJob::timeout(&self, request, settings: Option<Duration>) -> Option<Duration>` gives
  each request its time limit; the default returns `settings`, the settings' `timeout`, so a job compiles unchanged,
  and `None` is no limit. It is asked once, at the start, and the deadline is saved then and never moves. Covered by
  `tests/actor.rs`.
- `cicada-containers`: `OUT_OF_MEMORY` ("out of memory"), the exact reason of `ContainerStatus::Failed` for a
  container killed for lack of memory; the Docker backend uses it, and another runtime must too for the actor to
  report `outOfMemory`. Docker's reason text does not change. Covered by the unit test of `map::status_of`.
- `cicada-containers`: `ErrorKind::Unsupported`, for a call the runtime does not implement: the trait's default
  `logs_since`, `copy_out` and `make_dirs` return it. `ErrorKind` is `non_exhaustive`, so this is additive; it is not
  retryable. The container actor ends a container at once only on `Unsupported` from `make_dirs`; `Other` (on Docker,
  any daemon 500) backs off and retries like any runtime error. Covered by the unit tests in `src/error.rs` and
  `src/fake.rs`, and by `tests/actor.rs`.
- ADR 0062: a failure kind on the container end.

### Changed

- `cicada-containers`: the container actor's create step begins with `inspect`. A container past `Created`, which an
  earlier step started before its commit was lost, is adopted: saved as started with a fresh deadline, counted from
  the adoption, without calling `spec`, `inputs`, `create`, the copies or `start`, so a job's refusal now never ends a
  running container as `notStarted`. A missing or `Created` container goes through the step as before. Without inputs
  the runtime calls are `inspect`, `create` and `start`. Covered by `tests/actor.rs`.
- `cicada-containers`: a start refused with `InvalidSpec` ends the container `failed` as `notStarted` in the poll that
  tried it, without reading logs, as a refused `create` does; in 0.20.8 it was retried at every poll until
  `max_errors` and ended "the runtime failed N times in a row". A start refused with another kind, such as `Other`, is
  a change too: on Docker such a container then shows as `Failed`, so the next poll adopts it and ends it
  `runtimeEnded`, where 0.20.8 retried the start; a passing daemon error on start now ends the job. Covered by
  `tests/actor.rs`.
- `cicada-containers`: a deadline past the largest instant is capped at `MAX_INSTANT`; it used to be dropped, which
  meant no limit. Covered by `tests/actor.rs`.
- `cicada`: a node that hosts at least one type and starts on PostgreSQL with a pool smaller than its concurrency
  plus 4 logs one `tracing` warning that names the pool, the concurrency, the number it needs and the settings to
  change (`PostgresOptions::max_pool_size` or `--store-pool-size` / `CICADA_STORE_POOL_SIZE`, and
  `NodeConfig::concurrency` or `--concurrency` / `CICADA_CONCURRENCY`), and starts as before. Too small a pool can
  starve lease renewals under load, and a turn whose renewal comes too late is lost. Only at start; SQLite, the
  in-memory store and nodes without types never warn; nodes, or a local `Worker`, that share one `Store` are each
  checked alone, not summed. In `cicada-app` the default pool never warns; a smaller `--store-pool-size` is kept
  and warns. Covered by the unit test of `pool_warning` in `runtime/system.rs` and by
  `a_node_warns_at_start_when_its_pool_is_smaller_than_its_concurrency_plus_4` in `src/tests/postgres/runtime.rs`.

### Changed (breaking)

- `cicada-containers`: `ContainerEnd` has the new public field `kind`: a struct literal or an exhaustive
  destructuring of it no longer compiles; add `kind: None`, or the kind of a `failed` end. The end is in a container
  actor's state and in its end message, so the contract hash of every container job type, and of every target type
  whose messages take a `ContainerEnded`, changes, and a node refuses the old hash.

### Fixed

- `cicada-containers`: one path rule for `copy_in` on every runtime. Both now refuse with `InvalidSpec`, before any
  write, a path equal to the target of a volume or bind mount, with a writable or a read-only root (a file cannot
  replace a mount point: the fake wrote one at `/work`, and Docker uploaded one at a read-only bind's target and
  failed late with `Other`), and a path at or below a tmpfs with no deeper volume or bind mount, on any root (with a
  writable root both wrote it, and Docker then hid it under the tmpfs mounted at start). The fake now also refuses
  `/` and drops empty and `.` parts as Docker does, so `/work/./a` and `/work/a` are one file. Covered by the unit
  tests in `src/paths.rs`, `src/fake.rs` and `src/docker/map.rs`, and by `ContainerDockerTest` in
  `e2e/tests/suite/container_docker.rs`.
- `cicada-containers`: the `Swept` of a Docker `sweep` names only what this sweep removed or skipped. A container
  that was already gone or was being removed by another caller (Docker's 404 or 409) was counted as removed; now it is
  in no list, as a network or volume that was already gone was. A volume's 409 (in use) stays a skip. Covered by the
  unit test of `map::gone`.
- `cicada-containers`: a Docker `list` leaves out a container whose state Docker does not report, as it leaves out
  one removed since the listing, instead of failing the whole call with `Other`. `inspect`, `start`, `copy_in` and
  `stop` of that name still fail with `Other`. Covered by the unit test of `map::summary`.

### Upgrading

- Hosts on 0.20.x with container job types or end targets follow "Changing a contract" in the operations guide for
  each of them: a versioned type name for new runs while the old actors finish, or a maintenance window (stop every
  host of the type, delete its actors and then its registration, start the new hosts). Send `Remove` to a container
  actor before deleting it, or its container stays. Add `kind` to every `ContainerEnd` literal, such as in a host's
  tests.
- A host that told failures apart by the reason text can match `kind` instead; an end saved before 0.21.0 has none.

## 0.20.8 — 2026-11-24

A ready-made permit pool in `cicada-actors`, which caps how many jobs hold a permit at once, and a ready-made job
graph, which runs a graph of jobs given as data. Additive: the existing actor contracts, wire formats, the store schema
and the guarantees are those of 0.20.7.

### Added

- `cicada-actors`: the permit pool (ADR 0058), `PermitsActor` under the fixed type name `cicada.permits`, one actor per
  pool (`Permits::named(pool)`), with no settings. `PermitsState::new(capacity, max_waiting)`, each 0 through 1,000,
  is given at the create; a capacity of 0 pauses the pool. `Acquire { request, to, messageType }` (helper
  `PermitsMsg::acquire(request, &holder, "Permit")`) asks one permit for a request ID and is answered in its own
  commit with a `PermitNotice` (`{"pool", "request", "kind", "grant"}`): `granted` with a grant number that only goes
  up, `queued` in FIFO order, or `refused` when the queue is full, which records nothing; so a gone target records
  nothing either. A repeated `Acquire` answers the same grant, or `queued` with its place kept, to the first target.
  `Release { request, grant }` frees a permit only for its current grant number; `Cancel` only leaves the queue;
  `Resize { capacity }` grants to the queue on a growth and revokes nothing on a shrink; `Audit` sends every request in
  the books a `check`, but none to a holder not yet told of its grant, and frees nothing. A grant made by a release, cancel, resize or freed permit, and each `check`, is
  sent by the pool's own `Notify` in a turn of its own, so a holder that is gone fails only its own notice; the pool
  watches itself, and a grant notice that failed before it ever reached its holder frees that permit and grants the
  next request. Bad input (a capacity above 1,000, a request ID or message type out of bounds, a target or type the send
  refuses) is refused for good with the state unchanged. Its contract is checked against
  `docs/reference/contracts/cicada.permits.json`. Covered by `crates/cicada-actors/tests/permits.rs`, the unit tests in
  `src/permits.rs` and `tests/wire_formats.rs`.
- ADR 0058: a ready-made permit pool.
- `cicada-actors`: the job graph (ADR 0059), `JobGraphActor` under the fixed type name `cicada.job-graph`, one actor per
  run (`JobGraph::named(run)`), with no settings. The graph is the state, given at the create:
  `JobGraphState::new(jobs, max_in_flight, &report_to, report_type)`, each job a `Job::new(name, &target,
  message_type, input)` with `.with_needs([...])`. The constructor and the decoder refuse a cycle (named by a job of
  it), a need that names no job, is repeated or names the job itself, a repeated name, no jobs or more than 100, a name
  that is not 1–64 ASCII letters, digits, `-`, `_` or `.`, a message or report type out of bounds, an input over 16 KiB
  or 32 levels, and an in-flight limit outside 1–100; so a bad graph never becomes a run. `Start` starts every ready
  job in graph order while fewer than `maxInFlight` run, each with a new attempt numbered per job from 1; a second
  `Start` changes nothing. Each job message, a `JobStart { run, job, attempt, input }` sent as the job's message type
  to its target, goes in a `Dispatch` turn of the run's own, so a gone target, a type not registered or a type that
  does not take the message fails only that attempt (`the job message was not sent (<cause>): <error>`) and the run
  goes on. A worker reports `Result { job, attempt, outcome, output, reason }` (helpers `JobGraphMsg::succeeded(&start,
  output)` and `JobGraphMsg::failed(&start, reason)`); only a result for a running job's current attempt is taken, any
  other is refused for good, the state unchanged, and the sender is not checked. A failed job skips every waiting job
  that needs it, directly or through others, in the commit of its failure: each becomes `skipped`, without an attempt.
  The `Dispatch` turn that sends a job message also watches the job's target, and the run unwatches a target once none
  of its running jobs has it: a `Failed` the store wrote about the job message of a running job's current attempt
  ends that job `failed` (`the job message failed at the worker (<cause>): <error>`), and a `Stopped` about the target
  ends every running job dispatched to it (`the worker went away (<cause>)`); other failures of the target and
  notifications a client sent change nothing. A job message, and the failure of its `Dispatch`, count only while the
  job is `running`, and ready jobs start after every change of a job's status. The run cannot see work the worker
  handed on, a worker that lost its outside work without failing the job message, stopping or reporting, or a later
  message of the worker that fails; a target must stay until its job reports, and a result whose turn is rescheduled
  can be overtaken by a `Stopped` of its worker. When no job runs and none can start the run ends `succeeded` when
  every job succeeded, else `failed` when a job failed, else `cancelled`, and sends its
  `JobGraphEnd { run, status, jobs }` to the report target in a turn of its own; anyone may send `Report` again, so a
  report target treats a repeated report for the same run as one. The decoder also refuses a skipped job with an
  attempt or without a failed, skipped or cancelled need, a waiting job with such a need, and an ended run whose
  status does not follow its jobs. Its contract is checked
  against `docs/reference/contracts/cicada.job-graph.json`. Covered by `crates/cicada-actors/tests/job_graph.rs`, the
  unit tests in `src/job_graph.rs` and `tests/wire_formats.rs`.
- ADR 0059: a ready-made job graph.
- `cicada-actors`: cancel in the job graph (ADR 0060). `Cancel { job }` cancels one job, or every job when `job` is
  absent: a waiting job becomes `cancelled`, and so does a running one, whose attempt ends with outcome `cancelled` and
  reason `cancelled`; its target is unwatched unless another running job has it. Ended and skipped jobs and an ended
  run are left alone, so a repeated `Cancel` changes nothing; an unknown job name is refused for good, the state
  unchanged. In its commit the cancelled job's dependents are skipped, ready jobs start in the freed slots and the run
  may end: `cancelled` when no job failed, else `failed`. A job may name a cancel message type,
  `Job::with_cancel_type("CancelBuild")` (field `cancelType`, 1–200 characters without NUL, checked by the constructor
  and the decoder): a cancelled job whose job message was sent is told by the run's own `SendCancel` in a later turn,
  which sends `JobCancel { run, job, attempt }` as that type to the job's target with the shared retry policy, so a
  gone worker fails only that message. A worker's family takes it as e.g. `CancelBuild(JobCancel)`, and may get it
  before the `JobStart` of the same attempt. A job without a cancel type, or not yet dispatched, is not told, and a
  late result of a cancelled job is refused. The state keeps an attempt's outcome as `AttemptOutcome` (`succeeded`,
  `failed` or `cancelled`); a worker's `Result` keeps `JobOutcome`, so it cannot report `cancelled`. The decoder also
  refuses a cancelled job whose last attempt is open or did not end `cancelled`. The end report's contract does not
  change. Covered by `crates/cicada-actors/tests/job_graph.rs`, the unit tests in `src/job_graph.rs` and
  `tests/wire_formats.rs`.
- ADR 0060: cancel and skipped dependents in the job graph; skipped dependents, the end status, the watch on each
  target and what it cannot see; `Cancel` and the cancel message type.
- `cicada-actors`: retry in the job graph (ADR 0061). `Retry { job }` puts one `failed` or `cancelled` job back to
  `waiting`, its output cleared and its attempts kept; in the same commit every `skipped` job waits again and the skips
  are computed anew, so only the dependents skipped because of that job come back, an ended run goes back to
  `running` with no end time, and ready jobs start, the retried one with the next attempt number. A `Retry` is
  refused for good, the state unchanged, for an unknown job, a job that is not `failed` or `cancelled`, a job with a
  `failed`, `skipped` or `cancelled` need ("retry its need first"), and when the attempt number or the count of ends
  is at its largest; there is no cap on retries (the 20 kept attempts stay the bound). A result or failure of a
  replaced attempt is refused or ignored by the existing attempt checks, and a `SendCancel` of a replaced attempt
  still tells its worker. The state counts the run's ends (`ends`, absent: 0; 1 or more once ended), and
  `JobGraphEnd` carries `end`, the number of that end: a report sent again repeats it, a retried run's next end has a
  higher one, and an end reopened before its `Report` ran is never reported, so a report target keeps the report
  with the highest `end` per run. The end report's `reason` of a job is its last attempt's only when that attempt's
  outcome is the job's status, so a retried job cancelled before it started again has none. The decoder lets a
  `waiting` or `cancelled` job keep a last attempt that ended `failed` or `cancelled`, and refuses an ended run with
  `ends` 0. Covered by `crates/cicada-actors/tests/job_graph.rs`, the unit tests in `src/job_graph.rs` and
  `tests/wire_formats.rs`.
- ADR 0061: retry in the job graph, and the end number of its report.

## 0.20.7 — 2026-11-24

A new optional crate, `cicada-containers`, for applications that run containers, with a Docker backend and a
reconciled container actor. The existing crates, actor contracts, wire formats, the store schema and the guarantees are
those of 0.20.6.

### Added

- `cicada-containers` (ADR 0056): the `ContainerRuntime` trait, usable as `Arc<dyn ContainerRuntime>`, with `create`,
  `copy_in`, `start`, `inspect`, `logs`, `stop`, `remove`, `list`, `ensure_network`, `ensure_volume` and `sweep`. Every
  call is safe to repeat: a container is found by its name, `start` never runs an ended container again, and `stop` and
  `remove` of a missing one are `Ok`. `ContainerSpec` keeps secret environment apart (its `Debug` shows only the
  names) and has hardening fields that are off by default. `ContainerStatus` tells a program's exit (`Exited`) from
  the runtime's failure (`Failed`), and `ContainerError` has one `ErrorKind`, of which only `DaemonUnavailable` is
  retryable. The feature `testing` adds `FakeRuntime`, an in-memory runtime without timers that scripts exits, logs
  and failures and records every call. No other crate of the workspace depends on it, which the gate checks. Covered
  by the unit tests of the crate.
- `cicada-containers` feature `docker`: `DockerRuntime::connect()`, the `ContainerRuntime` over the Docker Engine API
  (bollard, without TLS). It reads `DOCKER_HOST` (`unix://`, `tcp://`, `http://`) or uses the local socket, agrees the
  API version on the first call and needs Docker 25 or later. `DockerRuntime::connect_to(host)` takes such a host
  directly, such as the endpoint of the current Docker CLI context, which the backend does not read itself. Containers are found by their exact name, never an ID
  prefix; `create` pulls a missing image (`ImageNotFound` when the registry has none) and keeps an existing container
  even when its image is gone; `copy_in` with a read-only root writes only below a volume or bind mount. The spec's
  limits and hardening map onto Docker's create call (a memory limit allows no swap), and secret environment goes
  only into its body and never into an error message. bollard logs request bodies at `debug` under the target
  `bollard`: keep that target at `info`. Covered by unit tests and the E2E class `ContainerDockerTest`.
- `FakeRuntime::copy_in` also refuses a path with a `..` part and a path on a read-only mount, as Docker does, and
  `CopyFile`'s `Debug` shows its path, mode and length, not its contents.
- ADR 0056: a layer for an external resource that two consumers need may live in an optional crate of this
  workspace; it supersedes in part ADR 0011.
- `cicada-containers`: the container actor (ADR 0057), `ContainerActor::new(job, runtime, settings)`, one actor per
  container under the type name of a host's `ContainerJob`, whose `spec` builds the `ContainerSpec` from a typed
  request in the state at each create, so secrets never reach the store. `Start { to, messageType }`, `Stop` and
  `Remove` only set the goal; one armed `Poll` at a time takes one step toward it, and `Check` brings back a lost one.
  The container's name is the settings' prefix plus the actor's key, so a step that runs again finds it. A container
  ends once, `exited`, `failed`, `timedOut` or `stopped`, with the tail of its logs; the end is saved and sent to the
  target as `ContainerEnded` (`{"key", "end"}`) in the commit that records it, before any removal, and an end the
  target's type refuses is saved as `undelivered`. Runtime errors back off and never fail a turn; at `max_errors` in a
  row a container ends `failed`. Logs and reasons are stored without NUL and bounded. The crate now depends on
  `cicada`. Covered by the tests in `crates/cicada-containers/tests/actor.rs`, with a test job's contract in its
  `tests/contracts`.
- ADR 0057: a reconciled container actor; placement stays with the host.

## 0.20.6 — 2026-11-23

A security update of the dependencies. The libraries' behaviour, public signatures, actor contracts, wire formats, the
store schema and the guarantees are those of 0.20.5.

### Security

- `Cargo.lock` is updated with `cargo update`, which fixes every RustSec advisory with a fixed version: `rustls`
  (RUSTSEC-2026-0346, 0347, 0348, 0349), `rustls-webpki` (RUSTSEC-2026-0382), `webpki-root-certs` (RUSTSEC-2026-0383),
  `libsqlite3-sys` through `sqlx` (RUSTSEC-2026-0459, the C SQLite CVE-2026-119375), `tokio` (RUSTSEC-2026-0386),
  `ctutils` (RUSTSEC-2026-0385) and `url` through `reqwest` (RUSTSEC-2026-0592). This covers the repository's tests, the
  sample service, the E2E images and the `cicada` binary; a project that uses Cicada resolves its own versions, so run
  `cargo update` there too.
- With the `sqlite` feature, `cicada` now needs `sqlx` 0.9.1 and `libsqlite3-sys` 0.39 or later (a new optional
  dependency, `>=0.39`, not used directly), so a consumer cannot resolve the SQLite with the CVE. The upper bound is
  the one `sqlx` sets. A project whose other crates hold `libsqlite3-sys` below 0.39 now gets a resolution
  error. The `postgres` feature does not build it.
- `chrono` is unmaintained (RUSTSEC-2026-0543) and has no fixed version. It stays for now: it is in the public API and
  a dependency of `sqlx`. `deny.toml` accepts that one advisory with its reason.

### Changed

- The full gate checks `Cargo.lock` against the RustSec advisory database with `cargo deny check advisories` (check
  `advisories`). Full gate runs now need `cargo-deny` 0.20.2 (`cargo install cargo-deny --locked --version 0.20.2`)
  and network access, as the database is fetched each run; without either the check fails. The manual Linux
  workflow installs it.
- `schemars` 1.2.3 is the new floor. The contract texts that `GET /types` serves, and `docs/openapi.json`, change key
  order only; the contract hashes are the same.

## 0.20.5 — 2026-11-17

A documentation correction. Only a doc comment changes: the libraries' behaviour, public signatures, actor contracts,
wire formats, the store schema and the guarantees are those of 0.20.4.

### Changed

- The rustdoc of `ActorSystem::nodes` no longer says the node view is for display only. It is for observability,
  including node-down alerts (`NodeDownMonitor` reads it through `NodeSnapshotSource`). The node table grants no
  authority to claim, move or fence actors; leases decide who runs what.

## 0.20.4 — 2026-11-17

The sample also hosts the ready-made heartbeat watch. Only the sample service changes: the libraries, actor contracts,
wire formats, the store schema and the guarantees are those of 0.20.3.

### Added

- The shared sample catalog hosts `HeartbeatWatchActor` as `cicada.heartbeat-watch`, so both `cicada-sample-service`
  and `cicada-sample-executor` (and the image) register it under `--types all` or its exact name; `none` leaves it
  out. Neither creates a watch, sends a `Beat` or arms a timer at start: a caller creates
  `cicada.heartbeat-watch/<name>` with its silence, target and alarm, makes sure the target exists, and sends `Beat`
  to begin a round. Its contract is the ready-made one, pinned in `sample-service/tests/contracts` and
  `tests/contract-texts.json`. The sample still does not host `NodeDownMonitor`, and an HTTP worker gains no node
  heartbeat. Covered in `sample-service/tests/sample_service.rs`.

## 0.20.3 — 2026-11-16

A stock Rust HTTP worker of the sample, for interop tests of other worker ports. Only the sample service changes: the
libraries, actor contracts, wire formats, the store schema and the guarantees are those of 0.20.2.

### Added

- `cicada-sample-executor`, a second binary of `cicada-sample-service` and of its image (`Sample::executor`): the
  sample's types, routes, `API_TOKEN` and variables with App's whole command line, but no `on_start` hook, so
  `cicada-sample-executor worker --server <url> --types <names>` runs the sample's types over HTTP (ADR 0042). It
  creates no singletons and no dead-letter watch in any mode: a node of `cicada-sample-service`, or an API caller,
  supplies those a workload needs. The client token is `CICADA_TOKEN` (or `--token`); `API_TOKEN` only protects a
  served API. An explicit `--server` wins over the image's `CICADA_STORE`. In the image, override the entry point
  with `/usr/local/bin/cicada-sample-executor` and turn the HTTP health check off (`--no-healthcheck`); the default
  entry point stays `cicada-sample-service serve`. `cargo run -p cicada-sample-service` still runs the node
  (`default-run`). Covered in `sample-service/tests/sample_service.rs` and the E2E class `SampleExecutorTest`.

### Changed

- The sample's E2E actors (`e2e.slow`, `e2e.counter`, `e2e.recorder`) and its `EffectLog` record `Turn::node_id`,
  the claim owner that ran the turn: a node's id, or a worker's id over HTTP. Their constructors and
  `Catalog::new`/`with_fakes` keep their node-id argument but ignore it. `--node-id` now works for the sample and wins
  over `CICADA_NODE_ID`; before, a differing flag failed the start. The sample no longer sets `node_config`.

## 0.20.2 — 2026-11-16

Node-down alerts (ADR 0054): an optional ready-made monitor that reads the node table and publishes one `NodeDown`
through a topic for each node row it sees silent for two minutes. Additive: existing actor contracts, the node table,
the store schema, the heartbeat and the guarantees are those of 0.20.1.

### Added

- `cicada_actors::node_down`: the singleton `NodeDownMonitor` (`cicada.node-down/_`) with `NodeDownMonitorActor<S>`,
  which reads a `NodeSnapshotSource`. `ActorSystem` is one: `nodes()`, then `now()` as the sample time; a system built
  on the same store and never started reads without a row or heartbeat of its own. Hosts create the topic and the
  monitor (`NodeDownMonitorState::new(topic)`) explicitly, unpinned, and send `Rearm`; nothing starts it by itself.
- Every ten seconds a guarded self `Poll` reads the source within five seconds. A row whose `lastSeen` is 120 seconds
  or more before the sample time is dead, the node view's threshold. Each dead row whose `startedAt` and `lastSeen`
  are not yet in `alerted` publishes one `NodeDown { node, startedAt, lastSeen, detectedAt, hostname?, cicadaVersion?,
  label? }` as a `TopicMsg::Publish`, in the same commit as its watermark and the next poll, so a row dead at the first
  sample alerts too, a newer heartbeat or a new row alerts again, and competing hosts or a replayed turn publish an
  observed pair once. Absent and alive rows leave `alerted` without any recovery message. Subscribers declare
  `NodeDown(cicada_actors::NodeDown)`.
- A failed, slow, invalid or oversized read (more than 1000 rows) publishes nothing, keeps `alerted`, records only
  `lastProblem` (`unavailable`, `timeout`, `invalid`, `tooManyNodes`) and polls on: the turn succeeds, so a watch is
  not told. One bad row refuses the whole sample. A topic that is gone fails the poll for good (`commit`) with nothing
  kept; repair the topic and send `Rearm`, which keeps `alerted`.
- Its contract is `docs/reference/contracts/cicada.node-down.json`. Example 22, `examples/src/node_down.rs`: a bounded
  pager subscribed to the topic, watching the monitor for `Failed` and `Stopped`, with `lastProblem` read separately
  and a repair. Both consumer skills route to the example and the contract.
- Limits: a sampled observation, not proof of a machine failure. Rows removed by a clean stop or the daily pruning
  before a read leave no trace, a heartbeat can race an alert, workers have no row, flapping nodes alert on each
  episode without a rate limit, and the node read includes every running turn. Never use an alert to move, claim or
  fence work.

## 0.20.1 — 2026-11-16

Example 21, a pinned process run (ADR 0053): a worked actor and host that start, observe and stop a local child
process on the node the actor is pinned to. An application example only: library APIs, wire formats, contract hashes,
store schema and guarantees are those of 0.20.0.

### Added

- `cicada_examples::pinned_process`: a `Run` actor per never-reused key keeps the desired intent (`Idle`, `Run`,
  `Stop`) and the observed status; a `ProcessHost`, built before the `ActorSystem` and given to `RunActor`, keeps the
  child handles. `create_run` creates a run pinned to the host's stable node id and checks the stored pin. The host
  has a fresh epoch per process start; a turn on another node id or epoch acts on no process and marks a run that was
  not terminal `NeedsReconciliation`, which is absorbing. At most 32 host slots, each reserved before spawning, start
  their child at most once and are kept, with terminal tombstones, for the host's life: a turn replayed after a lost
  commit finds the slot instead of spawning again, and `Stop` before `Start` reserves a terminal slot. A `pump` sends
  the latest observation of each slot as an ordinary message with a stable id per run and revision, retried every
  second with a five-second bound. `reconcile` refreshes the node's runs at startup; `ProcessHost::shutdown` closes
  admission, stops and reaps every child and waits for delivery within a bound, and reports what it could not
  confirm; `run_host` runs a host's work with the pump attached and then always shuts down in order (host cleanup,
  system, pump, store), an error included, which it still returns. No PID adoption, automatic restart, process groups or exactly-once spawn; an abrupt host death may leave an
  orphan.
- `OsBackend` and `child_main`: the real adapter runs only the example executable's inert child mode, with no shell,
  and waits for every exit after a kill. The demo host `examples/examples/pinned_process.rs` takes a SQLite file, a
  node id and an explicit `--new-run KEY`, cleans up through `run_host` also when an error ends it, and exits with a
  failure when its cleanup is not confirmed.
- Tests: `examples/tests/pinned_process.rs` on virtual time with a fake backend (repeats, races, capacity, stale
  observations, a lost commit replayed through a `TurnSource` wrapper, a lost `Stop` commit, uncertain sends,
  deletion, a new host epoch and shutdown) and `examples/tests/pinned_process_os.rs`, real children on Linux and
  macOS only. Both consumer skills route to the example.

## 0.20.0 — 2026-11-16

Pinned actors (ADR 0052): an actor may be pinned at creation to one claim owner, a node's `node_id` or a worker's
`worker_id`, and only that owner runs its turns. A breaking release: public structs gain fields, and store schema 8 is
not safe with running 0.19 nodes on the store, which must be stopped before the first 0.20 node opens it. The HTTP
fields are optional and old request and fixture bytes are unchanged. Guarantee S8 is qualified: progress needs an
eligible executor that keeps polling the type. No actor contract or contract hash changes.

### Added

- `CreateOptions { pinned_to: Option<NodeId> }` (`cicada::CreateOptions`, also in the prelude), default unpinned, with
  `CreateOptions::new().pinned_to(id)`. A pin is at most 200 characters with no NUL, like a claim owner; it need not
  name a node row, a started node or a known worker.
- Options variants that keep the existing calls unpinned: `ActorSystem::create_with`, `create_actor_with` and
  `create_actor_checked_with` (options before the check); `Turn::create_with`, `create_child_with`, `create_raw_with`
  and `create_child_raw_with`; `ActorClient::create_with` and `create_actor_with`. A pin beyond its bounds is
  `OpError::Invalid` before anything else, `TurnError::Invalid` at the turn's call, and `400` over HTTP.
- `Turn::node_id()`: the claim owner running the turn, a node's `NodeConfig::node_id` or a worker's
  `WorkerConfig::worker_id`, never the id of the server a worker leases through. Pin a child to the executor with
  `CreateOptions::new().pinned_to(turn.node_id().clone())`; children never inherit a pin.
- Claims and next-due reads of every store (memory, SQLite, PostgreSQL) admit an actor only when it is unpinned or
  pinned to the claim owner, before the claim's limit and before any change to it: a pinned actor whose owner is
  away keeps its messages pending with no new attempt, lost claim, failure or reassignment, and does not wake other
  nodes' dispatch loops. `ActorSystem::lease_turns` filters by the requesting worker's id, not the server's; the
  worker needs no node row.
- `pinnedTo` on the HTTP create body (`PUT /actors/{type}/{key}`) and on a returned create of `POST /leases/complete`
  (`ActorCreateView`), optional, absent or null for none and left out when none; `pinnedTo` on actor views (`GET
  /actors/{type}/{key}`, the state listing), `null` when unpinned. `ActorSnapshot` and `ActorRecord` gain `pinned_to`.
  A create answers only `address` and `created`: read the actor to see the pin it has.
- `cicada create actor --pinned-to ID`; `PINNED-TO` in the wide actor table after `PARENT`, `pinnedTo` in `describe
  actor` and JSON, and a `Pinned to` fact on the TUI's actor screen. The mailbox listing shows what a pinned actor's
  messages wait for.
- Portable fixtures: a valid pinned returned create, a `pinnedTo: null` input, and pins refused at decode (blank, not
  a string) and at conversion (too long, NUL), with the coverage label `TurnEffectsView.creates(pinned)`. Every older
  case keeps its bytes.

### Changed (breaking)

- `ActorCreate`, `ActorRecord`, `ActorSnapshot`, `CreateActorRequest`, `ActorCreateView` and `ActorView` have a new
  public field: a struct literal or an exhaustive destructuring of them no longer compiles; add `pinned_to: None`.
- An existing actor keeps its pin on a repeated create, whatever the new create asks; delete it to choose another.
  Creates of one address in one turn or one commit must agree on their pin as on being a child (`None` against
  `Some` included): `TurnError::Invalid` at the call, `StoreError::Invalid` for a commit.
- Store schema version 8 adds the nullable column `actors.pinned_to` and the partial index `actors_pinned_due` on
  SQLite and PostgreSQL; every existing actor is unpinned. Not additive in effect: a running 0.19 node keeps claiming
  pinned actors whatever their pin, and a 0.19 binary refuses the migrated store.

### Upgrading

- Stop every 0.19 node and worker that opens the store directly before the first 0.20 node migrates it; do not mix
  old and new direct-store nodes. Upgrade every HTTP server before a client or worker sends `pinnedTo`: a 0.19 server
  refuses the field as unknown. A 0.19 worker over HTTP keeps working against an upgraded server, which enforces the
  pins on its leases, but cannot create pinned actors.
- A pin names a routing identity, not an authenticated machine. Give each node or worker that owns pinned actors a
  stable id that is unique among live executors (`--node-id`, `CICADA_NODE_ID`, `WorkerConfig::worker_id`): a random
  id after a restart leaves the old pins waiting, and two executors with one id may run the same actor on different
  machines, one at a time. A pinned actor whose owner never comes back waits forever: find it with `get actor` or the
  state listing, look at its `mailbox`, and delete it, then create it again with another pin if it should run.

## 0.19.4 — 2026-11-16

A review mode for the `actor-design` skill: it checks an existing actor design or implementation and saves a report
of grounded findings. Library APIs, wire formats, contract hashes, store schema, guarantees and the skill archive's
layout are those of 0.19.3.

### Added

- Review mode in `skills/actor-design/SKILL.md`, chosen when the user explicitly asks to review an existing design
  or codebase; a new system still gets the nine-step interview, an unclear request one question, and a combined
  request the review first. A review runs four phases (scope and evidence, reconstruct, check, report) without
  waiting for confirmation after each. It keeps the consumer's code revision, the resolved Cicada source and the
  skill versions apart, reads only (it runs no consumer builds, tests or scripts), draws the actors, messages, watches
  and selected stories from evidence before judging, and tags each claim as observed, documented, inferred or
  unknown. It checks seven areas against the resolved design guide and reports findings with an id (R-001 and on),
  a priority by impact, evidence, a counterexample trace, the smallest useful correction and a check; missing
  requirements become open questions and documented tradeoffs that meet them stay accepted. The report goes to
  `docs/design/actors-review.md` by default, beside and never into the design document; it resumes a report of the
  same system and scope, marks findings stale when the evidence changes, keeps ids, history and the user's notes, and
  resolves a finding only with new evidence. It never fixes code.

### Changed

- The `cicada` skill's description and introduction name both modes of `actor-design`.

## 0.19.3 — 2026-11-16

A second consumer skill, `actor-design` (ADR 0051), that works out an actor design with the user step by step and
saves it with Mermaid diagrams, shipped beside `cicada` in the plugin and the release archive. Library APIs, wire
formats, contract hashes, store schema and guarantees are those of 0.19.2.

### Added

- `skills/actor-design/SKILL.md` and `assets/design-template.md`: a method for designing a consumer system with
  actors. It follows the nine steps of the resolved source's design guide, asks at most three questions at a time from
  concrete stories, shows each step's written result and drawing and waits for the user's confirmation before the
  next step, unless the user explicitly delegates choices, which it records with their reasons. It saves the design
  in `docs/design/actors.md` of the consumer's repository by default (a path the user or the repository names wins),
  never overwrites another design, and resumes from the saved Progress. It uses the sibling skill's resolver and
  routes to the design guide of the resolved source, falling back to an older source's core design section; it never
  implements, commits or deploys. In the plugin it is `/cicada:actor-design`.
- The release archive `cicada-skill-vX.Y.Z.tar.gz` holds both skill folders, `cicada/` and `actor-design/`, each with
  the same `SOURCE.json` (format 2). A tag's `scripts/skill-archive-format` selects the format; a tag without it and
  without `skills/actor-design` still packages the single-skill layout, byte for byte as before.

### Changed

- The `cicada` skill names `actor-design` for design work and keeps building, testing and operating.
- `scripts/release-check.sh` requires format 2 and the version of `skills/actor-design/SKILL.md`; `gate.py release`
  takes the archive layout from the pinned commit and checks every `SOURCE.json` and every packaged file against it.

## 0.19.2 — 2026-11-16

A guide to designing with actors (ADR 0050), `docs/guide/design.md`, and its worked example: a websocket session whose
durable output any host can serve. Library APIs, wire formats, contract hashes, store schema and guarantees are those
of 0.19.1.

### Added

- `docs/guide/design.md` explains actor design from problem to design: actors and identities, protocols,
  coordination and time, recovery, outside systems, a pattern chooser and design mistakes, each as a principle and
  then its Cicada form. The consumer skill routes to its sections.
- Example 20, `examples/src/websocket_session.rs`: a session actor keeps at most 100 unacknowledged frames of at most
  1024 UTF-8 bytes, numbered 1 to 2^53 − 1, and a client's cumulative acknowledgement. `Enqueue` appends a frame and
  replies `Accepted` with the request's message id in one commit, or replies `Rejected` (`InvalidBody`, `BufferFull`,
  `SequenceExhausted`) and changes nothing; a reply actor that is gone rejects the commit. A NUL character is refused
  by Cicada at the send, before the session. Only a valid, larger `Ack` of the stream trims frames.
- `websocket_session::bridge` serves one socket from the session's state with public `ActorSystem` reads, sends and
  the change feed: it subscribes before reading, reads again on a hint of its exact address, on `Resync` and every
  second, writes `Hello` and the frames above the acknowledgement, and forwards the client's `Ack`. Socket writes and
  store operations have a five-second bound; a missing session, a store error or uncertain send, the end of the feed,
  invalid input, the client's close and shutdown end both halves. `websocket_session::adapter` puts it behind the
  axum route `GET /sessions/{stream}/socket` with 4 KiB inbound and 16 KiB write limits; the route authenticates
  nobody. `cargo run -p cicada-examples --example websocket_session` serves it on loopback with a minimal browser
  client that keeps a per-stream cursor, acknowledges replays again and reconnects on a gap. Its producer emits
  through `websocket_session::emit_and_wait`, which waits for the outcome of its own Emit before it reads the answer.
- The examples crate turns on axum's `ws` feature and tests with `tokio-tungstenite` 0.29.
- The demo's browser client is tested as it ships, in Node against fake sockets, storage and timers
  (`examples/tests/websocket_session_client.test.mjs`, Node's built-in test runner, no packages): replays, gaps,
  old sockets, a lost cursor and other streams. The gate's full checks run it as `client-tests`, which needs
  Node 26 or later as `node` on PATH and fails when it is missing or older. No real browser runs.

## 0.19.1 — 2026-11-16

The consumer skills as a Claude Code plugin (ADR 0049), beside the release archive. Existing APIs, wire formats,
contract hashes, store schema and guarantees are those of 0.19.0.

### Added

- This repository is a Claude Code plugin marketplace: `.claude-plugin/marketplace.json` lists one plugin, `cicada`,
  whose source is the repository root, and `.claude-plugin/plugin.json` gives its name and the workspace version. Its
  skills are those of `skills/`, found by Claude Code, so far `/cicada:cicada`. A consumer with Git read access runs
  `claude plugin marketplace add helico-tech/cicada.rs` and `claude plugin install cicada@cicada`, and updates it
  through `/plugin`. The plugin follows `main`: Claude Code fetches it again when the version changes, so it can hold
  documentation changed after the release tag, and it has no `SOURCE.json` (the resolver reports `revision_match`
  `unknown`). The release archive `cicada-skill-vX.Y.Z.tar.gz` is unchanged and stays the tag-exact way.
- `examples/tests/skill.rs` checks the exact fields of both manifests, the plugin's version against the workspace,
  that each folder of `skills/` is a real folder with a regular `SKILL.md`, that the manifests and `skills/` hold no
  symbolic link, and that no other plugin component (`commands/`, `agents/`, `hooks/`, `bin/`, `output-styles/`,
  `workflows/`, `themes/`, `monitors/`, `.mcp.json`, `.lsp.json`) is at the root. `scripts/release-check.sh` refuses
  a release whose `plugin.json` is missing, is not JSON or has no string version equal to the tag's, before anything
  is pushed.

### Changed

- The skill `cicada` says to replace `<this skill's folder>` with the actual installed path, quoted for the shell, and
  explains updates of a plugin through `/plugin` apart from the staged comparison of an archive, with how to switch
  from an archive without losing local edits or keeping two copies.

## 0.19.0 — 2026-11-15

Node details (ADR 0048): each node row says which machine wrote it, with which Cicada version, under which label.
A breaking Rust API release, as public structs gain fields; the HTTP fields are optional. The store schema goes to
version 7 with an additive migration: running 0.18 nodes keep working, but cannot restart once a 0.19 node has
migrated the store. No guarantee, actor contract or contract hash changes.

### Added

- `NodeConfig::label: Option<String>`, default `None`: a free text shown with the node, such as the application and
  its version. A present label is not blank, has no NUL and at most 200 characters (`limits::MAX_ID_LENGTH`), and is
  kept as given, outer spaces included; a bad one is a `StartProblem` at `initialize` and `start`.
- A started node captures its OS host name once (through `whoami` 2), the version of the `cicada` crate and its label,
  and writes them with every heartbeat of its row, replacing all three, absent ones included. A host name the lookup
  cannot give, or one that is blank, holds NUL or is longer than 200 characters, is left out with a debug diagnostic
  that does not show it; the node still starts. `initialize`, `run_due_turns` and a worker over HTTP add no row.
- `NodeInfo` and `NodeView` gain `hostname`, `cicada_version` and `label`; `GET /nodes` and `GET /nodes/{node}` serve
  them as `hostname`, `cicadaVersion` and `label`, left out when absent. A client of this version reads an older
  server's nodes with the fields as `None` (an explicit `null` too); older clients ignore them.
- `cicada-app`: `--node-label` and `CICADA_NODE_LABEL` for `serve` and a worker on the store, with the precedence flag,
  variable, host `NodeConfig`, none. The variable is read as given: empty is unset, blank is a usage error (exit code
  2), as is a bad flag; a bad label of the host fails the start (exit code 1). A worker over HTTP refuses the flag, as
  it has no node row, and reads neither the variable nor the host's label.
- Node tables show `HOSTNAME` after `ID`; `-o wide` adds `VERSION` and `LABEL` after `HOSTED`, and the TUI's nodes
  screen does the same. Missing values show `-`. In table and TUI cells, newline, carriage return and tab show as `\n`,
  `\r` and `\t` and other control characters as `\u{hex}`; JSON and YAML keep the text.

### Changed (breaking)

- `NodeConfig`, `NodeInfo` and `NodeView` have new public fields: a struct literal without `..NodeConfig::default()`
  (or the like), or an exhaustive destructuring, no longer compiles.
- Store schema version 7 adds the nullable columns `hostname`, `cicada_version` and `label` to `nodes` on SQLite and
  PostgreSQL. 0.18 nodes that are already running keep reading and writing their rows (their inserts have no details,
  their updates leave another writer's details in place); a 0.18 binary refuses to open the migrated store, so an old
  node cannot restart or roll back. Replace the old nodes during the rollout.

### Upgrading

- The details are display only. They do not make an id unique or authenticated: nodes that share an id share a row,
  the last writer's details win and a 0.18 writer can leave stale ones. Choose a stable id per node with
  `CICADA_NODE_ID` where an id should survive a restart, and keep it unique among live nodes. The host name can
  identify a machine or container; the API's existing access control covers it.

## 0.18.12 — 2026-11-15

Conformance fixtures for other implementations. Existing APIs, wire formats, contract hashes, store schema and
guarantees are those of 0.18.11.

### Added

- Fixed, language-neutral test data in `fixtures/` (ADR 0047), format 1, with JSON inputs and bodies as strings:
  `canonical.json` (canonical JSON text and its `sha256:` hash, including key order by UTF-8 bytes, escapes, integer
  and float spelling, subnormals, underflow, notation thresholds and refused inputs), `contracts.json` (contract
  hashes with schema descriptions stripped at every traversed keyword and kept in data and names), `leases.json`
  (the four lease operations' bodies: Rust's exact output, normalized inputs, and refusals at decode or conversion)
  and `status.json` (success statuses, the status classification of ADR 0040 and the registration exceptions).
- `crates/cicada/tests/fixtures.rs` checks them against the implementation through public APIs, with no variable that
  rewrites them; the client's status classifier, its scripted operations and the OpenAPI document read
  `status.json` too. A dependency update that changes a covered canonical output or hash now fails these tests.
- Each release from this one on carries `cicada-fixtures-vX.Y.Z.tar.gz` (the four files and `SOURCE.json`) beside
  the skill archive. `scripts/package-fixtures.py vX.Y.Z DIR` packages it from the tag's Git objects. Earlier
  releases gain no fixture archive.

### Changed

- The canonical number format is a written rule (ADR 0047): shortest round-tripping digits, fixed notation for decimal
  exponents -5 to 15 with `.0` on whole floats, a signed exponent otherwise, `-0.0` kept. It is the output of
  serde_json 1.0.151 (`preserve_order`, `float_roundtrip`) with zmij 1.0.23, which the fixtures record; other
  dependency versions or merged features are not certified by them.
- `scripts/gate.py release` packages and verifies both archives before the tag push, saves their name-to-digest map
  and refuses a retry whose archives differ, resumes a draft holding any matching subset by uploading only the
  missing assets, and refuses the single-digest state of an earlier driver (resume that release with its own pinned
  runner). The skill archive's command, contents and bytes are unchanged.

## 0.18.11 — 2026-11-15

A documentation correction. Existing APIs, limits, wire formats, contract hashes, store schema and guarantees are those
of 0.18.10.

### Fixed

- The descriptions of `LeaseTurnsRequest`, in rustdoc and in `docs/openapi.json`, state the limits the core checks:
  1 to 100 types (`MAX_LEASE_TYPES`) and 1 to 100 turns (`MAX_LEASE_BATCH`), not 1 to 64 types and 1 to 256 turns.
  The test `the_lease_request_documents_the_limits_the_core_checks` compares the generated descriptions with these
  constants and `MAX_ID_LENGTH`.

## 0.18.10 — 2026-11-15

A new ready-made actor. Existing APIs, wire formats, contract hashes, store schema and guarantees are those of 0.18.9.

### Added

- The heartbeat watch, `cicada.heartbeat-watch` (`HeartbeatWatchActor`, `HeartbeatWatch`, `HeartbeatWatchMsg`,
  `HeartbeatWatchState`, `HeartbeatStatus`, `InvalidHeartbeat`): it sends a stored alarm to a fixed target once when no
  `Beat` was handled within the allowed silence. A create leaves it `waiting`; each `Beat` sets `lastBeatAt` to its
  store time, replaces the armed `Expire` at `lastBeatAt` plus the silence, and makes it `alive`; the due `Expire`
  sends the alarm (watch as sender, new id, the stored retry policy, no correlation id) and makes it `silent` in one
  commit, arming nothing more. The next `Beat` starts a new round without a recovery message. `Rearm` arms a lost or
  failed timer again at the unchanged deadline. Only the armed timer sent by the watch counts: an early one re-arms at
  the deadline, any other `Expire` changes nothing. Its contract is `docs/reference/contracts/cicada.heartbeat-watch.json`.
- Example 19, `examples/src/heartbeat.rs`: a nightly import watched with 25 hours of allowed silence, a first `Beat`
  with the create, one alert for a missed night and a silent recovery.

## 0.18.9 — 2026-11-15

A tooling release: local check evidence and a local release driver replace the automatic GitHub Actions (ADR 0046).
The library, its API, wire formats, contract hashes, store schema and guarantees are those of 0.18.8.

### Added

- `scripts/gate.py`, the local gate. `run [REV] [--e2e]` checks a commit's exact tree in an owned worktree
  (`.claude/worktrees/gate`) with the full policy of `scripts/gate-policy.json`: the Python tests, fmt, clippy with
  warnings denied, the workspace tests with PostgreSQL, each feature of cicada and cicada-app alone, the app-alone
  tests and the dependency-tree checks of `scripts/ci/`, and with `--e2e` the unfiltered Docker suite. It records the
  result in Git's common directory, shared by every worktree. A commit that changes only allowed Markdown since a
  full pass gets fresh docs checks instead. `status` explains the evidence of a commit; `recover` clears an
  interrupted run once its processes are gone.
- A pre-push hook (`install-hook`) that refuses a push of `main` without passing checks for its tip, a non-fast-forward
  or deleted `main`, a new version tag without checks, E2E and a place on the remote's accepted `main`, and any change
  to an existing remote version tag.
- `release vX.Y.Z [REV]`, the local release: it pins one commit, checks the version and dated notes, obtains the
  evidence, pushes `main` without force, tags, packages the skill, pushes that tag object and publishes a verified
  draft GitHub Release. A retry resumes the same commit and tag object; a published Release is never changed.

### Changed

- `.github/workflows/ci.yml` runs only when started by hand (`python3 scripts/gate.py checks --scope full`, no evidence,
  no Release). `.github/workflows/release.yml` is gone: pushes, pull requests and tags start no workflow.
- `scripts/release-check.sh` checks the checked-out commit, refuses a local tag that points elsewhere, and gains
  `--before-push`, which leaves out the fetch and the `origin/main` ancestry check.
- A remote version tag is never deleted, also when its checks failed before a Release existed (ADR 0046 supersedes
  that part of ADR 0016): a correction is a new version.

## 0.18.8 — 2026-11-15

A test-only release: a flaky PostgreSQL test is fixed. The library, its API, wire formats, contract hashes, store
schema and guarantees are those of 0.18.7.

### Fixed

- `a_cancelled_close_leaves_its_resources_to_the_final_drop_or_the_runtime_teardown_without_a_task`
  (`crates/cicada/src/tests/postgres/changes.rs`) no longer fails at random under load. It counted every task on the
  store's runtime, so sqlx's own connection cleanup could raise the count. It now polls and drops the cancelled close
  on a fresh current-thread runtime that runs nothing else, and asserts that no task is alive there; the store's
  setup, the ended change stream and both teardown legs stay on the store's runtime. The SQLite twin is unchanged.

## 0.18.7 — 2026-11-14

A correction-only release: the record of the dead-letter upgrade of 0.17.0 now says what its check does. The library,
its API, wire formats, contract hashes, store schema and guarantees are those of 0.18.6.

### Fixed

- The doc comment of `check` in the worked example `examples/src/dead_letters_upgrade.rs` no longer suggests that the
  schema and the decoder agree on every field of a letter: neither `check` nor `ActorSystem::create_actor` evaluates the
  JSON Schema, and the cited test covers the payload fields of a letter only. Before anything is deleted, `check`
  refuses NUL in strings or keys, a state deeper than 64 levels, a state that does not decode as the 0.17.0 state, and an
  address repeated in the export.
- The archived design and task 3 of `deep-failure-notices` carry marked correction notes saying the same; their
  original wording is kept.

## 0.18.6 — 2026-11-14

A process-only release: the loops that build Cicada hand user documentation to the docs session (ADR 0045). The
library, its API, wire formats, store schema and guarantees are those of 0.18.5.

### Changed

- Every built change carries `docs-handoff.md` for the docs session instead of editing the README or the handwritten
  docs, and a doctest, link, guarantee row or skill route that needs new prose keeps the change from landing until the
  docs session prepares the fix (the build, design and review loops and `openspec/config.yaml`).

## 0.18.5 — 2026-11-14

A cancelled flow runs its cleanup: `Flow::on_cancel`, with a default that does nothing, runs over a journal of its own
after `FlowMsg::Cancel`, and only its success marks the run cancelled (ADR 0044). This completes the six changes of
the flow line. The flow wire and every contract hash stay those of 0.18.0; no store schema and no guarantee changes.
Every host and worker of a flow type must run this release and the same cleanup code before a run is cancelled: a node
of 0.18.4, or code with the default cleanup, marks the run cancelled without the intended cleanup, although the hash
is the same, and a node of 0.18.4 refuses the messages of a cancelling run for good. Runs cancelled by 0.18.4 stay
cancelled and are never finalized.

### Added

- `Flow::on_cancel(&self, cleanup: &mut Cleanup, input)`, a default-success hook that runs only after a `Cancel`
  reached a running run, never after the flow returned `Ok`. `Cleanup` dereferences to a `Step` over the cleanup
  journal: the same helpers, rules and limits (one new entry per turn, the first latch wins, 1000 entries), ids of
  its own and step keys `<run address>/cancel/<id>`; request routes and the self-send check use the run's own address.
- Lookups of the main journal, which never changes once the run is cancelling: `Cleanup::main_key(id)`, the key a main
  step had or would have had, also when its result was never saved; `main_result::<T>(id)` and `main_request::<T>(id)`,
  `None` only when the id is absent; `main_reply(&request)`, a `#[must_use]` `MainReply::{Open, Replied(T), Closed}`
  decided by closed main waits alone, so it stays the same through the cleanup whatever replies come; and the raw
  `main_journal()`. Lookups consume no id and may repeat; a wrong kind, an invalid id or a decoding failure fails the
  turn for good, also when the error is caught, and a caught decoder panic leaves the in-progress marker.
- A cleanup wait may name its own earlier requests and main requests that no closed main wait names (never-awaited
  ones and those of an interrupted open main wait), mixed in one join; each in one cleanup wait only. A request a
  closed main wait names, a first wait's loser included, needs a new cleanup request.
- Example 18, `flow_resources`: a job that owns a sandbox; cancelled, its cleanup stops the sandbox, waits for the
  confirmed removal, releases the capacity by the reservation's key and reports, with owners that fence late work.

### Changed

- `FlowMsg::Cancel` to a running run now marks it `cancelling` and arms a cleanup `Continue` (due now, with
  `Flow::retry` and the `Cancel`'s correlation id) in its one commit, still without running flow code or cloning the
  input; the cleanup marks it `cancelled` in later turns, at least one even with the default hook. A status read right
  after the `Cancel` turn shows `cancelling`, and the `Cancel` turn now sends one message, the cleanup's `Continue`.
- A cancelling run executes only its cleanup: the phase comes from the saved status, so a stale, retried, redriven or
  public `Continue` normalizes and runs the cleanup, never main code. `Cancel` to a cancelling run is a no-op that
  writes nothing and resets no retry; a public `Continue` recovers a cleanup whose continuation failed, ran out of
  attempts or was purged. A `Reply` to a cancelling run passes the same checks, closed waits of both journals included,
  and closes only the open cleanup wait; the main journal is never rewritten.
- A cleanup that fails keeps the run cancelling with its cleanup progress. A cleanup's last send and the status
  `cancelled` are separate commits. Delete stays immediate and runs no cleanup. Successful finalization sends no
  synthetic `Failed` or `Stopped`.
- The descriptions of `FlowMsg` and `FlowStatus` in every flow contract describe the new behaviour; the schema and
  the contract hashes are unchanged.

## 0.18.4 — 2026-11-14

A host can cancel a running flow: `FlowMsg::Cancel` ends the run in one turn of its own, keeping its progress (ADR
0044). The flow wire and every contract hash stay those of 0.18.0; no store schema and no guarantee changes. A node of
0.18.3 refuses `Cancel` for good and leaves the run running, so upgrade every host and worker of a flow type before
sending it. This release runs no cleanup: a cancelled run's requests, children and external resources keep running.

### Added

- `FlowMsg::Cancel` to a running run cancels its armed `Continue` (a sleep's timer, a deadline, a retry in backoff or
  the next execution), sets the status `cancelled` and clears `armed`, in one commit with the `Cancel`'s completion. It
  runs no flow code, clones no input and keeps the input, the journal, the replies and an open wait exactly as they
  were. It needs no armed message: a run that was never started, is parked at a wait, lost its timer or cannot replay
  is cancelled the same way. It sends nothing, installs no watch, stops nothing and purges nothing.
- Any sender may cancel: an outside caller, another actor, an operator retry of a failed `Cancel` or its redrive by
  the dead letters. There is no sender check beyond the host's own authorization.

### Changed

- A cancelled run ignores `Continue`, `Reply` (before its value is checked) and `Cancel`, writing nothing, as a done run
  does; a `Cancel` to a done run is a no-op. A new job needs a new run key.
- A cancelling run now refuses `Cancel` for good too, as it refuses `Continue` and `Reply`: the cleanup phase needs
  the finalizer of a later release.
- `Cancel` is handled in mailbox order (deliverAt, then seq) and never preempts a turn: a running execution may commit
  its step first, and its effects are not undone. A reply or a timeout decision committed before the `Cancel` stays
  saved, but no flow code consumes it. Sending `Cancel` only queues it; read the run's status (or reread after a change
  hint) to learn that it was cancelled. A paused run takes it after resume; a purge or a delete discards a queued one.
- A successful `Cancel` is an ordinary done turn: the cancelled armed message has no turn and tells no watcher, and no
  synthetic `Failed` or `Stopped` is sent. A rejected commit saves nothing and is not retried by the sender's retry
  policy; a commit whose answer was lost may have applied, so reread the status or resend `Cancel`, which is
  idempotent.
- The descriptions of `FlowMsg::Cancel` and `FlowStatus` in every flow contract describe the new behaviour; the schema
  and the contract hashes are unchanged.

## 0.18.3 — 2026-11-14

A flow can wait for several replies with one durable wait: `Step::wait_all` and `Step::wait_all_timeout` join every
reply, `Step::wait_first` and `Step::wait_first_timeout` take the first (ADR 0044). The flow wire and every contract hash
stay those of 0.18.0; no store schema and no guarantee changes. A node of 0.18.2 refuses for good the reply that would
close a join of several requests or of mode first, so upgrade every host and worker of a flow type before its code
joins. A join of one request is the single wait that 0.18.1 and 0.18.2 already run.

### Added

- `Step::wait_all(id, &requests)` returns `Vec<T>` and `Step::wait_all_timeout(id, &requests, timeout)` returns
  `Vec<Waited<T>>`, one slot per request in the order of `requests`: `Received` with the reply, or `TimedOut` for a
  request whose reply had not come when the join timed out, so a timeout keeps the replies that came.
  `Step::wait_first(id, &requests)` returns `(usize, T)`, the winning request's position in `requests` and its reply,
  and `Step::wait_first_timeout` returns `Waited<(usize, T)>`. Each borrows a slice (a `Vec` or an array) of 1 through
  100 distinct earlier requests that no other wait names, typed by one reply `T`; an enum gives replies of several
  shapes one type. One wait entry holds a join; deadlines, early replies, zero timeouts, the first latch and decode
  failures follow the single timed wait.
- The winner of a first join is the listed request whose reply the run accepted first, in the order its replies are
  kept, also when they came before the join; not the first in `requests`.
- Example 17, `flow_approval`, adds a sign-off that joins the approvals of several teams for a week, and a quote that
  buys from the first supplier to offer.

### Changed

- A reply to a request of the open wait keeps its value and closes the wait only once it has enough replies (all of
  them, or any one for a first join), cancelling the deadline's `Continue` and arming one due now with its correlation
  id; a partial reply leaves the armed continuation and the deadline as they were. Closing a join closes all its
  requests: late replies to losers or missing positions are ignored before their values are checked, their targets
  keep running, and no later wait can name them.
- `Step::wait` and `Step::wait_all` of one request save the same entry and replay each other's, as do `wait_timeout` and
  `wait_all_timeout`. A saved wait whose list, order, mode or deadline presence differs from the call fails for good
  with a diagnostic that names both lists; a saved join no longer names a later release.
- A decode failure of a saved reply names its position and request.
- The description of the wait entry in a flow's contract describes joins; the hashes are those of 0.18.0.

## 0.18.2 — 2026-11-14

A flow's reply wait can give up at a deadline: `Step::wait_timeout` returns `Waited::Received(T)` or
`Waited::TimedOut` as data, decided once in a commit of its own before later code runs (ADR 0044). The flow wire and
every contract hash stay those of 0.18.0; no store schema and no guarantee changes. A node of 0.18.1 refuses for good the
reply to a wait with a deadline, so upgrade every host and worker of a flow type before its code waits with one.

### Added

- `Step::wait_timeout(id, &request, timeout)` returns `Waited<T>` (`#[must_use]`, `Received(T)` or `TimedOut`). The
  deadline is the store time of the claim of the turn that creates the wait plus `timeout`, rounded up to microseconds,
  saved in the wait entry's `until`; replay uses it and ignores `timeout`. A timeout longer than `MAX_DELAY`, or a
  deadline after `MAX_INSTANT`, fails the turn for good, also when the reply is already kept. A new wait without its
  reply commits open with a `Continue` due at the deadline, a timer for observers; a zero timeout commits open first
  too. A new wait whose reply came early commits received and arms a `Continue` due now.
- At or after the deadline, a `Continue` that executes the flow with no reply kept commits the wait timed out
  (`received` empty) and a `Continue` due now with its own correlation id, running no code after the wait; the next
  execution returns `TimedOut`. Before the deadline, an execution (a poke, a retry, a deliver-now) re-arms at the same
  deadline and saves nothing. A kept reply decides received, also past the deadline.
- Example 17, `flow_approval`, waits a week for the decision and tells the requester when the purchase expired.

### Changed

- A reply closes an open wait with a deadline as it closes one without: it never reads the clock, cancels the
  deadline's `Continue` and arms one due now with its own correlation id. Which of a reply and the deadline's
  `Continue` wins is the run's mailbox order; a committed outcome never changes, and a later reply is ignored before its
  value is checked. A timeout stops nothing of the target's work. A cancelled, purged or failed deadline `Continue`
  leaves the wait open, with no polling, until a reply or a public `Continue`.
- `Step::wait` on a saved wait with a deadline, and `Step::wait_timeout` on one without, fail for good with a
  diagnostic that names the other helper. Only a saved join, and a reply that would close one, still name a later
  release.
- The description of the wait entry in a flow's contract describes the new behaviour; the hashes are those of 0.18.0.

## 0.18.1 — 2026-11-14

A flow can ask an actor for work and wait for its reply, durably: `Step::request` sends a message that carries a typed
reply route, and `Step::wait` parks the run with no handler open and no polling until the target replies (ADR 0044).
The flow wire and every contract hash stay those of 0.18.0; no store schema and no guarantee changes. A node of 0.18.0
refuses `Reply` for good and cannot replay a request, so upgrade every host and worker of a flow type before its code
requests.

### Added

- `Step::request(id, to, build)` returns a `Request<T>`. `build` gets a `ReplyTo<T>`, `{run, request}`: the run's
  address and the request's fresh message id, which the target's message carries (a field `reply: ReplyTo<JobResult>`
  types the request). The create, the message, the request entry (`reply: true`) and the next `Continue` commit
  together, as for `Step::send`; replay returns the saved request without converting, building or serializing anything.
  A caught panic in the builder fails the turn for good.
- `Step::wait(id, &request)` returns the reply decoded as `T`, only on replay of a committed outcome. A new wait whose
  reply came early saves itself closed and arms the next `Continue`; one whose reply has not come saves itself open and
  parks the run: `armed` is `null` and no `Continue` is sent. A public `Continue` replays a parked run and parks it
  again without a new entry. A reply that does not decode as `T` fails the turn for good and keeps the outcome: fix the
  code and send a public `Continue`. The wait has no timeout; joins and deadlines come later.
- `ReplyTo::send(turn, &value)` replies from the target's turn: it encodes and checks the value at once (no NUL, at most
  61 levels deep, else `TurnError::Invalid`), and its future borrows only the turn. The reply is due now, carries the
  turn's correlation id and has no retry policy. A reply to a deleted run rejects the target's whole commit.
  `ReplyTo<T>` has one schema and wire form for every `T`; adding it to a target's message changes that target's
  contract like any field.

### Changed

- A running run handles `FlowMsg::Reply` without running flow code. It ignores, writing nothing, a reply that names no
  request of the run, comes from another address than the request's target (an HTTP send has none; a delegate, a child
  and a dead-letter redrive have their own), repeats a kept reply or names a closed wait; only then is the value
  checked, and a NUL or more than 61 levels fails it for good. The first eligible value is kept. When the open wait
  waits for it, the same commit closes the wait, cancels any armed continuation and arms a `Continue` with the flow's
  policy and the reply's correlation id; an early reply leaves `armed` alone. A done or cancelled run ignores `Reply`;
  a cancelling run still refuses it.
- A saved request or single wait replays; a saved join or wait with a deadline, and a reply that would close one, fail
  for good with a diagnostic that names a later release. A failed send or request now names its kind in its error
  (`request \`go\` to … could not be checked`). The diagnostics of `Cancel` and of a cancelling run are reworded.
- The descriptions of `FlowMsg::Reply`, the send entry's `reply` and the wait entry in a flow's contract describe the new
  behaviour. Descriptions are not part of a contract's hash: the hashes are those of 0.18.0.

## 0.18.0 — 2026-11-14

A breaking release for systems with flows: a flow can send actor work atomically with its progress (`Step::send`), and
the flow wire gains the shape that later releases fill with requests, reply waits, timeouts, joins and cancellation
(ADR 0044). Every hosted flow's contract changes, so a node of 0.18.0 that hosts a flow type refuses to start against
its registration of 0.17; no schema of the store, no other contract and no guarantee changes. Keep the flow types
running on pinned 0.17 binaries and host the new code under new type names, or drain and archive the runs and reset
the registrations before upgrading ([Upgrading](docs/operations.md#upgrading)).

### Changed

- **Breaking.** The contract of every flow type changes. `FlowMsg` gains `Reply { request, value }` (`value` is
  required and may be `null`) and `Cancel`; `FlowStatus` gains `Cancelling` and `Cancelled`; `JournalEntry` gains
  `Send { id, to, message, create, reply }` and `Wait { id, requests, mode, until, outcome, received }`; `FlowState`
  gains `replies` (at most 2000 `FlowReply { request, value }`) and `cleanup` (a second journal of at most 1000
  entries, with ids of its own). The new types `CreateKind`, `WaitMode`, `WaitOutcome` and `FlowReply` are re-exported
  by `cicada-actors`. An exhaustive `match` on `FlowMsg`, `FlowStatus` or `JournalEntry`, and a `FlowState { .. }`
  literal without `..FlowState::new(input)`, no longer compiles.
- A state saved by 0.17 decodes: the new fields default to empty, and the depth of a saved step result is not checked
  again. That does not make the registrations compatible; a decoded state is not a migration.
- Decoding a `FlowState` checks the rules of ADR 0044 across fields: send message ids unique across both journals,
  waits that name earlier requests of their journal (cleanup may name a main request that no closed main wait names),
  at most one wait per request in a journal, an open wait only last, a saved selection that matches its outcome and
  mode and names kept replies in request order, replies that name requests once, deadlines at most `MAX_INSTANT`, and a
  status that agrees with `armed`, `cleanup` and any open wait. The schema describes the fields, lengths, patterns and
  names; these checks are not part of it.
- This release refuses `Reply` and `Cancel` for good, and a `Continue` to a cancelling run, with a diagnostic that names
  the later release they need; a `Continue` to a done or cancelled run does nothing; replay stops for good at a saved
  request or wait. Equal contract hashes in the later releases do not mean equal behaviour: upgrade every host and
  worker of a flow type before using a feature of a later release.

### Added

- `Step::send(id, to, message)`: a new send creates its target first if asked (`Target::create`, independent, or
  `Target::child`, a child of the run) and enqueues the message, and the runner commits the create, the message, the
  send entry and the next `Continue` together. The entry saves the destination, the generated message id and the
  create kind, never the message or the state. The message is due now, carries the executing `Continue`'s correlation
  id and has no retry policy: a send promises enqueueing once per committed entry, not the target's success. Replay
  compares only the id and the kind and never converts or serializes the arguments; the code that builds them still
  runs on every replay. A send to the run itself, a target or message that does not convert or encode, a type that is
  not registered or does not accept the message, and an initial state that is `null`, holds a NUL or is too deep (also
  for a target that exists) fail the turn for good; a failed read of the type registry is an ordinary failure of the
  cause `node`; a missing target or a child create of an actor owned elsewhere rejects the commit, and a public
  `Continue` recovers the run. A caught panic while the arguments are converted fails the turn for good.
- `Target<'a, R, S>`: a borrowed ref with an optional borrowed initial state; `&ref` converts into
  `Target::existing(&ref)`.
- Tests: the typed state checks, the latch and conversion order of a send, atomic create and send on virtual time,
  replay without another message, core create rules, rejected commits and their recovery, a registry outage on SQLite,
  observer headers, the refused messages and phases, progress saved by 0.17, the event's runs with the new defaults
  while its contract keeps its hash, and the onboarding example under a new type name next to a registration of 0.17.

## 0.17.4 — 2026-11-14

A fix of closing a SQL store; no public API, contract, wire format or schema changes.

### Fixed

- `Store::close` on PostgreSQL and SQLite finishes its cleanup even when an earlier call was cancelled, for instance by
  a host's shutdown timeout. Before, the first call set the store closed before it awaited anything, and every later
  call returned at once: a cancelled first call left the main pool's connections (PostgreSQL) or the writer and idle
  readers (SQLite) open until the store was dropped. Now the first poll of every call closes the store to new work,
  ends its change streams and stops new connections; then the calls take turns: a later call resumes the unfinished
  cleanup, and concurrent calls all return once it is done. A close future that is never polled has no effect.
  Cleanup has no time bound: PostgreSQL waits for connections that operations still borrow, SQLite for its writer but
  not for reads in flight. Nothing runs in the background after a cancelled call; the next call or the final drop of
  the store finishes it.
- A SQLite read that ends while its store is closing can no longer put its connection back into the drained pool.

## 0.17.3 — 2026-11-14

A fix of the command line's usage errors; no public API, contract, wire format or schema changes.

### Fixed

- A usage error of an `App` binary no longer prints the password of an address whose password holds a raw `'` (legal in
  a URL, and the character clap quotes with): `postgres://u:S3'CRET@h/db` is printed as `postgres://u:***@h/db`, also in
  the errors of the settings, such as `CICADA_LISTEN`. Adjacent addresses in one value are each hidden:
  `--types 'a://h,,b://u:secret@k'` printed the second password. Hiding stays limited to
  `scheme://user:password@host` in usage errors; a password with a raw `/`, `?`, `#`, `"`, `` ` `` or a space is not
  covered.

## 0.17.2 — 2026-11-14

A skill for coding agents in projects that use Cicada (ADR 0041); no public API, contract, wire format or schema
changes.

### Added

- `skills/cicada`: a skill for coding agents that work in projects using Cicada. Its `SKILL.md` routes each task to the
  guides, examples and code of the Cicada source the project resolves, instead of repeating them, so a project on an
  older release reads that release's docs. Its helper `scripts/cicada_source.py` (Python 3.11 or newer) finds that
  source with locked, offline `cargo metadata` for one target and feature selection, and reports each source group
  with its version, full Cargo source, commit, local changes and docs as JSON. A checkout that git cannot read is
  reported as unknown, never as a match. It never fetches, installs, writes a lockfile or changes the project.
- Each GitHub Release from this one on attaches `cicada-skill-vX.Y.Z.tar.gz`: the skill, its helper and `SOURCE.json`
  (repository, tag, commit, version), packaged from the tag's commit by `scripts/package-skill.py`. A skill that cannot
  be packaged, or whose version differs from the tag, stops the release; `scripts/release-check.sh` checks the version
  too.

## 0.17.1 — 2026-11-14

A fix for PostgreSQL stores; no public API, contract, wire format or schema changes.

### Fixed

- Closing a PostgreSQL store no longer leaves the listener's probe running. The probe ran in a task of its own, which
  the stop of the listener did not end, so it kept the listener's pool of two connections open after the store was
  closed. It now runs inside the listener's task and ends with it, and both connections close.

## 0.17.0 — 2026-11-14

A breaking release for systems with dead letters: a letter keeps every payload that was accepted when it was sent, so
a failure at the depth limit can be watched, kept and redriven (ADR 0043). The contract of `cicada.dead-letters` and
the type `DeadLetter` change; a `Failed`, every other contract and the store stay as they were, and no schema of the
store changes. A system without dead letters can deploy it rolling: nodes of 0.16 and 0.17.0 share a database. A system
with dead letters moves its letters in a maintenance window ([Upgrading](docs/operations.md#upgrading)).

### Changed

- **Breaking.** A dead letter holds its payload in `payload`, as before, when it is at most 61 levels deep, and
  otherwise as compact JSON text in the new field `payloadJson`; a letter holds exactly one of the two, which the
  decoder and the schema of the state both check (`422` over HTTP for both, neither or `payloadJson: null`). No value
  inside a payload is read as a marker. Before, the turn that would record a payload of 62 to 64 levels failed and no
  letter was kept. The contract hash of `cicada.dead-letters` changes, so a node of 0.17.0 that hosts it refuses to
  start against the registration of 0.16.
- **Breaking.** `DeadLetter::payload` is a `LetterPayload` (`Value` or `Json`, re-exported by `cicada-actors`), whose
  `value()` returns the payload and parses the text only when it is called; reading a state never parses it, and a
  letter is written back in the form it was read in.
- A `Redrive` of a letter whose text cannot be parsed records `redriveError` `The kept payload cannot be read: …`, and
  one whose payload the send refuses (deeper than 64 levels, a NUL character) records the error of the send; nothing
  is sent and the letter keeps its text, and a `Discard` or another letter's `Redrive` works as before.
- serde_json parses JSON text with exact rounding (`float_roundtrip`, on the shared dependency of the workspace, so a
  crate that depends on Cicada gets it): a finite float read back from SQLite, a view or the text of a letter has the
  bits it was written with. PostgreSQL still stores a negative zero as a zero.
- The doc comment of `JournalEntry::Sleep` describes the sleep of `Step::sleep`, which shipped in 0.16.6; the contract
  texts of flows change in that description only, and their hashes stay.

### Added

- Example 16, [examples/src/dead_letters_upgrade.rs](examples/src/dead_letters_upgrade.rs): the tested steps of the
  upgrade of the dead letters (`export`, `convert`, `reset`, `restore`), which keep every letter, pause flag and watch
  of independent, childless dead-letters actors that no actor watches, and refuse any other shape, a busy actor and an
  export that is incomplete or stale before anything is deleted. The operations guide's Upgrading says what they
  cannot do: quiescing, the backup, stopping every node of 0.16, and the new identity of the restored actors.
- Tests at the depth limit: a `Failed` of a 64-level payload is written and claimed whole on every backend and read by
  a typed watcher on a node and in a worker over HTTP; numbers in states, payloads and notices are read back with their
  bits on every backend; a refused batch publication is kept by the dead letters and its redrive fails again for the
  state of its run; the event's lifecycle test runs an `Unsubscribe` while its runs exist.

## 0.16.17 — 2026-11-14

Nothing breaks: an `App`'s `worker` runs its actors through a server's HTTP API, without the database, the fifth part
of remote execution. No schema, contract, route, wire format or store behaviour changes; a worker on the store runs as
before.

### Added

- `worker --server URL`, `--context NAME` and `--token TOKEN`, before or after `worker` (ADR 0042, which amends ADR
  0027 for `worker`): HTTP mode, with `cicada::runtime::Worker` over `ActorClient`. `--store` and `--server` or
  `--context` together are a usage error; without them a non-blank `CICADA_STORE` selects the store, else a non-blank
  `CICADA_SERVER` or `CICADA_CONTEXT` HTTP; a token or the config file's current context alone selects nothing. The
  server, token and headers resolve as for the operator commands (a context's token and headers also under a
  `--server`); a worker on the store reads neither the config file nor `CICADA_TOKEN`.
- Over HTTP, `--node-id`, `--concurrency`, `--poll-interval`, `--max-turn` and `--shutdown-grace-period` (flag,
  variable, host's `NodeConfig`, default) and the host's `shutdown_cancel_timeout` set the `WorkerConfig`;
  `--store-schema`, `--store-pool-size`, `--retention`, `--lease-ttl`, `--heartbeat-interval` and `--max-lost-claims`
  are refused (exit 2), and their variables, the host's values of them and `CICADA_LISTEN` are not read. No store is
  opened and no store feature is needed. An `App` with `on_start` refuses HTTP mode (exit 2) before any request.
- The worker's client follows no redirect, retries nothing of its own and gives each request 30 s, with
  `ActorClient`'s 10 s turn timeout for the lease calls. Before the start it lists the types while the server answers
  "unavailable", a second apart and no try after a minute; a refusal fails at once (exit 1). A failed registration
  fails the command (exit 1). A stop calls `Worker::stop` (exit 0); a worker that stops by itself exits 1 with its
  error, also when a stop came at the same time.
- The app guide's [A worker over HTTP](docs/guide/app.md#a-worker-over-http), with a compiled example, and the new
  flags and help of `worker` in `docs/reference/cli.md`.

### Changed

- `worker` without a mode says how to give a store or a server, and `worker -o` says that a worker prints no results;
  `worker --token` on the store is refused as needing HTTP mode.

## 0.16.16 — 2026-11-14

Nothing breaks: a worker runs unchanged Rust actors through any server of the HTTP API, the fourth part of remote
execution. No schema, contract, store behaviour, command line or existing route changes; four routes are added.

### Added

- `POST /leases`, `/leases/renew`, `/leases/release` and `/leases/complete` on the full router (`actor_api`; not
  `actor_api_read`), operationIds `leaseTurns`, `renewTurn`, `releaseTurn` and `completeTurn` (ADR 0040). They serve
  the core's lease, renewal, release and completion on any system of the store, started or not, hosting the types or
  not, with no session: a worker can lease on one server and renew and complete on another. Core alternatives are
  `200` results tagged by `result` (`leased`, `contractMismatch`, `renewed`, `leaseLost`, `completed`, `rejected`); a
  release is `204`. Their bodies refuse unknown fields at every level and a key repeated at any depth (also spelled
  with other escapes), an empty or non-object body and anything after it, with `400` before any operation; the body
  limit and serde_json's nesting limit apply. A completion calls the core once, with no retry, fallback or release.
  They are trusted writes: the host's authentication must cover `/leases` and its children.
- `cicada::api::leases`, re-exported in `cicada::api`: `LeaseTurnsRequest`, `LeaseTurnsResponse`, `TurnLeaseView`,
  `TurnMessageView`, `RenewTurnRequest`, `RenewTurnResponse`, `ReleaseTurnRequest`, `CompleteTurnRequest`,
  `CompleteTurnResponse`, `TurnReportView`, `StateReportView`, `TurnEffectsView`, `TurnSendView`, `ActorCreateView`,
  `WatchChangeView`, `ControlView`, `ControlEffectView`, `TurnFailureView` and `TurnOutcomeView`, with conversions to
  and from the core types. A `TurnMessageView` has every field of `MessageView`, each required (nullable ones as
  `null`), its policies decoded strictly; `epoch`, `version` and `seq` are exact `i64` integers, durations keep
  nanoseconds. `MessageView` is unchanged.
- `ActorClient` implements `TurnSource`, so `Worker::builder(ActorClient::with_client(url, http))` runs actors without
  database access. One request per call (`send_attempts` applies to sends only), each lease, renewal, release and
  completion bounded by `ActorClient::turn_timeout` (default 10 s, positive) through the response body. Errors follow
  the HTTP status alone: `400`, `413` and `422` are `Invalid`; `408`, `429`, `502`, `503`, `504`, transport errors and
  timeouts `Unavailable`; any other status, a malformed body or an unknown variant `Fatal`. A registration conflict
  reads the existing type once and computes the requested hash from the schemas. The `reqwest::Client` keeps its own
  redirect and retry policy: give a worker one without redirects.
- Example 15, `examples/src/remote_worker.rs`: the cleanup singleton in a worker that reaches only the HTTP API, with a
  bearer token and redirects off.

### Changed

- The test `work_that_outlives_the_cancel_timeout_is_no_longer_renewed_and_its_lease_expires` stops the worker only
  once its handler signals that it entered its blocking section.

## 0.16.15 — 2026-11-13

Nothing breaks: a worker runs unchanged Rust actors through a turn source instead of a store, the third part of remote
execution. No schema, contract, route, command line or store behaviour changes; a node runs its turns as before.

### Added

- `Worker`, `WorkerBuilder` (`host`, `observe`, `config`, `build`), `WorkerConfig` (`worker_id`, `poll_interval`,
  `default_max_turn`, `concurrency`, `shutdown_grace_period`, `shutdown_cancel_timeout`) and `WorkerError` (`Start`,
  `Source`, `ContractMismatch`) (ADR 0039). A worker registers its contracts through its source (a conflict is the
  node's `StartProblem::ContractConflict`), leases turns by polling or `run_due_turns`, and runs them with the node's
  executor: the same handlers, buffering, raw sends and creates, timeouts, decode and panic handling, watches,
  controls and observers. Each lease follows the TTL its server granted, with a heartbeat of a third of it; a late
  answer cannot revive an expired lease. Completion keeps one report and one budget of a TTL: unavailable answers are
  retried with that report, `Rejected` replaces it for good with its failure, and only `Completed` ends observers.
  Polling and manual batches share `concurrency`, lease requests in flight included. A lease request answered with a
  contract mismatch, `Invalid` or `Fatal` stops the worker, and `Worker::stopped` returns that error. A worker has no
  node row, sweep or claim limit, and never stops or closes its source.
- `TurnSource`, the async trait a worker leases from (`register_contract`, `list_types`, `lease_turns`, `renew_turn`,
  `release_turn`, `complete_turn`), implemented by `ActorSystem` and `Arc<T>`, and `SourceError` (`Unavailable`,
  `Invalid`, `Fatal`), with the classification of `OpError` the reference source uses.
- The core guide's Run actors in a worker, with a tested example; workers in the architecture and in operations.

## 0.16.14 — 2026-11-13

Nothing breaks: an executor outside the system can complete the turns it leased with the report of its handler, the
second part of remote execution. No schema, contract, route, command line or store behaviour changes.

### Added

- `ActorSystem::complete_turn(&ActorAddress, i64, &MessageId, TurnReport)`, with `TurnReport` (`Returned` with a
  `StateReport` and `TurnEffects`, `Failed(TurnFailure)`, `Cancelled`), `TurnSend`, `CompleteTurnResult` (`Completed`
  with a `TurnCompletion`, `Rejected(TurnFailure)`, `LeaseLost`) (ADR 0038). Any system of the store, started or not,
  commits the report in one fenced commit attempt. It derives the outcome, a failure's retry from the message's own
  policy and failure count, each send's sender and cause, and the event id; a report cannot pick them. A retryable
  failure is allowed only for the causes `handler`, `node` and `timeout`, and `claim-limit` never. A send or create
  beyond the storage limits, or a new state that cannot be stored, fails the message at once without effects. A commit
  the store rejects or refuses commits nothing and returns `Rejected` with the failure to submit next; the executor
  then reports only that failure. A repeated report after its commit applied is `LeaseLost`; no receipt is kept.
  The core has no worker yet.
- `store::ActorCreate`, `store::Control`, `store::ControlEffect` and `watch::WatchChange` are public, as the records of
  `TurnEffects`.
- The core guide's Complete a leased turn, with a tested example, and the completion of leased turns in the
  architecture.

### Changed

- The docs of the claim limit for leased turns now say what the code does: the watchers' notifications are stored
  atomically with the commit that fails the message, also when its acknowledgement is lost; only the observers wait
  until it is confirmed.

## 0.16.13 — 2026-11-13

Nothing breaks: an executor outside the system can lease, renew and release turns through the core, the first part of
remote execution. No schema, contract, route or command line changes. One store behaviour changes: a renewal never
shortens a held lease.

### Added

- `ActorSystem::lease_turns(&NodeId, &BTreeMap<TypeName, String>, usize)`, `renew_turn(&ActorAddress, i64)` and
  `release_turn(&ActorAddress, i64)`, with `TurnLease`, `LeaseTurnsResult`, `RenewTurnResult` and the bounds
  `runtime::MAX_LEASE_BATCH` and `MAX_LEASE_TYPES` (100 each) (ADR 0037). A system without handlers, started or not,
  leases due turns of the selected types for a worker id, once one registry read found every requested contract hash
  (`ContractMismatch` names the first type that is absent or differs, and nothing is claimed). The serving system's
  `lease_ttl` and claim limit apply: a message at the limit fails by one fenced commit and is never handed out. A
  lease carries the claim and the TTL; the caller's deadline is its request time plus the TTL, less
  `min(ttl / 10, 1 s)`. Claims not handed out (a dropped call, a late answer, a stopping system) are released, best
  effort. Renew and release work by actor and epoch on any system of the store, in every life-cycle state; a renewal
  reports a cancel request. No completion yet: a leased turn ends by release, expiry or the removal of its actor.
- The core guide's Lease turns for an executor outside the node, and Leases outside the node in the architecture.

### Changed

- A renewal never shortens a held lease, on every store: it keeps the later of the lease's expiry and store now plus
  the TTL, atomically, and returns that expiry (S5). Nodes that share a TTL see no difference; a peer with a shorter TTL
  can no longer end a lease early.

## 0.16.12 — 2026-11-13

Nothing breaks: a server can register the contract of a type that runs elsewhere, so that it accepts the type's creates
and sends before a worker runs it. The route and the operation are new; no schema, actor contract, command line or
existing route changes. Upgrade a server before a worker registers through it.

### Added

- `ActorSystem::register_contract(&Contract)` and `RegisterContractResult` (ADR 0036): registers a contract without
  hosting the type, on any system, started or not, with no hosted type, node row or change hint. The hash is computed
  from the name and the schemas, also for a contract that carries another one (`Contract::of`). The store's atomic
  registration decides: an equal hash is `Registered` with the stored contract and its first descriptions, another is
  `Conflict` with the registration that stays and the requested hash, and nothing is replaced, even without actors.
  Before the store, each schema must be an object or a boolean, free of NUL characters and at most
  `limits::MAX_SCHEMA_DEPTH` (120) arrays and objects deep, and each message name must obey the send limits; otherwise
  `InvalidContract`. Schemas are not compiled. A node that hosts the type later starts only with an equal hash.
- `PUT /types/{name}` on `actor_api` (operationId `registerType`) with the strict body `cicada::api::RegisterTypeRequest`
  `{state, messages}`: `200` with the stored `TypeView` for a new and an equal registration, `400` for a malformed body,
  any other field (`name`, `type`, `hash`, …) or a path that is no type name, `409 contract-conflict` naming the type
  and both hashes, `422 invalid-contract`. `actor_api_read` answers `405`. The problem types `CONTRACT_CONFLICT` and
  `INVALID_CONTRACT` are new.
- `ActorClient::register_type(&Contract)`: sends only the schemas and returns the stored `TypeView`; a refusal is
  `ClientError::Api` with the problem.
- The core guide's Register a contract without hosting and the HTTP guide's Register a type over HTTP.

### Fixed

- A node's cache of parsed contracts is reused only when the stored JSON matches too, not only the hash: after another
  system removed a type and registered it again with other descriptions, `GET /types` and the resolver of every node
  show the new descriptions.

## 0.16.11 — 2026-11-13

Nothing breaks: a browser page can follow the change hints over HTTP. The route is new; no schema, actor contract,
command line or existing route changes. Upgrade a server before a page uses the route.

### Added

- `GET /changes?prefix=` on both `actor_api` and `actor_api_read` (operationId `streamActorChanges`, ADR 0035): the
  hints of `ActorSystem::changes` as Server-Sent Events, `event: changed` with `data: {"address":"<canonical>"}` and
  `event: resync` with `data: {}`, the first a `resync` once the store's source is live. The prefix is the literal
  one of the listings and other query keys are ignored. No `id` or `retry` field and no replay: `Last-Event-ID` is
  ignored and each request starts with a `resync`. The `200` comes at once with `Cache-Control: no-cache` and
  `X-Accel-Buffering: no`, and a `: keep-alive` comment follows each 15 seconds without output; comments do not mean
  the stream is live. Each response holds one subscription and no queue of its own, so a slow reader gets the core's
  `resync`. The OpenAPI document describes the body as a `text/event-stream` string with examples.
- The HTTP guide's Follow changes over HTTP: the frames, lifetimes, authentication, CORS and proxies, and a browser
  consumer with one worker, at most 1000 pending addresses, resyncs that drop an older incomplete listing, and
  retries that keep a failed address. `cicada-client` gets no method for the stream.

### Changed

- A closed store ends an open change stream without a final event, and `GET /changes` after the close answers an empty
  `200`: the streaming exception to the `503` of a closed store. Stopping a node leaves the streams open, so with a
  page open `serve` may use its whole 5-second drain before it cuts the stream and closes the store.
- The axum service of the HTTP guide closes its store inside the shutdown future, after the node stopped, so open
  change streams end and axum's graceful shutdown can finish.

## 0.16.10 — 2026-11-13

Nothing breaks: a host can follow hints that actors changed instead of polling them. No schema, HTTP route, actor
contract or command line change. On PostgreSQL a commit that changes an actor sends one more notification; nodes of
0.16.9 ignore it, but send no hints: upgrade every node that writes, then let followers subscribe again.

### Added

- `ActorSystem::changes(prefix)`, an `ActorChanges` stream (`Send`, `Unpin`, a cancel-safe `next`) of
  `ActorChange::Changed { address }` and `ActorChange::Resync` (ADR 0034). The first item is `Resync` once the store's
  source is live; read the prefix with `list_actors_with_state` after a `Resync` and each changed address with
  `get_actor`. A committed create, new state (also an equal one), pause, resume, restart and removal with every
  descendant is reported once per transaction, after its commit; more than 100 addresses or 7,998 encoded bytes are one
  `Resync`. Hints carry no state or version, have no history, replay or order across transactions, and a reader more
  than 1024 items behind gets a `Resync`. The prefix is literal, as for `list_actors`. It needs neither `start` nor
  hosted types; `stop` leaves it, `Store::close` ends it.
- Memory and SQLite report the writes through the same `Store` and its clones; a separately opened SQLite file is not
  covered. A SQLite `COMMIT` that fails or whose caller is cancelled settles under its writer before a `Resync`.
- PostgreSQL reports the writes of every node of 0.16.10 or later on the schema: one `@` payload per transaction on
  the store's channel, sent with the wake-ups in one `pg_notify` before `COMMIT`. The listener turns the feed live only
  after its `LISTEN`, also after a reconnect, and a lost connection gives a `Resync`.

### Changed

- `Store::close` ends change streams normally (they end without their backlog, and a new one ends at once), the one
  exception to "every operation fails as unavailable".
- PostgreSQL: a commit that only writes a state now sends a notification too, and a `COMMIT` that fails with SQLSTATE
  `54000` (a full notification queue) is `StoreError::Unavailable`, not `Invalid`; other errors map as before. The
  wake-ups of the dispatch loop also wake once when the listener first has its `LISTEN`.

## 0.16.9 — 2026-11-13

Nothing breaks: a host can list actors with their state in one read per page. The route and the methods are new;
`list_actors`, `GET /actors` and its OpenAPI operation stay as they were. No schema, actor contract or command line
change. Upgrade a server before its clients call the new route.

### Added

- `ActorSystem::list_actors_with_state(prefix, cursor, limit)`, a page of the `ActorSnapshot` that `get_actor` returns
  (address, state, version, `created_at`, `paused`, `parent`), with the prefix, order, cursors and positive-limit rule
  of `list_actors` (ADR 0033). Each page is one read of the store: one `SELECT` per read attempt on SQLite and
  PostgreSQL, one lock in memory. Its rows share one committed snapshot and each state comes with its own version;
  there is no snapshot across pages. The stored JSON is returned without an actor or a schema, also for a state stored
  before the depth limit; one row that does not decode fails the page.
- `GET /actors/states?prefix=&cursor=&limit=` on both `actor_api` and `actor_api_read` (operationId `listActorStates`,
  response `ActorViewPage`): the view of `GET /actors/{type}/{key}` for each actor, in the pages of `GET /actors`,
  limit 1 to 1000, 100 by default. An actor of a type named `states` stays at `GET /actors/states/{key}`. A page holds
  a state three levels deeper, 67 for a state at the limit.
- `ActorClient::list_actors_with_state(prefix, cursor, limit)` for that route. Against an older server it returns that
  server's error and does not fall back to reading each actor.

## 0.16.8 — 2026-11-13

Nothing breaks in the API: an observer can pick timers or the other sends for a header, and `cicada-otel` starts a new
trace for each timer, linked to the turn that set it. No schema, HTTP field or actor contract changes. Traces change on
purpose: an actor that re-arms itself no longer builds one trace that never ends.

### Added

- `cicada::observer::Sends` (`All`, `Timers`, `Others`) and `TurnStart::add_header_for(sends, name, value)`, which
  adds an observer's header to the timers of a turn, its other sends, or all of them; `add_send_header` is `All`, as
  before. A timer is a send to the turn's own actor with `At` or `After`, whatever the time: a zero delay and a past
  instant count, on every send method and for every actor type. Precedence, the limits of a message and the isolation
  of a panicking observer hold for each kind on its own (ADR 0032).

### Changed

- `ActorTelemetry` puts its propagator's headers on the other sends as before and, prefixed with `cicada-link-`
  (`cicada-link-traceparent`), on timers. A turn takes its parent from the plain headers and at most one link from the
  prefixed ones, so a timer's turn is a root span with a link to the turn that set it, and the messages it sends are in
  its trace. A timer that sets its own `traceparent` keeps it as its parent and gets the link too; the all-zero
  workaround now keeps a link. A flow's sleep, a scheduler's `Tick` and the deadline `Flush` of a debouncer or a batcher
  are timers; a flow's immediate continuations and a rate limiter's delayed forward stay in their sender's trace.
- Each attempt of a timer (a policy retry, a re-run after a lost claim, an operator retry) is a root linked to the same
  sender, and the configured sampler decides about these roots on its own: with a parent-based sampler, timers no
  longer follow the sender's sampling decision. Links are best effort: a missing or malformed field, another propagator
  or a full message leaves the link out, and nothing fails. A timer queued before the upgrade keeps its parent until it
  re-arms; an older node ignores the prefixed headers and starts a root without a link.

## 0.16.7 — 2026-11-13

Nothing breaks: a new ready-made actor starts flows from an event. No existing actor, flow contract or schema of the
store changes; a node that hosts the event registers one more type, `cicada.event`.

### Added

- The event (`EventActor`, `cicada.event`, one instance for each business event, built with `Event::named`): its state
  holds up to 100 subscribed flow types, each once and in order, which a host adds with `Subscribe` and removes with
  `Unsubscribe` (an invalid name and a 101st type are refused for good; the registry is read only on publication).
  `Publish { input }` creates one run of each subscribed flow with `FlowState::new(input)` and sends it a `Continue`,
  all in one commit, keyed by the publication's own message id (`orders.receipt/<id>`); `PublishBatch { items }` does
  the same with the input `{"items": [...]}`, so a batcher whose message type is `PublishBatch` feeds it directly.
  `EventMsg::publish` and `EventMsg::publish_batch` build them. A type that is not registered or does not take
  `Continue`, input that a hosting node cannot decode, and the ids `.` and `..` fail the publication for good and start
  no run; a read of the store that fails follows the publication's retry policy. The first `Continue` has the crate's
  shared policy and the publication's correlation id; the run then arms its flow's own policy. The actors guide,
  "Event", has the rules on keys, retries and redrives, repeated ids, validation and depth; the worked example
  `flow_events` starts a receipt and a fulfilment from a placed order and a digest from a batcher. The contract is in
  `docs/reference/contracts/cicada.event.json`, and the sample service hosts the type, with no event at startup.

### Changed

- The actors guide, "Durable sleep": only a zero sleep makes the next turn due at once; a positive duration under a
  microsecond rounds up to one microsecond.

## 0.16.6 — 2026-11-13

Nothing breaks: flows can sleep durably. No state field, message, journal entry or contract changes, so a flow that
starts to sleep keeps its contract hash; no schema of the store changes. Deploy 0.16.6, with the flow code that
sleeps, to every node that hosts a flow before starting runs that use it.

### Added

- `Step::sleep(id, duration)`: a durable sleep inside a flow. A new sleep saves `{"kind": "sleep", "id", "until"}`
  with `until` the claim's store time plus the duration, rounded up to microseconds, and ends the turn; the entry
  commits with the next `Continue`, due at `until`, so no handler stays open while the run waits. A zero duration
  commits too. A replayed sleep returns once store time reaches the saved `until`, whatever duration the code passes
  now; before that it arms a `Continue` due at the same time and saves nothing, so an early `Continue`, a deliver-now,
  a retry or a redrive cannot shorten it. A late wake (pause, outage, a cancelled timer resumed by `Continue`) does not
  sleep again. A new sleep longer than `MAX_DELAY` or ending after `MAX_INSTANT`, a repeated id, a mismatch with the
  saved entry and a 1,001st entry fail the turn for good. The actors guide, "Durable sleep", has the rules; the
  onboarding flow example sleeps a minute before it provisions the workspace, with its type name, input and contract
  unchanged.

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
