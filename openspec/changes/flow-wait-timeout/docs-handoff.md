# Docs handoff for flow-wait-timeout

The docs session owns `README.md` and the hand-written docs under `docs/` until `build-loop-docs-handoff` lands. This
file holds the prose this change needs from it. The text below is complete, so the file stands alone. It builds on the
docs of `flow-reply-waits` as merged on main (`983af4b`, "explain typed flow requests, parked waits and reply recovery").

What the code branch holds: `Step::wait_timeout` and `Waited<T>` in `crates/cicada-actors/src/flow.rs` (re-exported
from `cicada-actors`), with doc comments and a doctest; the runner's deadline handling in `FlowActor`; unit tests in
that file (`wait_timeout_tests`, and the updated `wait_tests`); actor tests in `crates/cicada-actors/tests/flow.rs`
(section "Waits with a deadline"); worked example 17, `examples/src/flow_approval.rs`, now waiting a week
(`DECISION_TIMEOUT`) with a new `Orders::expire` and the test `an_undecided_purchase_expires_and_ignores_a_later_decision`
in `examples/tests/flow_approval.rs`; the regenerated description of the wait entry
(`crates/cicada-actors/tests/contracts/test.sign-up.json`, `sample-service/tests/contract-texts.json`; hashes
unchanged); an "In Rust (0.18.2)" paragraph in ADR 0044 and its index row; the changelog entry, version 0.18.2,
`docs/openapi.json` and the skill's `cicada-version`. No CLI, HTTP route, store schema, ready-made actor contract or
guarantee changed: `docs/reference/` and `docs/guarantees.md` need nothing. The skill's routes need nothing new: example
17 and `docs/guide/actors.md#requests-and-reply-waits` are routed already.

What users must learn, in one place: `step.wait_timeout(id, &request, timeout)` waits for a reply until a deadline and
returns `Waited::Received(value)` or `Waited::TimedOut`, data to branch on, never an error; the deadline counts from
the claim of the turn that creates the wait (not from the request) and is saved, so replay and changed code keep it;
the outcome is committed in a turn of its own before code after the wait runs, and never changes; a reply handled
before the timeout is decided wins, also after the deadline, as the run's mailbox order decides; a zero timeout still
commits the wait first; a timeout stops nothing of the target's work and cleans nothing up; a lost deadline
`Continue` leaves the wait open until a reply or a public `Continue`; upgrade every host and worker of the flow type
before code waits with a deadline, as 0.18.1 refuses the reply to such a wait despite equal hashes.

## 1. Flows in the actors guide

**Where.** `docs/guide/actors.md`, `## Flows`, its rules table and `### Requests and reply waits`.

**Changes to existing text.**

- Rules table, row "Waiting": "`step.wait` parks with no Continue until the first eligible reply; `step.wait_timeout`
  arms one Continue at its deadline and returns `Waited::TimedOut` when no reply was handled first".
- Rules table, row "Available in 0.18.1": rename to "Available in 0.18.2" and write "Steps, sleeps, sends, requests and
  single reply waits, with or without a deadline; joins and cancellation remain refused".
- Rules table, row "Completion": "0.18.1 cannot produce cancelled runs" becomes "0.18.2 cannot produce cancelled runs".
- Rules table, row "Timeouts": keep it, and add after it: "To give up waiting for a reply, use `step.wait_timeout`, not
  a timeout around `step.wait`".
- In `### Requests and reply waits`, replace the first code block (the one ending in `step.wait("decision", ...)`) and
  the sentence before it with the agreed text below, and keep the rest of the subsection; then add the new paragraphs
  and the table rows given below.
- Recovery table of that subsection, row "Target fails, disappears or never replies": "With `step.wait` the run stays
  parked; use `step.wait_timeout` to give up at a deadline, or report business failure as reply data, such as
  `Result<T, String>`".
- Its upgrade sentence: "Upgrade every host and worker before enabling requests (the existing 0.18.1 rollout link),
  and again before enabling deadlines (0.18.2 rollout, linked from the guide as
  `../how-to/upgrading.md#enable-flow-wait-deadlines-in-0182`, the section of part 5); equal contract hashes do not
  prove support."

**Agreed text replacing the first code block and its lead-in.**

> `step.request(id, to, build)` sends work carrying a typed `ReplyTo<T>` route; `step.wait(id, &request)` returns its
> reply as `T`, and `step.wait_timeout(id, &request, timeout)` returns `Waited::Received(T)` or `Waited::TimedOut`.
> Targets use the same existing/create/child choices as `step.send`. From flow_approval.rs (linked from the guide as
> `../../examples/src/flow_approval.rs`, as now), which waits a week (`DECISION_TIMEOUT`) for a person:
>
> ```rust
> let ask = |reply| ApprovalsMsg::Ask { item: input.item.clone(), cents: input.cents, reply };
> let request = step.request("ask-approval", Target::create(&approvals, &empty), ask).await?;
> let decision: Option<Decision> = match step.wait_timeout("decision", &request, DECISION_TIMEOUT).await? {
>     Waited::Received(decision) => Some(decision),
>     Waited::TimedOut => None, // nobody decided within a week: the flow tells the requester
> };
> ```

The doctest harness of that block becomes (the example exports `DECISION_TIMEOUT`; `Waited` is in `cicada_actors`):

```text
<!-- doctest
# use cicada::HandlerResult;
# use cicada_actors::{Step, Target, Waited};
# use cicada_examples::flow_approval::{Approvals, ApprovalsMsg, ApprovalsState, DECISION_TIMEOUT, Decision, PurchaseInput};
# async fn ask(step: &mut Step, input: PurchaseInput) -> HandlerResult {
# let approvals = Approvals { team: input.team.clone() };
# let empty = ApprovalsState::default();
@@6
# let _: Option<Decision> = decision;
# Ok(()) }
-->
```

