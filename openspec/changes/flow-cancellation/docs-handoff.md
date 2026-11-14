# Docs handoff for flow-cancellation

The docs session owns `README.md` and the hand-written docs under `docs/` until `build-loop-docs-handoff` lands. This
file holds the prose this change needs from it. The text below is complete, so the file stands alone. It builds on the
docs handoff of `flow-wait-join` (`openspec/changes/archive/2026-11-14-flow-wait-join/docs-handoff.md`): apply that one
first, then this one; the places named below are as that handoff leaves them.

What the code branch holds: the `Cancel` branch of `FlowActor` in `crates/cicada-actors/src/flow.rs` (the private
`cancel` function, and the status check moved before the message dispatch), with reworded doc comments on the module,
`FlowMsg`, `FlowMsg::Cancel`, `FlowStatus`, `FlowStatus::Cancelled` and `FlowActor`, and a compiled example on
`FlowMsg`; actor tests in `crates/cicada-actors/tests/flow.rs` (section "Cancellation"); the regenerated descriptions
of `Cancel` and `FlowStatus` (`crates/cicada-actors/tests/contracts/test.sign-up.json`,
`sample-service/tests/contract-texts.json`; hashes and description-stripped schemas unchanged); an "In Rust (0.18.4)"
paragraph in ADR 0044 and its index row; the changelog entry, version 0.18.4, `docs/openapi.json` and the skill's
`cicada-version`. No new public item, CLI, HTTP route, store schema, ready-made actor contract, worked example or
guarantee: `docs/reference/` and `docs/guarantees.md` need nothing. The skill's routes need nothing new; if part 1
below gets its own heading, `skills/cicada/SKILL.md` may add `docs/guide/actors.md#cancelling-a-run` to its flows row.

What users must learn, in one place: send `FlowMsg::Cancel` to a run like any message. Sending only queues it. When
its turn runs, a running run is cancelled in one commit: the armed `Continue` is cancelled, the status becomes
`cancelled`, `armed` becomes null, and the input, the journal, the replies and any open wait stay as saved. No flow code
runs and no cleanup runs: requests, children and external resources keep running. The `Cancel` waits its turn in the
mailbox; it does not interrupt a running execution, and effects already made are not undone. A cancelled run is
terminal: it ignores `Continue`, `Reply` and `Cancel`, stays readable until deleted, and a new job needs a new run key.
Read the status to learn the outcome. Upgrade every host and worker of the flow type before sending `Cancel`: 0.18.3
refuses it for good, despite equal hashes, and the run stays running.

## 1. Cancelling a run in the actors guide

**Where.** `docs/guide/actors.md`, `## Flows`: its rules table, and a new subsection after `### Joins: waiting for
several replies` and before `## Event`.

**Changes to existing text.**

- Rules table, row "Completion": replace with "Done and cancelled runs ignore Continue, Reply and Cancel and retain
  their progress until deleted".
- Rules table, row "Available in 0.18.3": rename to "Available in 0.18.4" and write "Steps, sleeps, sends, requests,
  single waits and joins, and cancellation of a running run; cleanup after cancellation is not yet available".
- Rules table, row "Recovery": add at the end "; it never restarts a cancelled run".
- The upgrade sentence at the end of `### Requests and reply waits`: add "and again before sending Cancel (0.18.4
  rollout, linked as `../how-to/upgrading.md#enable-flow-cancellation-in-0184`, the section of part 3)".
- The sentence "A timeout neither stops the target's work nor cleans it up; send an application cancellation message
  if needed." stays; a `FlowMsg::Cancel` does not stop the target either.

