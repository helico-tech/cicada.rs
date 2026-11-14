# Docs handoff for flow-finalizer

The docs session owns `README.md` and the hand-written docs under `docs/` until `build-loop-docs-handoff` lands. This
file holds the prose this change needs from it. The text below is complete, so the file stands alone. It builds on the
docs handoff of `flow-cancellation` (`openspec/changes/archive/2026-11-14-flow-cancellation/docs-handoff.md`): apply
that one first, then this one; the places named below are as that handoff leaves them. Where this file replaces text
that handoff asked for, this file wins: 0.18.5 changes what `Cancel` does.

What the code branch holds: `Flow::on_cancel`, `Cleanup` and `MainReply` in `crates/cicada-actors/src/flow.rs`
(exported from `cicada_actors`), the phase dispatch of `FlowActor` (private `settle`, `accept` with the phase,
`cancel` that now marks cancelling and arms the cleanup), doc comments on the module, `Flow`, `Flow::on_cancel`,
`FlowMsg`, `FlowStatus`, `Step`, `Step::run`, `Step::wait`, `Cleanup` and its methods, `MainReply` and `FlowActor`,
with compiled examples on `Flow::on_cancel` and `Cleanup`; unit tests (`cleanup_tests` in `flow.rs`); actor tests in
`crates/cicada-actors/tests/flow.rs` (section "Finalization", and the updated section "Cancellation"); example 18,
`examples/src/flow_resources.rs` with `examples/tests/flow_resources.rs`, its row in `examples/src/lib.rs` and its
path in the flows row of `skills/cicada/SKILL.md`; the regenerated descriptions of `FlowMsg` and `FlowStatus`
(`crates/cicada-actors/tests/contracts/test.sign-up.json`, `sample-service/tests/contract-texts.json`; hashes and
description-stripped schemas unchanged); an "In Rust (0.18.5)" paragraph in ADR 0044 and its index row; the changelog
entry, version 0.18.5, `docs/openapi.json` and the skill's `cicada-version`. No CLI, HTTP route, store schema,
ready-made actor contract or guarantee changes: `docs/reference/` and `docs/guarantees.md` need nothing.

What users must learn, in one place: `Cancel` no longer ends a run in one turn. Its turn marks a running run
`cancelling` and arms a cleanup `Continue`; that `Continue` runs `Flow::on_cancel`, which does nothing by default, and
only a cleanup that returns `Ok` after replaying all its saved entries marks the run `cancelled`, in a turn of its own.
No main code runs once the `Cancel` committed. The cleanup is ordinary flow code over a journal of its own, with the
same helpers (`Cleanup` dereferences to `Step`), its own ids, and step keys `<run address>/cancel/<id>`. It reads what
the main code saved through lookups that never change during the cleanup. A cleanup that fails keeps the run
cancelling; a public `Continue` recovers it, a second `Cancel` does not. Every host and worker of the type must run
0.18.5 and the same cleanup code before a run is cancelled: a 0.18.4 node, or code with the default hook, marks the run
cancelled with no cleanup, although the contract hash is the same.

## 1. Cleaning up after a cancel, in the actors guide

**Where.** `docs/guide/actors.md`, `## Flows`: its rules table, the subsection `### Cancelling a run` that the
flow-cancellation handoff adds, and a new subsection after it, before `## Event`.

**Changes to existing text.**

- Rules table, row "Completion": "Done and cancelled runs ignore Continue, Reply and Cancel and retain their progress
  until deleted; a cancelling run ignores Cancel".
- Rules table, row "Available in 0.18.4" (from the flow-cancellation handoff): rename to "Available in 0.18.5" and write
  "Steps, sleeps, sends, requests, single waits and joins, cancellation and the cleanup after it".
- Rules table, row "Recovery": "Public Continue rearms a running or cancelling run whose armed message was
  cancelled, purged or exhausted, in its active phase; it never restarts a cancelled run".
