# Docs handoff for job-graph

Version: 0.20.8 (the unreleased batch opened by capacity-permits). Refresh the dependency pins of
`docs/how-to/install.md` once it is published.

## What changed

`cicada-actors` gets a ready-made job graph (ADR 0059): one actor per run holds a graph of jobs given as data, starts
each job once every job it needs succeeded, up to an in-flight limit, takes an explicit result per attempt, and reports
the run's end to a target.

Public items (all re-exported at the crate root, module `cicada_actors::job_graph`):

- `JobGraph::named(run)`: the ref, `cicada.job-graph/<run>`. `JobGraphActor`: the actor type, no settings.
- `Job::new(name, &target_ref, message_type, input) -> Result<Job, InvalidJobGraph>` and `.with_needs([...])`: a job
  definition (`name`, `needs`, `to`, `message_type`, `input`). Jobs of one graph may go to different actor types.
- `JobGraphState::new(jobs, max_in_flight, &report_to_ref, report_type) -> Result<_, InvalidJobGraph>`: fields
  `max_in_flight`, `report_to`, `report_type`, `status: JobGraphStatus`, `ended_at`, `jobs: Vec<JobEntry>` (the job's
  definition plus `status: JobStatus`, `output`, `attempts: Vec<JobAttempt>`). `JobAttempt`: `attempt`,
  `dispatched_at`, `ended_at`, `outcome: Option<JobOutcome>`, `reason`. On the wire
  `{"maxInFlight":2,"reportTo":"ci/run-7","reportType":"GraphEnded","status":"running","endedAt":null,"jobs":[{"name":
  "build","needs":[],"to":"builder/run-7","messageType":"Build","input":{},"status":"waiting","output":null,
  "attempts":[]}]}`; a create may leave out `status`, `endedAt` and a job's `status`, `output` and `attempts`.
- `JobGraphMsg`: `Start`, `Result { job, attempt, outcome, output?, reason? }` (helpers
  `JobGraphMsg::succeeded(&start, output)`, `JobGraphMsg::failed(&start, reason)`, which cuts the reason to 2,000
  characters), `Report`; and the run's own `Dispatch { job, attempt }`, `Failed`, `Stopped`.
- `JobStart { run, job, attempt, input }`: the job message's payload; a worker's family takes it as e.g.
  `Build(JobStart)`.
- `JobGraphEnd { run, status, jobs: Vec<JobReport> }`, `JobReport { name, status, attempt, output, reason }`: the end
  report; a report target's family takes it as e.g. `GraphEnded(JobGraphEnd)`.
- `JobGraphStatus` = `running` | `succeeded` | `failed` | `cancelled`; `JobStatus` = `waiting` | `running` |
  `succeeded` | `failed` | `skipped` | `cancelled`; `JobOutcome` = `succeeded` | `failed`. `cancelled` (run and job)
  and `skipped` are never set by this release (they come with `job-graph-failures` and `job-graph-cancel`).
- `InvalidJobGraph`.

Proved by `crates/cicada-actors/tests/job_graph.rs` (19 tests on virtual time, one or more per spec scenario), the unit
tests in `crates/cicada-actors/src/job_graph.rs` (constructor and decoder) and `tests/wire_formats.rs` (contract against
`docs/reference/contracts/cicada.job-graph.json`, samples of each message, a created and a mid-run state, a `JobStart`,
a `JobGraphEnd`, and the states a run writes). No worked example in `examples/` (design: the skill test would ask a
route for it, which needs the guide section first).

## 1. A job graph section in the actors guide

**Where.** `docs/guide/actors.md`, a new section `## Job graph` (anchor `#job-graph`), after `Permit pool`; and a row
in the table at the top: `| Run a graph of jobs given as data | JobGraphActor / cicada.job-graph | [job graph tests](../../crates/cicada-actors/tests/) |`.
**Text.**
- Create the run with its graph and send `Start` in one commit: `JobGraphState::new(jobs, max_in_flight, &report_to,
  "GraphEnded")`, then `JobGraphMsg::Start`; host `JobGraphActor`. A bad graph (a cycle, an unknown or repeated need, a
  self-need, bounds) is refused by the constructor and the decoder, so it never becomes a run. Rust block from the
  `JobGraphState::new` doc example in `crates/cicada-actors/src/job_graph.rs`.