**New subsection.** Agreed text; the Rust block is the example on `FlowMsg` in
`crates/cicada-actors/src/flow.rs`, so it compiles as a guide doctest with the same hidden lines (the `#` lines, as
the guide's doctest comment form needs them):

> ### Cancelling a run
>
> Send `FlowMsg::Cancel` to end a run, like any other message. Sending it only queues it; read the run's status, or
> reread it after a change hint, to learn that it ended.
>
> ```rust
> # use std::time::Duration;
> # use cicada::prelude::*;
> # use cicada_actors::{Flow, FlowActor, FlowMsg, FlowRun, FlowState, FlowStatus, Step};
> # use cicada_test::TestActors;
> struct Nightly;
>
> impl Flow for Nightly {
>     const TYPE: &'static str = "nightly";
>     type Input = String;
>
>     async fn run(&self, step: &mut Step, _input: String) -> HandlerResult {
>         step.sleep("wait", Duration::from_secs(3600)).await?;
>         step.run("report", |_key| async { Ok(()) }).await
>     }
> }
>
> # #[tokio::main(flavor = "current_thread", start_paused = true)]
> # async fn main() {
> let test = TestActors::builder().with(|node| node.host(FlowActor::new(Nightly))).build().await.unwrap();
> let run = FlowRun::<Nightly>::named("n1");
> test.create(&run, &FlowState::new("n1".to_owned())).await;
> test.send(&run, FlowMsg::Continue).await;
> test.run_until_idle().await; // the run sleeps, its `Continue` armed for the wake time
>
> test.send(&run, FlowMsg::Cancel).await; // queued: the run is not cancelled yet
> test.run_until_idle().await;
> let state: FlowState<String> = test.state(&run).await;
> assert_eq!((state.status, state.armed), (FlowStatus::Cancelled, None));
> assert_eq!(state.journal.len(), 1, "the saved sleep stays");
> assert!(test.pending(&run).await.is_empty(), "the timer is cancelled");
> # }
> ```
>
> The `Cancel` turn runs no flow code. In one commit it cancels the armed `Continue` (a sleep's timer, a deadline, a
> retry waiting for its backoff, or the next execution), sets the status to `cancelled` and clears `armed`. The input,
> the journal, the replies and an open wait stay exactly as saved, so you can still read what the run did. A run that
> was never started, waits for a reply, lost its timer or cannot replay is cancelled the same way. If the commit fails,
> nothing of it is saved and the run keeps running.
>
> **Cancel does not interrupt and does not clean up.** It is handled in mailbox order `(deliverAt, seq)`, after the
> turn before it. An execution that is running when you send `Cancel` may still commit its step, and the `Cancel` then
> cancels the next one. Effects already made stay made: a cancelled run may have called services, sent messages or
> started jobs. A reply or a timeout committed before the `Cancel` stays saved, but no flow code reads it. Requests,
> child actors and external resources keep running; their later replies are ignored. Cleanup after cancellation
> comes in a later release; until then, release resources in your application. Runs cancelled now will not be cleaned
> up when that release arrives.
>
> A cancelled run is final. `Continue`, `Reply` and another `Cancel` change nothing, and the run stays until you delete
> it. Use a new run key for a new job. A `Cancel` to a done run does nothing. Anyone who may send to the run may cancel
> it; the flow checks no sender, so control access as for any message. The `Cancel` turn is an ordinary done turn: no
> watcher is told, and the cancelled `Continue` has no turn. The state write can send a change hint, but hints are best
> effort, so reread the status rather than wait for an event.
>
> | Control | What it does to a flow run |
> | --- | --- |
> | `FlowMsg::Cancel` | Ends the run: status `cancelled`, armed `Continue` cancelled, progress kept; no cleanup |
> | Cancel a pending message | Cancels that one message, for example the armed `Continue`; the run stalls, still running |
> | Cancel a running turn (operator) | Stops the running execution at the next heartbeat; the run stalls, still running |
> | Pause | Holds every message, `Cancel` too, until resume |
> | Purge | Discards every pending message, a queued `Cancel` too; the run stays running and stalls |
> | Delete | Removes the run and its mailbox without running code; a queued `Cancel` never runs |
> | `FlowMsg::Continue` | Restarts a stalled running run; does nothing to a done or cancelled run |
>
> A stalled run (its armed `Continue` cancelled, purged or failed) can be resumed with `Continue` or ended with
> `Cancel`. A `Cancel` with a delay is an ordinary delayed message, not a deadline: the run may finish first.

## 2. Operations

**Where.** `docs/operations.md`, `## On call`.

- Table "Symptom / Check / Action": add a row. Symptom "Flow Cancel failed". Check "Its error names a refusal for good
  by an older release, or the commit was rejected". Action "The run is still running. Upgrade every host and worker of
  the type, then send a new `Cancel`, retry the failed one or redrive it from the dead letters".
- Same table, row "Timer chain stopped", Action: after "flow: Continue" add "(or `FlowMsg::Cancel` to end the run)".
- `### Destructive controls`, after the table, add: "For a flow run, `FlowMsg::Cancel` is the way to end it and keep
  its progress; cancelling its pending or running message only stalls it (see the actors guide,
  `guide/actors.md#cancelling-a-run`)."

## 3. Upgrading

**Where.** `docs/how-to/upgrading.md`, a new section after `## Enable flow joins in 0.18.3`. Agreed text:

> ## Enable flow cancellation in 0.18.4
>
> No store schema or flow contract hash changes. Upgrade **every host and worker of the flow type** before sending
> `FlowMsg::Cancel`: 0.18.3 refuses it for good, despite equal hashes, and the run keeps running. After the upgrade, a
> new `Cancel`, an operator retry of the failed one or its redrive from the dead letters cancels the run; unlike a
> reply, a `Cancel` may come from any sender. A failed `Cancel` never means the run was cancelled: read its status.
>
> 0.18.4 runs no cleanup after cancellation. A cancelled run keeps its requests, children and external resources
> running, and a later release with cleanup will not clean up runs cancelled before it. Release resources in your
> application, or wait for that release before cancelling runs that hold them.

## 4. Architecture

**Where.** `docs/architecture.md`, the `cicada-actors` row of the crate table.

- Replace "(ADR 0044; 0.18.3 also executes requests, single waits and joins of up to 100 requests for all replies or
  the first, with optional deadlines, but refuses `Cancel` and cancelling runs)" with "(ADR 0044; 0.18.4 also executes
  requests, single waits and joins of up to 100 requests for all replies or the first, with optional deadlines, and
  `Cancel`, but refuses every message to a cancelling run)".
- In the same row, after "a public `Continue` starts or recovers a run", add: "; a `Cancel` to a running run runs no
  flow code and, in one commit, cancels the armed `Continue`, marks the run cancelled and clears `armed`, keeping its
  progress; it is handled in mailbox order, never preempts a turn and cleans nothing up, and a done or cancelled run
  ignores `Continue`, `Reply` and `Cancel`".

## Timing, stated precisely

Wherever the prose mentions time, keep these exact: a `Cancel` takes effect when its turn commits, not when it is sent
or when it becomes due; a delayed `Cancel` is due at its time, which is the earliest it can run, and a pause, an
outage or the turn before it can make it later. A deadline or a wake time is likewise the earliest moment its decision
can be taken; whichever of the deadline's `Continue`, a reply and a `Cancel` commits first wins.
