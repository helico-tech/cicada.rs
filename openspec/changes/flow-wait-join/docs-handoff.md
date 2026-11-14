# Docs handoff for flow-wait-join

The docs session owns `README.md` and the hand-written docs under `docs/` until `build-loop-docs-handoff` lands. This
file holds the prose this change needs from it. The text below is complete, so the file stands alone. It builds on the
docs handoff of `flow-wait-timeout` (`openspec/changes/archive/2026-11-14-flow-wait-timeout/docs-handoff.md`): apply
that one first, then this one; the places named below are as that handoff leaves them.

What the code branch holds: `Step::wait_all`, `Step::wait_all_timeout`, `Step::wait_first` and
`Step::wait_first_timeout` in `crates/cicada-actors/src/flow.rs`, with doc comments and doctests (no new public type:
they reuse `Request<T>` and `Waited<T>`); the runner's join handling in `FlowActor` (a reply closes the open wait only
once it has enough replies); unit tests in that file (`join_tests`, and the updated `wait_tests` and
`wait_timeout_tests`); actor tests in `crates/cicada-actors/tests/flow.rs` (section "Joins"); worked example 17,
`examples/src/flow_approval.rs`, which adds `SignOffFlow` (`flows.sign-off`, input `SignOffInput`), `QuoteFlow`
(`flows.quote`, input `QuoteInput`), the `suppliers` actor (`Supplier`, `SupplierMsg`, `SupplierState`, `Offer`,
`SupplierActor`) and `Orders::buy`, with four new tests in `examples/tests/flow_approval.rs`; the regenerated
description of the wait entry (`crates/cicada-actors/tests/contracts/test.sign-up.json`,
`sample-service/tests/contract-texts.json`; hashes unchanged, and pinned in `tests/wire_formats.rs`); an "In Rust
(0.18.3)" paragraph in ADR 0044 and its index row; the changelog entry, version 0.18.3, `docs/openapi.json` and the
skill's `cicada-version`. No CLI, HTTP route, store schema, ready-made actor contract or guarantee changed:
`docs/reference/` and `docs/guarantees.md` need nothing. The skill's routes need nothing new: example 17 and
`docs/guide/actors.md#requests-and-reply-waits` are routed already; if the new subsection below gets its own heading,
`skills/cicada/SKILL.md` may add `docs/guide/actors.md#joins-waiting-for-several-replies` to its flows row.

What users must learn, in one place: `step.wait_all(id, &requests)` waits for every reply and returns them in the order
of `requests`; `step.wait_all_timeout` returns one `Waited` slot per request, keeping the replies that came when it
times out; `step.wait_first` returns the position of the first request whose reply the run accepted, and its reply, and
`step.wait_first_timeout` wraps that in `Waited`. A join takes one journal entry for up to 100 requests of one reply
type, in a list the code builds deterministically; replay compares the list in order. Replies are kept as they come,
before the join too; only the reply that completes the join closes it, and only the saved selection is decoded. The
first join's winner is acceptance order, not list order. Closing a join closes all its requests: losers keep running,
their late replies are ignored, and cleanup needs new requests. A join of one request is the single wait. Upgrade every
host and worker of the flow type before code joins more than one request or uses a first wait: 0.18.2 refuses such a
reply for good despite equal hashes.

## 1. Flows in the actors guide

**Where.** `docs/guide/actors.md`, `## Flows`, its rules table and `### Requests and reply waits`.

**Changes to existing text.**

- Rules table, row "Waiting": add at the end "; `step.wait_all` and `step.wait_first` join up to 100 requests in one
  wait, closed by the reply that completes it".
- Rules table, row "Available in 0.18.2": rename to "Available in 0.18.3" and write "Steps, sleeps, sends, requests,
  single reply waits and joins of all or the first reply, with or without a deadline; cancellation remains refused".
- Rules table, row "Completion": "0.18.2 cannot produce cancelled runs" becomes "0.18.3 cannot produce cancelled runs".
- The upgrade sentence at the end of `### Requests and reply waits`: add "and again before enabling joins (0.18.3
  rollout, linked from the guide as `../how-to/upgrading.md#enable-flow-joins-in-0183`, the section of part 5)".