**Agreed new paragraphs, after the paragraph that starts "A wait saves an open entry and **parks**".**

> `step.wait_timeout` saves its deadline with the new wait: the store time of the claim of that turn plus the timeout,
> rounded up to microseconds. It counts from the wait, not from the request, and replay uses the saved deadline even
> when the code now passes another timeout. A timeout above `MAX_DELAY`, or a deadline after `MAX_INSTANT`, fails the
> turn for good, also when the reply is already kept. Instead of parking, the open wait arms one Continue due at the
> deadline, a timer for observers; nothing polls before it.
>
> The outcome is decided once, in a turn that runs no code after the wait, and committed before the next execution
> returns it. A reply turn closes the wait as received without reading the clock, cancels the deadline's Continue and
> arms one with the reply's correlation id. A Continue that executes at or after the deadline with no reply kept closes
> it as timed out, keeping its own correlation id; one before the deadline (a poke, a retry, a deliver-now) arms a
> Continue at the same deadline and saves nothing. The deadline is the earliest moment a timeout can be chosen, not a
> cutoff for replies: whichever of the reply and the deadline's Continue the run's mailbox handles first wins, also
> when the reply is handled after the deadline. A pause, an outage or a slow commit can make the timeout later, never
> earlier. Even a zero timeout first commits the wait open; an early reply closes it at once as received.
>
> A committed outcome never changes. A reply after a timeout is ignored and writes nothing, and a retry of a later step
> returns the same `TimedOut` without requesting again. A timeout stops nothing: the target may still be working and
> may still reply, so make its work safe to finish or to cancel with a message of your own.

**New rows for the recovery table of the subsection.**

| Situation | Recovery or limit |
| --- | --- |
| Deadline's Continue cancelled, purged or failed | The wait stays open past its deadline with nothing polling; a reply still closes it as received, or a public Continue times it out |
| Reply arrives after the timeout | Ignored without a write; the target's work is not stopped |
| Code changes the timeout of a running wait | Ignored: the saved deadline decides |
| `step.wait` meets a saved wait with a deadline, or `step.wait_timeout` one without | Permanently fails execution: keep the helper the run saved |

## 2. The flow request diagram

**Where.** `docs/guide/patterns.md`, `## Flow requests and replies`.

**Change.** Replace "and a wait has no timeout in 0.18.1" in the paragraph after the diagram with: "and since 0.18.2
the example's wait gives up after a week: the deadline's Continue closes the wait as timed out in a turn of its own,
unless the reply was handled first." Optionally add to the diagram, after `Note over F: Open wait saved<br/>no Continue
or polling`, the alternative `Note over F: With a deadline: one Continue<br/>due then closes it timed out`; if the note
stays, change the existing one to `Open wait saved<br/>no polling`.

## 3. Architecture

**Where.** `docs/architecture.md`, the `cicada-actors` row of the crate table.

**Change.** Replace "(ADR 0044; 0.18.1 also executes requests and single waits without deadlines, but refuses joins,
deadlines, `Cancel` and cancelling runs)" with "(ADR 0044; 0.18.2 also executes requests and single waits, with or
without a deadline, but refuses joins, `Cancel` and cancelling runs)", and replace "or one new wait, which commits
closed if its reply is already kept and otherwise parks open with no `Continue`" with "or one new wait, which commits
closed if its reply is already kept and otherwise commits open, parked with no `Continue` or, with a deadline, with a
`Continue` due then; an execution at or after the deadline with no reply kept commits the wait timed out before later
code runs". After "when its single wait is open, closing it and arming `Continue` commit with that value", add "(with
or without a deadline; reply turns never read the clock, so the run's mailbox order decides a race with the
deadline)".

## 4. Operations

**Where.** `docs/operations.md`, the troubleshooting table that has the rows "Timer chain stopped" and "Flow run parked
for long".

**Changes.**

- Row "Timer chain stopped", last column: "Scheduler: Start; debounce/batcher: Flush; flow: Continue (a timed wait whose
  deadline passed then times out)".
- New row after "Flow run parked for long": "Timed flow wait open past its deadline" | "Open wait with `until` in the
  past as last journal entry; `armed` names a cancelled, purged or failed Continue, or the run's mailbox is paused" |
  "Resume the mailbox, or send a public Continue: it commits the timeout. A reply from the target handled first still
  wins".

## 5. Upgrading

**Where.** `docs/how-to/upgrading.md`, a new section after `## Enable flow replies in 0.18.1`.

**Agreed text.**

> ## Enable flow wait deadlines in 0.18.2
>
> No store schema or flow contract hash changes. `Step::wait_timeout` adds a deadline to a single reply wait and
> returns `Waited<T>`. Upgrade **every host and worker of the flow type** before deploying flow code that calls it, or
> use a new type name: 0.18.1 permanently refuses the reply to an open wait with a deadline, despite equal hashes, and
> cannot replay such a wait.
>
> Switching a running run's wait between `step.wait` and `step.wait_timeout` breaks its replay: a saved wait keeps the
> helper that created it. Change the helper only for new runs, or drain the old ones first. A reply that 0.18.1
> refused for good must be sent again by its target after upgrading; a dead-letter redrive is ignored, as for any
> reply.

## 6. Core guide

**Where.** `docs/guide/core.md`, the table of worked examples, row "Flow waiting for approval".

**Change.** "Flow waiting for approval, for at most a week".
