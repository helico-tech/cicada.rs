# Docs handoff for worker-stop-now

Version: 0.26.0. Refresh the dependency pins of `docs/how-to/install.md` once it is published.

## What changed

Stop Now ends a worker's drain early. `cicada::Worker::end_grace()` ends the grace period of a stop that has begun, as
if it had run out. The running turns are cancelled at once, a turn whose work stops before its handler ended tries to
release its lease (a failed release, or work that ignores cancellation, leaves the lease to expire), and the stop
returns within the cancel timeout. A wait for the polling loop also ends, and the loop is aborted. Before a stop begins,
or after it ended, the call does nothing and is not remembered. It is a plain call, not `async`. Tests:
`end_grace_cancels_the_running_turns_at_once_and_the_stop_returns_within_the_cancel_timeout`,
`end_grace_before_the_stop_reaches_its_grace_wait_still_ends_it` and
`end_grace_before_a_stop_is_not_remembered_and_the_later_stop_keeps_its_full_grace_period` in
`crates/cicada/src/tests/runtime/remote_worker.rs`.

`cicada_app::StopNow` is a new `Clone + Debug` handle in `WorkerReady::stop_now`. It holds no reference to the worker.
`StopNow::now()` ends the grace period of the worker over HTTP's stop, including the stop that follows a panicking
`App::on_worker_ready` function. A request made before the stop begins is kept until it does, also between the host's
stop signal and the worker's own stop. A request after the stop does nothing. `now()` does not stop the worker by
itself. A turn whose work stops before its handler ended tries to release its lease, which may instead expire; a turn
whose handler had ended may still complete, or be abandoned while its result is checked. `WorkerReady` loses `PartialEq`
and `Eq`. Tests: `over_http_stop_now_ends_the_drain_early_and_releases_the_running_turn`,
`over_http_stop_now_right_after_the_stop_signal_is_kept`,
`over_http_stop_now_before_the_stop_signal_applies_when_the_stop_begins` and
`a_panicking_worker_ready_function_still_takes_stop_now` in `crates/cicada-app/src/node.rs`.

`cicada_tray::Tray::stop_now()` works while the tray stops a worker that was ready. It calls the kept `StopNow`, and
sets `TrayState::Stopping::until` to the earlier of the current `until` and now plus the cancel timeout. The state stays
`Stopping` until the command returns, within the cancel timeout, then becomes `Offline`. The running turns are
cancelled. A turn that ends without an acknowledged commit, also after a failed release, is listed as abandoned. A turn
whose handler had already ended may still commit. A turn the worker leaves behind, such as one whose release is still in
flight at the cancel timeout, stays listed as running after `Offline` until it ends. `Offline` does not wait for it, as
such a turn may never end. In any other state, or while stopping a worker that was never ready, it does nothing. Like
`connect` and `disconnect`, `stop_now` must not be called while a borrow of `Tray::state()` is held. The menu shows a
Stop Now row under the header in that state. The header's countdown stays. Tests: the `stop_now_…` tests in
`crates/cicada-tray/tests/control.rs` and `stop_now_is_offered_only_while_stopping_a_ready_worker` in
`crates/cicada-tray/src/menu.rs`.

No guarantee row, wire format, schema or command line changes.

## 1. Ending a worker's grace period early

**Where.** `docs/guide/core.md`, `Run actors in a worker`, next to `stop()`.
**Text.** `stop()` drains: running turns get the grace period, and then they are cancelled. To end the drain early,
call `worker.end_grace()` from anywhere while the stop runs, for example when a user asks to stop now. The turns are
cancelled at once, and the stop returns within `shutdown_cancel_timeout`. Calling it when no stop is running does
nothing, and a later stop still gets its full grace period.
**Caveats.** `end_grace` is not a stop. Call `stop()` or `stop_with_grace()` first. A turn that ignores cancellation
is left behind after the cancel timeout, as at the normal end of the grace period.

## 2. Stop Now for a worker over HTTP

**Where.** `docs/guide/app.md`, `A worker over HTTP`, where `App::on_worker_ready` and `WorkerReady` are described.
**Text.** `WorkerReady::stop_now` is a `StopNow` handle. Keep a clone of it. Calling `stop_now.now()` ends the grace
period of the worker's stop, so the stop ends within the cancel timeout. The running turns are cancelled; one whose work
stops before its handler ended tries to release its lease (a lease whose release fails expires instead); a turn whose
handler had already ended may still complete. Call it after the stop signal, the future given to `run_with`. A call that
comes before the worker itself has begun stopping is kept, and so is a call before the stop signal: it applies as soon
as the stop begins. It never stops the worker by itself.
**Caveats.** Only a worker over HTTP reports `WorkerReady`. `serve` and a worker on the store keep their
signal-driven stop.

## 3. Stop Now in the menu bar app

**Where.** `docs/guide/app.md`, `Menu bar app on macOS` (from the tray-menu-bar handoff).
**Text.** While the worker drains after a disconnect or a Quit, the menu shows Stop Now under the header. Picking it
cancels the running turns at once, and the countdown drops to the cancel timeout. A turn that ends without an
acknowledged commit appears as Abandoned in Recent Turns. `Tray::stop_now()` does the same in code.
**Caveats.** Stop Now appears only for a worker that was ready. A connection interrupted while starting stops at
once anyway. A turn whose handler had already ended may still commit and appear as Done. The tray is offline within
the cancel timeout even when a turn's release is slow: that turn stays under Running until its release ends. Do not
call `stop_now` while holding a borrow of the state receiver.

## 4. Upgrading to 0.26.0

**Where.** `docs/how-to/upgrading.md`, the 0.26.0 upgrade (next to the `WorkerReady` items of tray-menu-bar).
**Text.** `WorkerReady` has a new field `stop_now` and no longer implements `PartialEq` or `Eq`. Compare its fields,
such as `types`, `grace_period` and `cancel_timeout`, instead of whole values.

## 5. Architecture

**Where.** `docs/architecture.md`, the worker's `Life cycle` item (the text "`stop` stops polling, waits
`shutdown_cancel_timeout` for the loop, gives the tracked turns the grace period").
**Text.** Add: `end_grace` ends the grace period early. It sets a token that exists only while `Stopping` and is
set under the lifecycle lock. The stop's waits for the polling loop and for the grace period end at that token, and
the stop goes on as at the end of the grace period. A call outside a stop is not remembered. A worker over HTTP in
`cicada-app` calls it for `StopNow` once its stop has set `Stopping`.

## Blocking

None.

## Status

Proposed prose for the core and app guides, the upgrade guide and the architecture. The changelog's 0.26.0 section
names the new items under Added and the `WorkerReady` breaks under its BREAKING line. Checks run: the docs checks
(11 link and guarantee tests, 51 doctests), `cargo fmt --all --check`, workspace Clippy with warnings denied, the
workspace run with PostgreSQL (3,785 passed, 0 failed) and `openspec validate worker-stop-now --strict`. The AppKit
row compiles and passes Clippy, but nobody has clicked it in a running menu yet. Fix round 1 (code review 1): the
stop after a panicking `on_worker_ready` function takes Stop Now; the tray's claims now separate cancellation,
abandonment and a turn that commits; a polling loop that hangs is tested; `stop_now` joins the borrow rule.
Rerun: `-p cicada --lib`, `-p cicada-app`, `-p cicada-tray`, the docs checks, Clippy and fmt (see the report).