- `### Cancelling a run`: replace the paragraph after the Rust block, which says that the run is cancelled in one
  commit and that no cleanup runs, with: "When its turn runs, a running run becomes `cancelling` in one commit: the
  armed `Continue` (a sleep's timer, a deadline, a retry in backoff or the next execution) is cancelled, a cleanup
  `Continue` is armed, due now, with the flow's retry policy and the `Cancel`'s correlation id, and the input, the
  journal, the replies and any open wait stay as saved. No flow code runs in that turn. The cleanup's turns follow, and
  the run is `cancelled` once its cleanup succeeds (see the next section); with the default cleanup that is the very
  next turn. The `Cancel` waits its turn in the mailbox; it does not interrupt a running execution, and effects already
  made are not undone." The Rust block stays as it is: it already says "the cancel, then the default cleanup in a turn
  of its own" (the example on `FlowMsg`).
- The paragraph or table of the flow-cancellation handoff that says a cancelled run keeps its requests, children and
  external resources running and that no cleanup runs: keep "a `Cancel` stops no target and no child by itself", and
  replace the rest with "release them in `Flow::on_cancel`".

**New subsection.** Agreed text; the first Rust block is the example on `Flow::on_cancel` in
`crates/cicada-actors/src/flow.rs`, the second the example on `Cleanup`, so both compile as guide doctests with the
same hidden lines:

> ### Cleaning up after a cancel
>
> `Flow::on_cancel` is a run's cleanup. It runs only after a `Cancel` reached the run while it was running, never after
> `run` returned `Ok`, and by default it does nothing. The cleanup `Continue` that the `Cancel` armed calls it with a
> clone of the saved input. It is replayed like `run`, over a journal of its own: `cleanup` dereferences to a `Step`,
> so `run`, `sleep`, `send`, `request` and the waits work as in the main code, one new entry per turn. Its ids are a
> namespace of their own, and its step keys are `<run address>/cancel/<id>`, so a cleanup step never shares a key with
> a main step of the same id. A request's route still names the run itself. Only a cleanup that returns `Ok` after
> replaying every saved cleanup entry, with no new one, marks the run `cancelled`, in a turn of its own.
>
> *(first Rust block: the example on `Flow::on_cancel`, which releases a reservation by `main_key`)*
>
> The main journal stays as it was when the run was cancelled, and the cleanup reads it with lookups:
>
> | Lookup | Returns |
> | --- | --- |
> | `main_key(id)` | `<run address>/<id>`, the key the main step had or would have had, also when nothing is saved |
> | `main_result::<T>(id)` | The saved result of the main step `id`, or `None` when the main journal has no entry `id` |
> | `main_request::<T>(id)` | The main request `id`, or `None` when its send never committed |
> | `main_reply(&request)` | `Open` when no closed main wait names the request; else `Replied(value)`, or `Closed` without a reply |
> | `main_journal()` | The raw saved entries |
>
> `None` from `main_result` does not prove that the step did nothing: a step can act and then fail before its result
> is saved. Release what a main step may have made by its key, with an owner that is idempotent by key and keeps a
> tombstone of a released key before it confirms: a create of that key that is still in flight (accepted, though its
> call failed or timed out) or comes later must create nothing. Fencing later messages is not enough; the library
> cannot know what a step or a target made.
>
> `main_reply` is decided by the main journal alone, so its answer never changes during the cleanup. A reply that comes
> after the `Cancel` is kept but leaves the answer `Open`; to use it, wait for the request in the cleanup. A cleanup
> wait may name the cleanup's own requests and main requests that no closed main wait names: requests the main code
> never waited for, and those of the wait the `Cancel` interrupted, mixed in one join. Each request goes in one cleanup
> wait only. A request that a closed main wait names, a first wait's loser included, takes no new reply; send a new
> request from the cleanup. While a run is cancelling, a reply closes only a cleanup wait; the interrupted main wait
> stays open in the journal.
>
> *(second Rust block: the example on `Cleanup`, which stops a job unless the main code had decided on its reply)*
>
> The lookups consume no id and may repeat. An invalid id, an entry of another kind, a request that is not a main
> request and a value that does not decode fail the turn for good, also when the cleanup catches the error, so a
> swallowed lookup cannot mark the run cancelled; the data stays saved, so fix the code and send a public `Continue`.
> `main_journal` has no such protection: code that decodes its raw values is ordinary replayed code.
>
> A cleanup that fails keeps the run `cancelling` with the cleanup entries it saved. The cleanup `Continue` follows the
> flow's retry policy; when it fails for good, runs out of attempts or is purged, send a public `Continue`, which
> replays the cleanup from its saved entries. A second `Cancel` changes nothing and does not restart it. A cleanup
> parked on a wait sends nothing until the reply comes, or a deadline it set passes. A timed cleanup wait counts its
> deadline from the claim of the cleanup turn that creates it, not from the interrupted main wait's deadline; the
> deadline is the earliest time a timeout can be decided. Deleting a run is immediate and runs no cleanup; the core
> removes its children as for any actor. A cleanup's last send, such as a notice to the owner, commits in a turn before
> the one that marks the run cancelled: the two are not one commit. Read the status, or wait for an application
> message, to learn that the cleanup ended; no `Stopped` or `Failed` notification is sent.
>
> Example 18, `examples/src/flow_resources.rs`, is a job that owns a sandbox: cancelled, its cleanup stops the
> sandbox, waits for the confirmed removal, releases the capacity by the reservation's key and reports.

**Upgrade sentence.** At the end of `### Cleaning up after a cancel`, or after the upgrade sentence the
flow-cancellation handoff adds: "Upgrade every host and worker of the flow type to 0.18.5, with the same `on_cancel`
code, before you rely on a cleanup (0.18.5 rollout, linked as `../how-to/upgrading.md#enable-flow-cleanup-in-0185`, the
section of part 3)."

## 2. Operations

**Where.** `docs/operations.md`, `## On call`.

- Table "Symptom / Check / Action": add a row. Symptom "Flow stuck in cancelling". Check "The cleanup `Continue`
  failed for good, ran out of attempts or was purged (its outcome; the dead letters), or the cleanup is parked at a
  wait". Action "Fix the cause, then send a public `Continue`; it replays the cleanup from its saved entries. A second
  `Cancel` does nothing. A parked cleanup waits for its reply".
- The row "Flow Cancel failed" from the flow-cancellation handoff: in Check, add "or a 0.18.4 node refused a message
  of a cancelling run"; Action unchanged.
- `### Destructive controls`, the sentence about `FlowMsg::Cancel` from the flow-cancellation handoff: add "; deleting
  a run runs no cleanup".

## 3. Upgrading

**Where.** `docs/how-to/upgrading.md`, a new section after `## Enable flow cancellation in 0.18.4`. In that 0.18.4
section, replace "and a later release with cleanup will not clean up runs cancelled before it. Release resources in
your application, or wait for that release before cancelling runs that hold them." with "and 0.18.5, which adds the
cleanup, does not clean up runs cancelled before it." Agreed text of the new section:

> ## Enable flow cleanup in 0.18.5
>
> No store schema or flow contract hash changes. `FlowMsg::Cancel` now marks a run `cancelling` and runs
> `Flow::on_cancel` in later turns; only its success marks the run `cancelled`. Code that reads the status right after
> the `Cancel` turn sees `cancelling`; wait for `cancelled`.
>
> Upgrade **every host and worker of the flow type**, all with the same `on_cancel` code, before you cancel a run that
> needs its cleanup. Equal contract hashes do not show equal cleanup code: a 0.18.4 node, or a binary whose flow keeps
> the default hook, marks the run cancelled without the cleanup, and that run is never cleaned up later. A 0.18.4 node
> also refuses for good every message to a cancelling run, a cleanup's replies included. A `Continue` after the
> upgrade resumes the cleanup, but cannot bring back a refused reply: the target must reply again, or the cleanup needs
> a timeout or another recovery path of the application. To change `on_cancel` while runs are cancelling, drain them
> first, or give the new code a new type name.
>
> Runs that 0.18.4 cancelled stay cancelled and are never finalized.

## 4. Architecture

**Where.** `docs/architecture.md`, the `cicada-actors` row of the crate table.

- Replace the parenthesis that the flow-cancellation handoff left, "(ADR 0044; 0.18.4 also executes requests, ... and
  `Cancel`, but refuses every message to a cancelling run)", with "(ADR 0044; 0.18.5 also executes requests, single
  waits and joins of up to 100 requests for all replies or the first, with optional deadlines, and `Cancel` with the
  cleanup after it)".
