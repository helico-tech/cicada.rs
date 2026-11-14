# Docs handoff for flow-reply-waits

The docs session owns `README.md` and the hand-written docs under `docs/` until `build-loop-docs-handoff` lands. This
file holds the prose this change needs from it. The text below is complete, so the file stands alone. If the
`flow-step-sends` handoff (archived with 0.18.0) is not merged yet, merge it first: section 1 below goes after its
`### Sending to actors`.

What the code branch holds: `Step::request`, `Step::wait`, `Request<T>`, `ReplyTo<T>` and `ReplyTo::send` in
`crates/cicada-actors/src/flow.rs` (re-exported from `cicada-actors`), with doc comments and doctests; the `Reply`
handling of `FlowActor`; unit tests in that file (`send_tests` for requests and the helper's checks, `wait_tests`);
actor tests in `crates/cicada-actors/tests/flow.rs` (section "Requests, replies and waits"),
`tests/store_failure.rs` (a request whose registry read fails) and `tests/wire_formats.rs` (the route pinned in the new
fixture `tests/contracts/test.job.json`); worked example 17, `examples/src/flow_approval.rs` with
`examples/tests/flow_approval.rs`, routed in `skills/cicada/SKILL.md`; the regenerated flow contract descriptions
(`crates/cicada-actors/tests/contracts/test.sign-up.json`, `sample-service/tests/contract-texts.json`; hashes
unchanged); an "In Rust (0.18.1)" paragraph in ADR 0044 and its index row; the changelog entry, version 0.18.1,
`docs/openapi.json` and the skill's `cicada-version`. No CLI, HTTP route, store schema, ready-made actor contract or
guarantee changed: `docs/reference/` and `docs/guarantees.md` need nothing.

What users must learn, in one place: a flow asks an actor for work with `step.request`, whose builder puts a typed
`ReplyTo<T>` route into the target's message, and waits with `step.wait`; the run parks with no handler and no polling;
the target replies from its own turn with `reply.send(turn, &value)`; only the first reply from the request's target
counts (sender addresses, not identities); the value reaches flow code only after its outcome commits; an early reply is
kept; a wait has no timeout; a reply that does not decode fails for good and is recovered by fixed code and a poke; a
reply to a deleted run rejects the target's commit; upgrade every host and worker before a flow requests.

## 1. Flows in the actors guide

**Where.** `docs/guide/actors.md`, `## Flows`. Add a subsection `### Requests and reply waits` after
`### Sending to actors` (or after `### Durable sleep` if that is not there yet), and widen the rules table and one
sentence.

**Changes to existing text.**

- In the paragraph "The code runs from the top on each execution", replace "only one new step runs per turn" (or "only
  one new step, sleep or send runs per turn") with "only one new step, sleep, send, request or wait runs per turn".
- Rules table, row "Ids and size": "Unique ids, 1–100 chars of `A-Za-z0-9._~-:`, shared by steps, sleeps, sends,
  requests and waits; at most 1000 entries".
- Rules table, new row "Waiting": "A wait parks the run with no Continue; the first reply from the request's target
  closes it. No timeout yet".

**Agreed text of the new subsection.**

> ### Requests and reply waits
>
> `step.request(id, to, build)` sends one message that a reply answers, and `step.wait(id, &request)` waits for that
> reply. `build` receives a `ReplyTo<T>` route and returns the target's message; a field `reply: ReplyTo<Decision>` in
> that message types the request, so the wait returns a `Decision`. `to` is a `&ref`, a `Target::create` or a
> `Target::child`, as for `step.send`. From flow_approval.rs (link it, from the guide, as
> `../../examples/src/flow_approval.rs`):
>
> ```rust
> let ask = |reply| ApprovalsMsg::Ask { item: input.item.clone(), cents: input.cents, reply };
> let request = step.request("ask-approval", Target::create(&approvals, &empty), ask).await?;
> let decision = step.wait("decision", &request).await?;
> ```
>
> The target keeps the route as long as it needs and replies from one of its own turns:
>
> ```rust
> decided.reply.send(turn, &Decision { approved, by }).await?;
> ```
>
> The route is `{run, request}`: the run's address and the request's message id, the same wire form for every `T`.
> The request commits like a send: its create, its message, its entry and the next Continue together; replay returns the
> saved request without calling `build`. The reply is an ordinary send of the target's turn: it commits with that turn
> or not at all, is due now, carries the turn's correlation id and has no retry policy. `reply.send` checks the value
> when called: one that does not encode, holds a NUL or is more than 61 levels deep returns `TurnError::Invalid`.
>
> A wait whose reply has not come saves an open wait and **parks** the run: no Continue is armed, nothing polls and no
> handler stays open, for minutes or for days. The reply's turn keeps the value, closes the wait and arms a Continue
> with the reply's correlation id, all in one commit and without running flow code; that Continue replays the wait,
> which returns the value. A reply that comes before its wait is kept; the wait then saves itself closed and the next
> execution returns the value. Either way, flow code sees the value only once its outcome is committed.
>
> The run accepts only the **first** reply that comes from the address its request went to, and ignores every other
> `Reply` without writing anything: an unknown request, a second reply, a reply from another actor. A direct HTTP send
> has no sender; a delegate or a child of the target has its own address; a dead-letter redrive is sent by the dead
> letters actor. So a delegate must hand its result back to the target, which replies. A trusted remote worker that
> completes the target's leased turn sends as the target. This checks actor addresses, not people: anyone who can make
> the target reply decides the reply.
>
> A wait has **no timeout** in this release: a target that fails, is deleted or never replies leaves the run parked.
> Report business failure as data in the reply (`Result<T, String>`). A reply that does not decode as the wait's `T`
> fails the execution for good and keeps the saved outcome: fix the code and send a public Continue. A target that
> replies to a run that was deleted has its whole commit rejected, as for any send to a missing actor.
>
> A public Continue to a parked run replays it and parks it again, without a new entry. A paused run handles its
> replies after resume. A lost or purged Continue after closure is recovered by a public Continue, which replays the
> saved reply without asking again. Joins of several requests, waits with a deadline and cancellation come in later
> releases; this one refuses them for good.

**Index sentence of the guide (if it lists flow helpers):** add `Step::request`, `Step::wait` and `ReplyTo::send`.

## 2. Architecture

**Where.** `docs/architecture.md`, the `cicada-actors` row of the crate table (line beginning
`` | `cicada-actors` | Layer 3 ``), in the part on durable flows.

**Change.** After "or one new sleep, whose wake time commits with a `Continue` due then and is never recomputed,"
insert: "or one new send or request, whose create and message commit with its entry, or one new wait, which commits
closed when its reply is kept and otherwise open, parking the run with no `Continue` until the reply's turn, which runs
no flow code, keeps the first reply from the request's target, closes the wait and arms the `Continue`,". Change "at
most 1,000 `step` or `sleep` entries" to "at most 1,000 entries".

If the table of one-way messaging or known limitations names flow replies as future work, say they are available since
0.18.1 for single waits without a timeout (joins, deadlines and cancellation remain later releases).

## 3. Operations

**Where.** `docs/operations.md`, `## On call`, the symptom table.

**New row** after "Timer chain stopped":

> | Flow run parked for long | Last journal entry is an open wait; `armed` is null; the target's mailbox and state | Make the target reply (it holds the route), or delete the run; a public Continue only parks it again |

**Where.** Same file, `### Changing a contract`, or wherever a contract change of a host's own types is explained.

**Add one sentence:** "Adding a `ReplyTo<T>` field to a message of your own actor type changes its contract like any
new field: follow this procedure."

## 4. Upgrading

**Where.** `docs/how-to/upgrading.md`, with the other per-version notes (after the note on 0.18.0, if present).

**Agreed text.**

> **Version 0.18.1 changes no schema and no contract hash.** It lets flows request work and wait for replies
> (`Step::request`, `Step::wait`, `ReplyTo::send`). Equal hashes do not mean equal behaviour: a host or worker of
> 0.18.0 refuses `FlowMsg::Reply` for good and stops for good at a saved request. Upgrade every host and worker of a
> flow type before you deploy flow code that requests, or give that code a new type name. A reply refused by a node of
> 0.18.0 is lost: redriving it from the dead letters does not help, because the redrive is sent by the dead letters
> actor and the run ignores a reply that does not come from its request's target. A run that stalled at a saved
> request on 0.18.0 recovers with a public Continue after the upgrade. Adding a `ReplyTo<T>` to a message of a target
> type changes that target's contract: follow Changing a contract (link `../operations.md#changing-a-contract` from
> the upgrading page).

## 5. README

No change needed beyond what the docs session judges for the feature list; the release pins move at the release.
