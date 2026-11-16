# Design

The owner delegated design decisions on 2026-11-16, including the split from pinned-actors and Opus 5.5 review.
One worked example, five tasks plus finishing. ADR 0053 records its bounded application contract, not a new Cicada
guarantee. Files: examples/src/pinned_process.rs, examples/examples/pinned_process.rs and tests/pinned_process.rs.

## One actor, one run

1. A fresh, never-reused actor key identifies one run of one fixed example child program. The host creates it
   directly with CreateOptions pinned to its stable configured node id, then verifies the stored pin. No ready-made
   supervisor/flow/event creates or restarts it. Restarting a business job requires an explicitly created new run;
   a general-purpose process supervisor is outside this example. This makes repeat safety small enough to inspect.
2. The host generates a fresh host_epoch (MessageId::random) at each process start; it is distinct from the stable
   node id and never reused. Initial actor state stores host_epoch, desired Idle/Run/Stop, observed Idle/Starting/Running(pid)/Stopping/
   Exited(optional code)/SpawnFailed/StoppedBeforeStart/NeedsReconciliation, last observation revision and one bounded
   refusal reason. A PID is diagnostic only, never an identity to adopt or signal. State contains no live handle.
3. Messages are Start, Stop, Refresh and Observed {host_epoch, revision, status}. Start requests this run once;
   further Start messages refer to the same run regardless of MessageId. Terminal observations are absorbing;
   Start after completion/Stop is refused without another spawn. Stop records desired Stop and requests host stop;
   Stop before Start reserves a terminal tombstone: even a lost Stop commit leaves stop intent in the host. It
   consumes one of 32 slots deliberately. Natural exit does not restart.
4. All host actions check Turn::node_id against the adapter's configured id and state.host_epoch against its current
   epoch. A mismatch performs no OS action; nonterminal runs become NeedsReconciliation and already confirmed
   terminal runs remain terminal. Refresh/start/stop after a new host epoch
   cannot adopt, kill or respawn old runs. Every event checks host_epoch and strictly increasing revision; duplicates,
   stale/out-of-order events and a Running event after terminal are ignored. An event never changes desired intent.
   NeedsReconciliation is absorbing too: even a late terminal event from the original host cannot clear it.
   A second live host sharing the id may mark a still-live run uncertain; unique ids are the deployment obligation.
   These are trusted application messages; sender ids are not authentication.

## Host ownership and the commit gap

5. Construct Arc<ProcessHost> before ActorSystem and inject it into the actor implementation. The host keeps at most
   32 slots indexed by actor address, with immutable host_epoch and per-slot revision. Retain every completed slot
   until host shutdown: no tombstone eviction or slot reuse while a lost claim could replay. Capacity refusal starts
   nothing and is visible as a bounded refusal; a fresh host epoch permits new slots but never restarts old runs.
6. Reserve the slot synchronously before spawning or yielding. A single owned per-slot task starts the fixed child
   at most once, owns its Child handle, waits for exit and receives an idempotent stop signal. Repeated Start returns
   the same Starting/Running/terminal snapshot; cancellation of a handler awaiting that result does not cancel the
   owned task or erase the reservation. Stop is sticky even if it races Starting. No lock is held across an await.
   A spawn error becomes a retained terminal record; retrying a failed command does not repeatedly spawn.
7. Host slot states include Starting and Stopping internally; actor snapshots may report them while pending. Start
   completion is not a durable acknowledgment: the outside effect may happen before the actor turn commits, and the
   slot lets a later claim observe it without repeating it. Runtime restart on the SAME ProcessHost can reconcile;
   process restart with a NEW host_epoch cannot. A crash may leave an orphan. No exactly-once OS effect is promised.
8. A host pump takes ActorSystem only after it is built, avoiding a construction cycle. It sends latest observations
   as ordinary messages (one stable message id per run/revision) and retries uncertain sends with that id. Keep one
   pending latest observation per slot, not an unbounded event queue; a terminal snapshot subsumes Running. Retry
   once per second with a five-second send bound until accepted, actor absent, or shutdown. Actor idempotence handles
   repeats beyond transport dedup retention. All send errors are surfaced; terminal records are never discarded.