**New subsection, after `### Requests and reply waits` and before `## Event`.** Agreed text:

> ### Joins: waiting for several replies
>
> A flow that starts several independent jobs waits for their replies with one join instead of a wait per job.
> `step.wait_all(id, &requests)` returns every reply as a `Vec<T>`, in the order of `requests`, whichever came first.
> `step.wait_all_timeout(id, &requests, timeout)` returns a `Vec<Waited<T>>`: one slot per request, in the same order,
> `Received` with the reply or `TimedOut` for a request whose reply had not come at the deadline. Every slot is
> `Received` when all replies came in time; one `TimedOut` slot means the join timed out, and the other slots keep the
> replies that came. From flow_approval.rs (linked as `../../examples/src/flow_approval.rs`), whose sign-off asks every
> team of its input:
>
> ```rust
> let mut requests = Vec::new();
> for team in &input.teams {
>     let approvals = Approvals { team: team.clone() };
>     let ask = |reply| ApprovalsMsg::Ask { item: input.item.clone(), cents: input.cents, reply };
>     requests.push(step.request(&format!("ask-{team}"), Target::create(&approvals, &empty), ask).await?);
> }
> // One slot per team, in the order of `input.teams`.
> let slots = step.wait_all_timeout("sign-offs", &requests, DECISION_TIMEOUT).await?;
> ```
>
> `step.wait_first(id, &requests)` returns `(index, value)`: the position in `requests` of the request whose reply the
> run accepted first, and that reply; `step.wait_first_timeout` returns it as `Waited::Received((index, value))`, or
> `Waited::TimedOut` with no winner. The quote of the same example buys from the first supplier to offer:
>
> ```rust
> let (winner, offer) = step.wait_first("first-quote", &quotes).await?;
> let supplier = input.suppliers[winner].as_str();
> ```
>
> A join borrows a slice of handles of one reply type `T`, a `Vec` or an array; clone named handles into it, and use
> an enum when the replies have several shapes. Build the list deterministically from the input and earlier results:
> replay compares the saved list with it in order, and the helper neither sorts nor deduplicates it. A join names 1 to
> 100 distinct earlier requests of the run that no other wait names, and takes one journal entry: 100 requests and
> their join use 101 of the 1000 entries.
>
> Replies are kept as they come, also before the join exists. The reply that completes the join (the last missing one,
> or for a first join any listed one) closes it in its own turn, without flow code, cancels the deadline's Continue and
> arms a Continue with its correlation id. A partial reply only keeps its value: the run stays parked, or keeps its
> deadline's Continue unchanged. The values reach the code after the join in a later execution, once the outcome is
> committed, and only the selected ones are decoded.
>
> The winner of a first join is the request whose reply the run **accepted** first, as the run keeps its replies in
> that order, not the first one in the list: if `b` replied before `a`, `b` wins, also when both replies came before
> the join. Supplied progress is trusted with the order of its `replies`; keep that order when restoring a run. Once
> committed, the winner and every slot stay the same on every replay.
>
> Closing a join closes **all** its requests. A losing target of a first join, or a target that missed the deadline,
> is not stopped and may still be working; its late reply is ignored without a write. No later wait can name a closed
> request: send new work, with a request of its own, to clean up after a loser.

The doctest harnesses of the two blocks (`ActorsGuideDoctests` compiles them; the example exports every name):

```text
<!-- doctest
# use cicada::HandlerResult;
# use cicada_actors::{Step, Target, Waited};
# use cicada_examples::flow_approval::{Approvals, ApprovalsMsg, ApprovalsState, DECISION_TIMEOUT, Decision, SignOffInput};
# async fn sign_off(step: &mut Step, input: SignOffInput) -> HandlerResult {
# let empty = ApprovalsState::default();
@@8
# let _: Vec<Waited<Decision>> = slots;
# Ok(()) }
-->
```

