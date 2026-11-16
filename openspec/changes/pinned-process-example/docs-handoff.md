# Docs handoff for pinned-process-example

Version: 0.20.1. Refresh the dependency pins of `docs/how-to/install.md` once it is published. Not breaking: an
application example only; library APIs, wire formats, contract hashes, store schema and guarantees are those of 0.20.0.

## What changed

Example 21, `cicada_examples::pinned_process` (ADR 0053): a worked actor and host that start, observe and stop a
local child process on the node the actor is pinned to. It teaches durable intent in the actor versus a live handle
in the host, and what repeat safety a host can give within its own life.

- Actor: `Run { key }` (type `process.runs`), one never-reused key per run. `RunMsg::{Start, Stop, Refresh,
  Observed { host_epoch, revision, status }}`. `RunState { host_epoch, desired, observed, revision, refusal }` with
  `Desired::{Idle, Run, Stop}`, `Observed::{Idle, Starting, Running { pid }, Stopping, Exited { code }, SpawnFailed,
  StoppedBeforeStart, NeedsReconciliation}` and `Refusal::{Stopped, Ended, Capacity, HostClosed}`. `RunActor::new(host)`.
- Host: `ProcessHost::new(node_id, backend)` has a fresh epoch per process start, separate from the stable node id.
  At most `MAX_SLOTS` (32) slots, one per run key, reserved before spawning and kept (with terminal tombstones) for
  the host's life; each starts its child at most once. `start`/`stop` wait at most `IO_TIMEOUT` (5 s); `Starting`
  after the wait means unknown. `Stop` is sticky, also before `Start` (a terminal tombstone that uses a slot).
- Turn checks: a turn whose `Turn::node_id()` is not the host's id, or whose stored `host_epoch` is not the host's,
  does nothing to a process; a run that was not terminal becomes `NeedsReconciliation`, before any refusal (a
  `Start` to an old run that was stopping marks it too), and that is absorbing (a late
  exit report of the original host does not clear it). Observations apply only for the run's epoch, a higher revision,
  and a status that is not final; they never change `desired`. Start after the end is refused (`Ended`).
- Helpers: `create_run(system, host, key)` creates the run pinned to the host's node id and checks the stored pin
  (an existing key is refused); `reconcile(system, node)` sends `Refresh` to every run pinned to the node, paging;
  `pump(host, sink, shutdown)` sends the latest observation of each slot with the stable id `observation_id(epoch,
  run, revision)`, retrying every `RETRY_INTERVAL` (1 s) within `IO_TIMEOUT`, counting failures in
  `ProcessHost::delivery()`; `ProcessHost::shutdown(budget)` closes admission, stops and reaps every child and waits
  for delivery, returning a `CleanupReport` of what it could not confirm. `run_host(store, system, host, body)`
  attaches the pump, runs `body`, and then always shuts down in order (host cleanup within `IO_TIMEOUT`, the system's
  bounded stop, the pump's stop and join, the store's close), also when `body` fails; it returns `body`'s result and
  the report (whose `pump` field names a pump task that failed). `ObservationSink` (implemented by
  `ActorSystem`), `ProcessBackend` and `ChildProcess` are the seams.
- OS adapter: `OsBackend::new(program, args)` (with `env`, `exit_with`) runs only the example executable's inert
  child mode `child_main()`: no shell, stdout/stderr discarded, a one-line stdin control, kill-on-drop as a best-effort
  fallback, and an explicit wait after every kill.
- Demo: `cargo run -p cicada-examples --example pinned_process -- --db runs.db --node host-a [--new-run KEY]
  [--exit-code N]` (`examples/examples/pinned_process.rs`).
- Tests: `examples/tests/pinned_process.rs` (20 cases on virtual time with a fake backend, among them
  `a_spawn_whose_commit_is_lost_is_found_by_the_replayed_turn`, `a_stop_whose_commit_is_lost_still_keeps_the_run_from_starting`,
  `a_new_host_epoch_marks_old_runs_for_reconciliation_and_acts_on_none`,
  `shutdown_reports_an_exit_it_cannot_confirm_and_an_observation_it_cannot_flush`,
  `an_error_while_a_child_runs_still_cleans_up_in_order_and_is_returned`,
  `a_start_on_an_old_epoch_marks_a_stopping_run_for_reconciliation_before_any_refusal`) and
  `examples/tests/pinned_process_os.rs` (`the_os_adapter_exits_repeats_stops_and_reaps_real_children`, Linux and
  macOS only). No guarantee row: the example's at-most-one spawn per slot is an application property, not G4.