9. On startup, page through this example's actors and enqueue Refresh for those pinned to this node; refresh of a
   previous epoch exposes NeedsReconciliation for nonterminal runs without acting. Stop/delete of an actor does not automatically stop
   an OS child: the host retains ownership and shutdown cleanup, and guidance requires Stop plus confirmed terminal
   before ordinary deletion. Exception: after independently verifying/cleaning up an uncertain old resource, the
   operator may delete its NeedsReconciliation actor with the existing delete API. No Abandon message is added.
   Deletion itself does not verify cleanup; a replacement still needs an explicit new never-reused key. The demo must not create a replacement automatically or signal a stored PID.

## Runnable adapter and limits

10. The binary has a hidden child mode of the same executable, with no shell or arbitrary user-supplied command.
    The child path is injected by trusted host construction (current_exe in the demo, a fixed fixture in tests).
    Child performs no external business effect, spawns no descendants, and supports controlled normal exit or waiting
    for stop through a bounded stdin control. Null stdout/stderr avoid pipe growth. Enable Tokio process only for
    examples. The parent uses a user-chosen SQLite file and stable node id; a new run requires an explicit flag.
    Default startup reconciles existing runs and reports their state. Keep example protocol separate from core HTTP.
11. Bound command/result waits to five seconds; timeout means unknown, never not-started or a new spawn permission.
    Stop sends kill once through the owned Child handle then waits/reaps; successful kill request alone is not exit.
    Shutdown closes host admission first, then stops/reaps slots and flushes their latest observations while the
    ActorSystem, pump and store remain open, within one five-second cleanup budget. Then stop ActorSystem with its
    configured bounded grace, stop/join the pump and close the store last. Failed flush/cleanup is reported and
    exits nonzero; unconfirmed cleanup never reports Stopped. kill_on_drop is a best-effort
    fallback only, not a promise after abrupt host death. No PID adoption, process groups or arbitrary-program fencing.

## Tests and documentation ownership

12. Actor and adapter-protocol tests use TestActors/paused time with a deterministic fake process backend. Cover
    start/repeat, natural exit, stop-before-start, stop/spawn race, capacity/tombstone retention, invalid/stale events,
    lost send responses, stalled operations, actor deletion and shutdown. A public TurnSource test wrapper withholds
    completion until lease expiry to exercise a real lost-commit replay against the SAME host table: spawn count one.
    A new epoch with the same node id must yield NeedsReconciliation and zero spawn/kill/adoption of old runs.
    Test a late original-host terminal event after that mark, and deletion after simulated outside verification.
13. Separately test OS plumbing in tests/pinned_process_os.rs, with no actor runtime or virtual-time assumptions.
    A synchronous test runs its own test executable in a narrowly selected helper mode under a bounded watchdog;
    helper uses the same adapter and a fixed inert child fixture for start, normal exit, repeated start and stop/reap.
    The test parent retains owned Child handles for timeout cleanup; use handshakes, not sleeps as success evidence.
    Run these OS tests on Linux/macOS (explicit cfg); other platforms retain protocol tests with no OS assurance.
    cargo test runs them normally there and cargo check --examples checks the demo. No separate gate script/check.
    State platform assumptions and actual evidence; these tests do not prove cleanup after abrupt host death.
14. Export the module, add both consumer skills' worked-example route coverage and prepare docs-handoff.md for
    design guide Work with outside systems/A websocket session comparison, core Examples and operations cleanup/
    restart caveats. The guide and both skill predecessors must be landed before building; use their actual routes,
    not an unbuilt proposal as a consumer source. The docs owner chooses prose within existing headings; no new guide is needed. Keep doctests,
    links and skill checks blocking until exact owner files are integrated. No new G/S row unless a test actually
    proves an existing core guarantee; the example's at-most-one spawn scope is not G4.
15. Builder owns source/rustdoc/tests, ADR/index and ordinary release pins/changelog. Finish with targeted
    example/OS/docs/doctest/skill tests, strict OpenSpec and committed-tree PostgreSQL/Docker gate.

Adapter basis: Tokio's [Child documentation](https://docs.rs/tokio/latest/tokio/process/struct.Child.html) and
[process lifecycle caveats](https://docs.rs/tokio/latest/tokio/process/), checked 2026-11-16: explicit waiting/reaping
and best-effort drop cleanup. The example's run/epoch/tombstone protocol supplies its own application limits.