```text
<!-- doctest
# use cicada::HandlerResult;
# use cicada_actors::{Request, Step};
# use cicada_examples::flow_approval::{Offer, QuoteInput};
# async fn first(step: &mut Step, input: QuoteInput, quotes: Vec<Request<Offer>>) -> HandlerResult {
@@2
# let _: (&str, Offer) = (supplier, offer);
# Ok(()) }
-->
```

**Rows for a table after the subsection's text** (the same columns as the recovery table of `### Requests and reply
waits`):

| Situation | Recovery or limit |
| --- | --- |
| Empty list, more than 100 handles, a handle twice, or a request another wait names | Permanently fails execution, before kept replies are looked at |
| Code reorders, extends or changes the list, or switches between all and first, for a running run | Permanently fails execution; the saved join stays. Change joins only for new runs |
| A target of a join never replies | `wait_all`/`wait_first` stay parked; the timed helpers give up at the deadline with the replies that came |
| A selected reply does not decode as `T` | Permanently fails execution, naming the position; fix code and poke with Continue. The saved outcome stays |
| A loser or a late target replies after the join closed | Ignored without a write; its work is not stopped |

## 2. The flow request diagram

**Where.** `docs/guide/patterns.md`, `## Flow requests and replies`, the paragraph after the diagram.

**Change.** Add at the end: "The same example's sign-off asks several teams and waits for all of them with one join,
and its quote takes the first supplier to offer: each reply is kept as it comes, and only the reply that completes the
join closes it and arms the next Continue."

## 3. Architecture

**Where.** `docs/architecture.md`, the `cicada-actors` row of the crate table.

**Change.** Replace "(ADR 0044; 0.18.2 also executes requests and single waits, with or without a deadline, but refuses
joins, `Cancel` and cancelling runs)" with "(ADR 0044; 0.18.3 also executes requests, single waits and joins of up to
100 requests for all replies or the first, with or without a deadline, but refuses `Cancel` and cancelling runs)".
Replace "or one new wait, which commits closed if its reply is already kept" with "or one new wait, which commits
closed if enough of its replies are already kept (every one, or any one of a first wait)". Replace "when its single wait
is open, closing it and arming `Continue` commit with that value" with "when the open wait names its request and now
has enough replies, closing it with its selection (every request in request order, or the listed request accepted
earliest) and arming `Continue` commit with that value; a partial reply of a join keeps only its value".

## 4. Operations

**Where.** `docs/operations.md`, the troubleshooting table with the rows "Flow run parked for long" and "Timed flow wait
open past its deadline".

**Change.** New row after "Flow run parked for long": "Flow join waits for some of its replies" | "Open wait naming
several requests as last journal entry; `replies` holds some of them" | "Find the targets whose requests have no kept
reply and make them reply; a timed join times out at its deadline with the replies that came. A reply to a request of
a closed join is ignored by design". In the row "Flow run parked for long", if it says "the reply", read "the reply, or
for a join the missing replies".

## 5. Upgrading

**Where.** `docs/how-to/upgrading.md`, a new section after `## Enable flow wait deadlines in 0.18.2`.

**Agreed text.**

> ## Enable flow joins in 0.18.3
>
> No store schema or flow contract hash changes. `Step::wait_all`, `Step::wait_all_timeout`, `Step::wait_first` and
> `Step::wait_first_timeout` wait for several replies with one wait entry. Upgrade **every host and worker of the flow
> type** before deploying flow code that joins more than one request or uses a first wait, or use a new type name:
> 0.18.2 permanently refuses a reply that would close such a wait, despite equal hashes, and cannot replay it.
>
> A join of one request is the same entry as a single wait: `step.wait` and `step.wait_all` of one request replay each
> other's, as do `step.wait_timeout` and `step.wait_all_timeout`, and 0.18.1 and 0.18.2 already run them. Changing the
> list of a running run's join, its order, or all to first breaks its replay: change joins only for new runs, or drain
> the old ones first. A reply that 0.18.2 refused for good must be sent again by its target after upgrading; a
> dead-letter redrive is ignored, as for any reply.

## 6. Core guide

**Where.** `docs/guide/core.md`, the table of worked examples, row "Flow waiting for approval, for at most a week".

**Change.** "Flow waiting for approval, for at most a week; joins of approvals and quotes".