- Skills (done in this change): `skills/cicada/SKILL.md` routes the socket/process/device row to
  `examples/src/pinned_process.rs` and `examples/examples/pinned_process.rs`; `skills/actor-design/SKILL.md` adds
  `examples/src/pinned_process.rs` to its outside-effects rows.

## 1. Design guide: the host-held handle form now has a worked example

**Where.** `docs/guide/design.md`, `Work with outside systems`: the paragraph after the table of forms.
**Text.** Replace "The first form works today when only one node hosts the type: any hosting node may execute its
next turn. Do not assume actor placement or a pin API." with substance like: the first form needs the turn to run
where the handle lives; pin the actor to that host's stable node id at creation (`CreateOptions::new().pinned_to`),
and check `turn.node_id()` against the host's id before acting. Example 21 (`examples/src/pinned_process.rs`) shows
it for a child process: the actor keeps desired and observed state, the host keeps the handles in bounded slots that
are never reused, a fresh host epoch per process start tells a host's own runs from older ones, and versioned
observations come back as ordinary messages. Name the recovery cases it handles: an outside action whose commit was
lost (the replayed turn finds the slot), a stop that raced the spawn, stale or reordered events, and a host restart
(old runs become `NeedsReconciliation`; nothing is adopted or respawned).
**Caveats.** Repeat safety holds only while one host process lives. A host that dies abruptly may leave an orphan,
and its successor cannot know whether a spawn happened. A stored PID is diagnostic, never a handle. Unique node ids
are the deployment's duty.

## 2. Design guide: compare the websocket session with the pinned process

**Where.** `docs/guide/design.md`, `A websocket session`: a short closing paragraph or a sentence near its start.
**Text.** The session needs no pin: its socket can be re-created on any host that shares the store, so the host
follows durable state (the second form). A child process exists on one machine only, so example 21 pins its actor
and lets the turn use the host-held handle (the first form). Choose by whether a new host can rebuild the resource
from durable state.

## 3. Core guide: the examples table

**Where.** `docs/guide/core.md`, `Examples`, the table of sources.
**Text.** A row after the websocket session's: `Process run pinned to its node: host-held child, slots, reconciliation`
→ `[pinned_process.rs](../../examples/src/pinned_process.rs)`, with its demo in `examples/examples/pinned_process.rs`.

## 4. Operations: lifecycle, cleanup and restart of host-held resources

**Where.** `docs/operations.md`, `Shutdown and probes` (shutdown order) and `On call` (a table row), or one short
paragraph under `Deploying`.
**Text.** For a host that owns outside resources like example 21's: run exactly one host per node id. On shutdown,
close the host to new work and stop and reap its resources, and deliver their last observations, while the node and
the store still run; then stop the node, then the pump, then close the store. The demo bounds that cleanup to 5 s
and exits with a failure, naming the runs, when it cannot confirm an exit or a delivery. An error after the node
started goes through the same cleanup before it is reported. Deleting a run's actor does
not stop its process: stop the run and see it terminal first. On-call row: symptom `Run shows NeedsReconciliation`;
check whether the old process (from a crashed or restarted host, or a host sharing the node id) still exists outside
Cicada; action: clean it up there, then delete the actor with the ordinary delete, and create a replacement under a
new key if the work is still needed. Nothing replaces or restarts it automatically.
**Caveats.** Deletion proves no cleanup. Kill-on-drop is best effort; an abrupt host death (kill -9, machine loss)
may leave an orphan. The demo's child exits when its stdin closes, but that is the example's inert child, not a
property of arbitrary programs.

## Blocking

None. No doctest, link, guarantee row or skill route needs protected prose: the skill routes name existing files.

## Status

Ran `cargo test -p cicada-examples --doc` (30 passed), `--test docs` (11 passed) and `--test skill` (18 passed) on
this branch; `--test pinned_process` 20 passed, `--test pinned_process_os` 3 passed on macOS. All prose above is
proposed text for the docs session; nothing protected was edited.