- A job is a message to a target: the worker's family takes the job's message type as `Build(JobStart)`. Any actor can
  be a worker (a host actor, a flow, a container actor), and one graph can mix types.
- The worker answers with `JobGraphMsg::succeeded(&start, output)` or `JobGraphMsg::failed(&start, reason)` to
  `JobGraph::named(start.run)`. Rust blocks from those two doc examples. The sender is not checked: the work may be
  handed on to whoever reports.
- The attempt fence: only a result that names the job's current attempt, while the job runs, is taken; any other
  (unknown job, job not running, another attempt, a late result) is refused for good and kept as a failed message of
  the run, which dead letters keep.
- The in-flight limit is per run (`maxInFlight`, 1–100), dispatched or not; ready jobs start in graph order. A limit
  across runs is not this actor's (a permit pool can give one).
- A failed job blocks the jobs that need it (they stay `waiting`); jobs that do not need it still run. The run ends
  when no job runs and none can start: `succeeded` when every job succeeded, else `failed`.
- A gone target, a target type no node registers, or a type that does not take the message type fails only that job's
  attempt, with the reason `the job message was not sent (<cause>): <error>`; the run goes on.
- The end report `JobGraphEnd` goes to the report target in a turn of its own. A gone report target fails only that
  `Report`; send `JobGraphMsg::Report` again once it exists. Anyone may resend it, so a report target treats a
  repeated report for the same run as one.
- Bounds: 100 jobs; names 1–64 of `A–Z a–z 0–9 - _ .`; input and output 16 KiB as JSON and 32 levels; reason 2,000
  characters; 20 attempts kept per job.
**Caveats.** A worker that fails or goes away after it took its job message leaves its job `running` in this release:
failure notices, skipped dependents and cancel come later (`job-graph-failures`, `job-graph-cancel`). No retry of one
job yet (`job-graph-retry`). The graph is fixed for the run.

## 2. The actor in the map and the catalog

**Where.** `AGENTS.md`, the `crates/cicada-actors` row of `## Map`: add `job_graph` to the list of ready-made actors
(after `permits`, which the permits handoff adds). `docs/architecture.md`, the catalog of ready-made actors: a line for
`cicada.job-graph` (ADR 0059).
**Text.** "`job_graph`, a graph of jobs given as data, one run per actor, with an in-flight limit, an attempt fence and
an end report".

## 3. A pattern in the design guide

**Where.** `docs/guide/design.md`, `## Coordinate work and time` (anchor `#coordinate-work-and-time`).
**Text.** When to use which: a job graph when the caller supplies the steps and their dependencies as data and wants to
inspect, and later retry, job by job (CI pipelines, media processing, imports); a flow when the steps are fixed and
written as code; reply waits when one actor needs "all of" or "first of" a few replies.

## 4. A skill route

**Where.** `skills/cicada/SKILL.md`, the routes of ready-made actors.
**Text.** A route to `docs/guide/actors.md#job-graph` for "run a graph of jobs / a pipeline given as data". It can only
land with the guide section, as the skill test checks that the anchor resolves.

## Blocking

None: no doctest, link, guarantee row or skill route names the job graph yet.

## Status

Ran `cargo test -p cicada-examples --test docs` and `--test skill` (both pass) on the branch head; nothing protected was
edited. All sections above are proposed text; no prose is written yet. After landing, the change loop links the change
in issue #2.

## Notes from the code review (added by the change loop)

- Because the run watches itself, every refused `Result` or `Report` and every failed `Dispatch` costs the run one
  extra turn (a `Failed` about its own message, which it ignores or uses). Harmless; worth a line in the guide.
- The table row should link to `crates/cicada-actors/tests/job_graph.rs`, not the `tests/` folder.
