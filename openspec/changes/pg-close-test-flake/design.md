# Design

## Isolate the runtime whose task count proves the close path

The failing PostgreSQL test counts every task on the store's runtime. Aborting its listener can make sqlx spawn
UNLISTEN/connection cleanup there, so a higher count does not imply Cicada scheduled shutdown work. The behavior
already follows the store-backends spec and architecture: close is resumable, with no Cicada background shutdown.
This change fixes test attribution, not close itself; no new ADR, public API, wire, schema or guarantee.

Keep `a_cancelled_close_leaves_its_resources_to_the_final_drop_or_the_runtime_teardown_without_a_task` and its two
teardown legs. Set up the PostgreSQL store, subscription and borrowed connection on its existing multithread runtime.
Return these values to the synchronous test before entering a second, fresh current-thread runtime; never nest
Runtime::block_on calls. The store's workers stay alive so its aborted listener can finish and sqlx can clean up there.

Use the second runtime only to poll/drop close. In one uninterrupted block_on root poll: pin close in a scope, poll
it once with `futures::poll!`, assert Pending and pool.is_closed(), and leave the scope to drop the actual future.
Then assert this runtime's num_alive_tasks() is zero before yielding. The held connection ensures Pending even if
joining the listener completes immediately. The admission prefix runs before any await, so a polling loop is not
needed. No sleep, timeout, task spawn, database setup or connection drop runs on this measuring runtime.

Because it is current-thread and the root never yields, a task spawned during close's poll or cancellation cannot
finish before the count. Both tokio::spawn and a current Handle::spawn are detected. Driver cleanup triggered when
Tokio drops the aborted listener executes on the store runtime and does not contaminate this count. Return to the
store runtime for the ended-stream assertion and borrowed connection drop; do not move those into measurement.

Retain cleanup-progress Some((_, false)), which also shows no caller holds the cleanup mutex, the ended stream,
both runtime-alive/runtime-torn-down legs, Arc strong_count == 1 and failed Weak upgrade after final drop. Add the
existing listener_stopped() observation so no later start can create another listener. These are ownership/lifetime
proofs; they do not promise synchronous socket closure or joining the driver's own cleanup.

## State the proof boundary honestly

The dynamic count proves no task is left scheduled on the runtime executing that poll/drop, including guard Drop
code using the current runtime. It does not detect spawning via a previously captured runtime Handle, spawn_blocking,
an OS thread, or new Cicada work in the listener's abort path on the store runtime. Review the actual close/ownership
paths for those: the store holds no runtime Handle, close has no spawn, and Run has no custom Drop spawning work.
Do not present the count as a universal detector or weaken the no-Cicada-shutdown-task contract to just one runtime.
Driver-owned cleanup is expressly allowed already. Keep that distinction in the test comment and spec scenario.

Prove regression sensitivity with temporary, uncommitted mutations in the builder worktree: a tokio::spawn after
close sets its flag, and a guard dropped on cancellation that uses Handle::try_current().spawn. Each must fail the
targeted real-PostgreSQL test. Restore the source, verify a clean production diff, and rerun. Do not retain an injected
spawn, instrumentation helper or a test that greps source. Review evidence covers the structural exclusions above.

Reject a counter at selected spawn sites: a new direct spawn can bypass it. Reject waiting for the old count to
settle, which adds timing dependence and misses short tasks. Reject ownership assertions alone, which miss a task
holding a pool clone, and Tokio unstable task hooks/dumps, which would widen workspace configuration.

## SQLite and existing specs

Leave the SQLite twin unchanged. Its harness performs no WriteTx drop that spawns settle work, and current sqlx-sqlite
connection workers are OS threads, not Tokio tasks. Its own runtime has no unrelated Tokio source, so its exact counts
remain attributable and retain the later count after the stream check. Recheck this evidence against locked sources
at build time. Do not generalize the PostgreSQL workaround or loosen the SQLite assertions without new evidence.

The MODIFIED store-backends delta copies the full resumable-close requirement and clarifies its No retry yet scenario:
no Cicada shutdown task, while driver cleanup on the store runtime may occur. The measurement is test strategy here,
not a new restriction on sqlx. Preserve every existing scenario and cleanup boundary. No new spec capability.

## Handoff, verification and release

Builder owns tests/comments and change artifacts. Keep test names; no guarantee row, doctest, link or skill route
needs protected prose. Write docs-handoff.md with no new user behavior and Blocking: none, plus release version for
docs-owned pins. Add a test-fix changelog entry and normal patch version at build time. No handwritten docs edits.

Two implementation tasks plus finishing suffice: test restructuring with mutation checks; handoff/version/changelog.
Run the PostgreSQL regression repeatedly (20 targeted runs) and the unchanged SQLite twin, then normal fmt/clippy and
workspace checks with PostgreSQL. A second full run is justified for this load-sensitive flake; repetitions support
the structural fix, not a statistical promise of no flakes. Set CICADA_PG_URL to a running test database and verify
execution rather than counting a skip as evidence. No code fix or release is performed during design.