- Replace the sentence that the flow-cancellation handoff added after "a public `Continue` starts or recovers a run"
  with: "; a `Cancel` to a running run runs no flow code and, in one commit, cancels the armed `Continue`, marks the run
  cancelling and arms a cleanup `Continue`; it is handled in mailbox order and never preempts a turn; a cancelling run
  executes only its cleanup, `Flow::on_cancel`, chosen by the saved status, over the cleanup journal with keys
  `<run address>/cancel/<id>`, reading the unchanged main journal through lookups that stay the same during the
  cleanup and waiting also for main requests no closed main wait names; a reply closes only a cleanup wait then; only
  a cleanup that returns `Ok` after replaying every cleanup entry marks the run cancelled, in a turn of its own; a
  cancelling run ignores `Cancel`, and a done or cancelled run ignores `Continue`, `Reply` and `Cancel`".

## 5. README

Nothing is required. If the README lists what flows can do, add "and clean up after a cancel" after cancellation.

## Timing and failures, stated precisely

- A `Cancel` takes effect when its turn commits, not when it is sent or becomes due. The run is `cancelled` only when
  a later cleanup turn commits; with the default cleanup that is the next turn, which a pause, an outage or a full
  mailbox can delay.
- A deadline, a wake time or a delayed `Cancel` is the earliest moment its decision can be taken, not the moment it
  happens.
- A rejected commit, of the `Cancel` or of a cleanup turn, saves nothing: the run stays as it was, and the message
  fails without its retry policy retrying it. A rejected cleanup turn may already have made its step's effect, which
  runs again with the same key when a public `Continue` resumes the cleanup.
- A commit whose answer is lost may have applied. The sender cannot tell: reread the status. A `Cancel` resent after a
  lost answer is harmless, as `Cancel` to a cancelling or cancelled run does nothing; a public `Continue` resumes a
  cleanup that did not finish.
- A commit that never reaches the store is claimed again when its lease runs out, and fails at the claim limit; that
  is the core's recovery, not the flow's retry policy.
