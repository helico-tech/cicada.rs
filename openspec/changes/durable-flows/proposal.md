# Proposal

## Why

A job of several steps is written in cicada as a state machine: an enum of steps, a field per result in the state,
and a message per step that the actor sends to itself ([examples/src/steps.rs](../../../examples/src/steps.rs)). It is
correct, since a committed step never runs again (G2, G4), but every job repeats the same code, and the order of its
steps is spread over a `match`.

Inngest, Temporal, Restate and DBOS let a developer write the same job as plain code. Each call of a step runs once and
its result is saved; when the job runs again, the saved result is returned instead. Inngest also starts a run of each
function that listens to an event, retries each step on its own, and sleeps durably. Cicada has the parts under all of
this: an actor per run, one commit per step, delayed sends, a retry policy per message, idempotent creates, watches and
deletes. What it lacks is the programming model on top. This change adds that model in layer 3, with no change to the
core.

## What Changes

- A new module `flow` in `cicada-actors`, on the public API of `cicada` only. The crate's rule that each actor has a
  fixed type name gets one exception: the runs of a flow take the flow's type name.
- A host writes a **flow**: a type that implements the trait `Flow`, with a type name, the type of the event that
  starts it, and `async fn run(&self, step: &mut Step<'_>, event: Self::Event) -> FlowResult<()>`.
- Each **run** of a flow is one actor of the flow's type name. Its state holds the event, the saved result of each
  step by its id, and whether the run is running or done. Results are kept as JSON.
- `FlowActor<F>` hosts the runs of `F`. Each turn runs the flow from the top:
  - `step.run(id, f)` returns the saved result of `id` when there is one. Otherwise it runs `f`, saves the result,
    sends the run a `Continue` and returns `Suspended`, which the flow passes on with `?`. The turn then commits the
    result and the `Continue` together, so a committed step never runs again.
  - `step.sleep(id, duration)` saves the wake time the first time, sends `Continue` at that time (`SendOptions::at`,
    the store clock) and suspends. Once the time has passed it returns at once.
  - A step that fails fails the turn. `Continue` carries the flow's retry policy, so the step is retried after its
    backoff, and the steps before it return their saved results. Out of retries, the message ends `failed` and the
    run's watchers get a `Failed`, so the dead letters or a host's own watcher take it from there.
  - Each step gets an idempotency key, `<run address>/<step id>`, for the calls it makes: a step runs again when its
    turn ends without a commit (G4).
  - When `run` returns, the run is marked done and stays, so its results can be read.
- **Events**: a ready-made actor per event name, `cicada.event/<name>`, keeps the flows that start on it. A `Publish`
  creates a run of each of those flows, keyed by the event's id, and sends it `Start`, all in one commit. A create of a
  run that exists does nothing, so an event published twice starts each flow once.
- Cancel is the core's `delete` of the run; pause and resume are the core's too. A run's state is read through the
  existing actor reads, with no new endpoint.
- A section on flows in the actors guide, with doctests, and a worked example: the job of `examples/src/steps.rs`
  written as a flow.
- An ADR for the flows and their rule: a flow calls its steps in the same order on every run.

The simplest version: `step.run`, `step.sleep` and the start by event; one flow per type name; results as JSON in the
run's state.

## Capabilities

### New Capabilities

- `flows`: a flow written as plain async code, each run an actor, each step run once and saved, a durable sleep, a
  retry per step, and runs started by an event, once per event id.

### Modified Capabilities

None.

## Out of scope

Each is a later change, when someone needs it:

- `step.wait_for_event`: a run that waits for a matching event, with a timeout. It needs a lookup from a match key to
  the waiting runs.
- `step.invoke`: a run that starts another flow and waits for its result.
- Parallel steps. An actor runs one turn at a time (G1), so each parallel step needs a child actor.
- A limit above one on the runs at once per key, and priority. They need a notice that a message succeeded, which the
  core does not give.
- Matching events by an expression, and a filter on the data of an event.
- Cron, rate limits, debounce and batching as settings of a flow. The scheduler, rate limiter, debouncer and batcher
  can send to an event actor; the guide shows how.
- Moving runs in flight to a new version of a flow.
- Results kept outside the run's state, for large outputs.
- A view of runs beyond the existing HTTP API and terminal UI.
- Changes to the core.

## Questions for the design

- The names: `Flow`, run, `Step`, `Suspended`, `cicada.event`.
- How does a flow end its turn after a new step: an error that `?` carries out, or does the runner drop the flow's
  future? The first shows in the flow's signature; the second does not, but is harder to follow.
- Is a change of the order of steps caught? A run could save the order of its steps and fail for good when a run asks
  for another step at a place, instead of returning a wrong result.
- Is the event actor new, or is it `cicada.topic`? The topic delivers to actors that exist, and a run does not exist
  before its event.
- How does a flow get onto its event actor: does the node subscribe it at start, or the host?
- What is the key of a run: an event id the publisher gives, or the id of the `Publish` message, which the store
  already keeps unique (S7, S10)?
- Does a done run stay until a host deletes it, or stop itself after a time? While it stays, the same event id starts
  no new run.
- Limits: the most steps in a run, or the largest state. The whole state is read and written on every turn, and a
  result sits about three levels deep in it, toward the depth limit of 64 (ADR 0029).
- How does `App` host a flow: as an actor type like any other, or through a call of its own?

## Order

After `batcher-actor`, the last of the line.

## Impact

- `crates/cicada-actors` (a module `flow`, its tests on virtual time, `tests/wire_formats.rs`),
  `docs/reference/contracts` (a new file for `cicada.event`), [docs/guide/actors.md](../../../docs/guide/actors.md) and
  its doctests, a worked example in `examples/`, an ADR and its row in `docs/adr/README.md`, `docs/architecture.md`
  (the crate table: the exception to fixed type names), `README.md` (the table of actors), `AGENTS.md` (the map),
  `CHANGELOG.md`, and possibly the sample service to host a flow.
- Hosts: nothing changes for a host that does not use flows. No host has asked for it. Chorus builds its own workflows
  with their steps in state (CHANGELOG, 0.16.0); whether flows would serve it is worth asking.
