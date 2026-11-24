# Design

Picks of the change loop, 2026-11-24, on the proposal's list, checked against the code (the batcher, the topic, the
permit pool's design) and the reference (brood `crates/brood-core/src/pipeline/actor.rs` and `pipeline.rs`), updated
after design review 1. ADR 0059.

## Decisions

1. **A module `job_graph` in `cicada-actors`, type name `cicada.job-graph`, one actor per run**: `JobGraph::named(run)`,
   hosted by `JobGraphActor` with no settings. Hyphenated like `cicada.rate-limiter`, not `cicada.job_graph`: every
   type name of the crate uses hyphens. Not a crate `cicada-jobs`: no outside dependency, like the other ten.
2. **The graph is the state, given at the create; `Start` starts it.** `JobGraphState::new(jobs, max_in_flight,
   &report_to, report_type)` checks every bound and the graph, and the decoder checks the same, so a cycle, an unknown
   or repeated need, a self-need or a graph out of bounds is refused before the actor exists. A host creates the run
   and sends `Start` in one commit. `Start` starts ready jobs and is a no-op once nothing more can start, so a second
   one is harmless. Not a `Submit` message to an empty run: a state without a graph, and a refusal after the create.
3. **A job is a message to a target, per job, in the batcher's form**: `{name, needs, to, messageType, input}`. The
   payload is `JobStart {run, job, attempt, input}`; a worker's family takes it as e.g. `Build(JobStart)`, and reports
   to `JobGraph::named(run)`. Any actor can be a worker (a host's worker, a flow, a container actor), and jobs of one
   graph can go to different types. Not one target per graph: one worker type that routes by job name. Not a child
   actor of a host type (brood): it ties the graph to one type.
4. **Dispatch goes in a turn of its own.** A job that becomes ready is marked `running` with a new attempt in the
   commit that made it ready, which also sends the run an internal `Dispatch {job, attempt}` (as the topic's
   `Deliver`). Only a `Dispatch` the run sent itself, for the job's current attempt not yet dispatched, sends the job
   message (with the crate's shared retry policy) and sets the attempt's `dispatchedAt` in its commit. Not in the
   same commit: a gone target of one job would reject the commit of another job's result and jam the run.
5. **A dispatch that fails fails the attempt, and the run goes on.** Each turn that sends a `Dispatch` watches the run
   itself. A send the call refuses (target type not registered, a type that does not take `messageType`, a payload
   out of bounds) fails that `Dispatch` for good (cause `refused`); a gone target rejects its commit (`commit`). A
   `Failed` the store wrote for the run's own `Dispatch` of a current attempt without `dispatchedAt` ends that
   attempt `failed`, reason `the job message was not sent (<cause>): <error>`: its commit never happened, so no worker
   has it. Other `Failed`, a client's, and `Stopped` are ignored. A worker that fails later is `job-graph-cancel`'s.
6. **A worker reports `Result {job, attempt, outcome, output?, reason?}`**, outcome `succeeded` or `failed`. It is
   taken only when the job is `running` and names its current attempt; the attempt ends, the job takes the outcome and
   keeps `output`, and ready jobs start. **The sender is not checked**: a target may hand the work on (to a flow, a
   container actor, a process that reports over HTTP), and the permit pool does not check either. Helpers:
   `JobGraphMsg::succeeded(&start, output)` and `failed(&start, reason)`, which cuts the reason to the bound.
7. **Any other result is refused for good, state unchanged**, with the reason (unknown job, job not running, attempt
   not current): the store records it as a failed message of the run, which dead letters keep. This is the hook of
   `job-graph-retry`. Not a list of refused results in the state: one more bound and invariant. Not ignoring: unseen.
8. **Attempts are numbered per job from 1**, each kept as `{attempt, dispatchedAt, endedAt, outcome, reason}`, at most
   20 per job, the oldest dropped; the last is the current one. In this change a job has at most one.
   **Job statuses include `skipped` and `cancelled` now**, set only by `job-graph-cancel`: so cancel does not change
   the contract of a host type that takes `JobGraphEnd`. Not a second contract change: a release can ship this alone.
9. **The in-flight limit is per run**, `maxInFlight` 1–100, given at the create: running jobs, dispatched or not,
   never exceed it. Ready jobs start in graph order. A limit across runs is out of scope (later via `capacity-permits`).
10. **A failed job blocks its dependents; the run ends when nothing runs and nothing can start.** Jobs that do not need
    the failed one still start and finish. The run ends `succeeded` when every job succeeded, else `failed`, with
    `endedAt`; blocked jobs stay `waiting`. `job-graph-cancel` refines this: it marks them `skipped`.
11. **The end report goes in a turn of its own**: the commit that ends the run sends itself `Report`, which sends
    `reportType` to `reportTo` with `JobGraphEnd {run, status, jobs}`, each job with its status, last attempt, output
    and reason. `Report` is public, like the batcher's `Flush`: an operator resends a lost report; on a running run it
    does nothing. A gone report target then fails `Report` only, not the last result. So a report target can get the
    same report more than once, and treats a repeat for the same run as one.
12. **Bounds, refused for good, state unchanged, never clamped**: 1–100 jobs; names 1–64 characters of ASCII letters,
    digits, `-`, `_`, `.`, unique; needs unique and named; `messageType` and `reportType` 1–200 characters without NUL;
    `input` and `output` at most 16 KiB as JSON and 32 levels deep; `reason` at most 2,000 characters. The decoder also
    checks the attempts (at most 20, consecutive numbers ending at the current one, only the last open), that a job's
    status matches its last attempt, that a started job's needs all succeeded, that at most `maxInFlight` run, and that
    the run is ended exactly when none runs and none is ready. Cycles are found with Kahn's algorithm, no recursion.

State on the wire: `{"maxInFlight":2,"reportTo":"ci/run-7","reportType":"GraphEnded","status":"running",
"endedAt":null,"jobs":[{"name":"build","needs":[],"to":"builder/run-7","messageType":"Build","input":{},
"status":"running","output":null,"attempts":[{"attempt":1,"dispatchedAt":null,"endedAt":null,"outcome":null,
"reason":null}]}]}`. Run status `running | succeeded | failed | cancelled`; job status `waiting | running | succeeded |
failed | skipped | cancelled`; the run's `cancelled` and the last two job statuses are set only from `job-graph-cancel`.

## Contract and tests

The contract of `cicada.job-graph` is checked against `docs/reference/contracts/cicada.job-graph.json` in
`crates/cicada-actors/tests/wire_formats.rs` (one more actor than then), with samples of each message, a state mid-run,
a `JobStart` and a `JobGraphEnd`. Tests in `crates/cicada-actors/tests/job_graph.rs`, on virtual time with
`TestActors`, cover every scenario of the spec; unit tests cover the constructor and the decoder.

## Version

Additive: the next patch. No guarantee, core wire format, schema or command line change; a new actor contract, hence
ADR 0059. With `skipped` and `cancelled` in the contract now (decision 8), this change can be released on its own.

## Docs it breaks

None: no doctest, link, guarantee row or skill route names the job graph yet. No worked example in `examples/src`: the
skill test asks a route for each, which needs the guide section first. The builder updates the code that lists every
ready-made actor: the rustdoc table of `crates/cicada-actors/src/lib.rs` and the count in `wire_formats.rs`'s module
comment (the sample service's catalog lists only some). For the docs handoff: a `## Job graph` section in
`docs/guide/actors.md` and its row in the table at its top (create the run with its graph and `Start` in one commit,
the `Build(JobStart)` variant, the report helpers, the attempt fence, the in-flight limit, a failed job blocks its
dependents, a gone or refused target fails its attempt, the end report and resending it, and that a report target
treats a repeated report for the same run as one); the actor in the AGENTS.md map and `docs/architecture.md`'s
catalog; a pattern in `docs/guide/design.md#coordinate-work-and-time` (data graph vs flow code vs reply waits); the
route `docs/guide/actors.md#job-graph` in `skills/cicada/SKILL.md`. After landing, link the change in issue #2.
