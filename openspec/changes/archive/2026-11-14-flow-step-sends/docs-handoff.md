# Docs handoff for flow-step-sends

The docs session owns `README.md` and the hand-written docs under `docs/` until `build-loop-docs-handoff` lands. This
file holds the prose this change needs from it. The text below is complete, so the file stands alone.

What the code branch holds: the shared flow wire of ADR 0044 and `Step::send` in `crates/cicada-actors/src/flow.rs`
(types `Target`, `CreateKind`, `WaitMode`, `WaitOutcome`, `FlowReply`, re-exported from `cicada-actors`), with doc
comments; unit tests in that file (`state_tests`, `send_tests`); actor tests in `crates/cicada-actors/tests/flow.rs`
(section "Sends", plus "Upgrade"), `tests/store_failure.rs`, `tests/wire_formats.rs` and `tests/event.rs`; the
regenerated flow contracts (`crates/cicada-actors/tests/contracts/test.sign-up.json`,
`sample-service/tests/contracts/flows.onboarding.json`, `sample-service/tests/contract-texts.json`); a new state sample
`sample-service/tests/states/flow-onboarding/sent-and-replied.json`; the upgrade scenario in
`examples/tests/flow_onboarding.rs` with the fixture `examples/tests/fixtures/flows.onboarding-0.17.json`; a short
"In Rust" paragraph in ADR 0044; the changelog entry, version 0.18.0, `docs/openapi.json` and the skill's
`cicada-version`. No CLI, HTTP route, store schema, ready-made actor contract (the event's hash is pinned unchanged) or
guarantee changed: `docs/reference/` needs nothing.

What users must learn, in one place: `Step::send(id, to, message)` creates its target first if asked and enqueues one
message, committed with the send entry and the next `Continue`; replay never converts or serializes the arguments;
enqueueing once per committed entry is not the target's success; the message has no retry policy and carries the
executing `Continue`'s correlation id; the error policy (permanent, `node`, rejected commit and recovery); the wire
change breaks every flow contract and some Rust code; the two supported deployment choices; same-job restore is a
host procedure with caveats; upgrade every host before later features.

## 1. Flows in the actors guide

**Where.** `docs/guide/actors.md`, `## Flows`. Add a subsection `### Sending to actors` after `### Durable sleep`, add
two rows to the rules table, and widen two sentences.

**Changes to existing text.**

- In the paragraph "The code runs from the top on each execution", replace "only one new step runs per turn" with
  "only one new step, sleep or send runs per turn".
- Rules table, row "Ids and size": "Unique ids, 1–100 chars of `A-Za-z0-9._~-:`, shared by steps, sleeps and sends; at
  most 1000 entries".
- Rules table, row "Completion": "Done runs ignore Continue and retain results until explicitly deleted; so do
  cancelled runs, which this release cannot produce".

**Agreed text of the new subsection.**

> ### Sending to actors
>
> `step.send(id, to, message)` sends one message to an actor as a step of the flow. The create of the target (if any),
> the message, the send entry and the next Continue commit together; a retry of a later step replays the entry and
> sends nothing again. `to` is `&job` for an actor that exists, `Target::create(&job, &state)` for an independent
> actor created first, or `Target::child(&job, &state)` for a child of the run, which deleting the run removes. An
> actor that exists keeps its state. The compiler checks the message against the ref's family.
>
> ```rust
> step.send("start", Target::child(&job, &JobState { size: 3 }), JobMsg::Start { size: 3 }).await?;
> step.send("stop", &job, JobMsg::Stop).await?;
> ```
>
> The entry saves where the message went, its generated message id and how the target was created, never the message
> or the state. Replay compares only the id and the kind (a send is not a step or a sleep), and never converts the
> target, computes its address or serializes the message or the state: the saved destination stands even if the code
> now names another. The code that builds the arguments still runs on every replay, so it must be deterministic too.
>
> A send promises that the message is enqueued once per committed entry, not that the target succeeds. The message is
> due now, carries the correlation id of the executing Continue and has **no retry policy**: a failure of the target's
> handler ends that message and leaves the flow's progress as it is. Have the target report a business failure as data,
> or give its own design the recovery it needs. The target runs it under the ordinary at-least-once rules.
>
> | Problem | Outcome |
> | --- | --- |
> | The run itself as target; a target, message or state that does not convert or encode | Refused for good, nothing sent |
> | A type that is not registered or does not take the message; a state that is `null`, holds a NUL or is too deep, also for an actor that exists | Refused for good when the send commits (a raw send checks the registry even where a typed `Turn::send` would not) |
> | A failed read of the type registry | An ordinary failure of the cause `node`, retried by the flow's policy |
> | A missing target; a child create of an actor that has another parent or none | The commit is rejected (cause `commit`) and the run stalls; fix it and send a public Continue |
> | A panic while the arguments convert, caught by the flow | Refused for good |
>
> Nothing of a failed turn commits: no create, no message, no entry. A rejected send is attempted afresh after a public
> Continue, with a new message id. This release refuses `FlowMsg::Reply` and `FlowMsg::Cancel` for good; they are on
> the wire for later releases (ADR 0044).

**Example file.** Both lines are in the doc comment of `Step::send`; the guide block may use it as a doctest with
`Job`, `JobMsg` and `JobState` hidden, as that doc comment defines them. No file of `examples/` sends yet; the
upgrade scenario is in `examples/tests/flow_onboarding.rs`.

## 2. The flow wire, in the architecture

**Where.** `docs/architecture.md`, the `cicada-actors` row of the crate table (line 412), the part on durable flows.

**Agreed replacement** of "each run an actor whose state holds the input and an ordered journal of saved results (at
most 1,000 `step` or `sleep` entries); each execution, only by the armed `Continue`, replays the journal and runs one
new step, whose result commits with the next `Continue`, or one new sleep, whose wake time commits with a `Continue`
due then and is never recomputed,":

> each run an actor whose state holds the input, an ordered journal (at most 1,000 `step`, `sleep`, `send` or `wait`
> entries), a status (running, cancelling, done, cancelled), the armed `Continue`, kept replies and a cleanup journal
> (ADR 0044; this release executes steps, sleeps and sends only, and refuses `Reply`, `Cancel` and cancelling runs);
> each execution, only by the armed `Continue`, replays the journal and runs one new step, whose result commits with
> the next `Continue`, one new sleep, whose wake time commits with a `Continue` due then and is never recomputed, or
> one new send, whose create and message commit with its entry and the next `Continue`, enqueued once per entry and
> never converted again on replay,

## 3. Upgrading to 0.18.0

**Where.** `docs/how-to/upgrading.md`, a new paragraph after the entry for 0.17.0. `docs/operations.md` `## Upgrading`
already links there, and the changelog links `docs/operations.md#upgrading`.

**Agreed text.**

> **Version 0.18.0 changes no schema of the store, but it changes the contract of every flow type** (ADR 0044). The
> flow wire gains `Reply` and `Cancel`, two statuses, send and wait entries, kept replies and a cleanup journal, so a
> node of 0.18.0 that hosts a flow type refuses to start against its registration of 0.17 (`ContractConflict`), and a
> worker of 0.17 that leases it is refused (`ContractMismatch`). Other types, the event among them, keep their
> contracts. Choose one of two ways:
>
> 1. **New type names.** Keep the existing flow types on a pinned binary of 0.17 until their runs finish, and host the
>    new code under new type names (`flows.onboarding-v2`) on binaries of 0.18.0, on the same store. Every type a
>    binary hosts must still match its registration, so a binary that co-hosts other types needs their contracts
>    unchanged. Renaming a type inside an upgraded binary is not enough: the binary of 0.17 must keep the old name.
>    `examples/tests/flow_onboarding.rs` shows the new name running next to a registration of 0.17.
> 2. **Drain and reset.** Stop starting runs, let the running ones finish, archive what you need of them, stop every
>    host and worker of the type, delete its runs, then its registration, and start the hosts of 0.18.0. The deleted
>    journals are gone; drain and delete does not preserve them.
>
> A state saved by 0.17 decodes with 0.18.0 (its new fields default to empty), but that does not make the
> registrations compatible, and no upgraded binary keeps the old schema. Recreating a run at the same address from an
> exported state is a host-specific procedure for the same job, not a utility of Cicada: quiesce every producer and
> worker; verify the backup and that each state decodes and passes the core's limits before deleting anything; clear
> an armed id that no longer exists; restore controls and topology (pause flags, watches, children) deliberately; then
> send each running run a public Continue. The restored run is a new instance with a new mailbox: pending messages are
> lost, and an external step that had not committed may run again with its old key.
>
> Rust code changes: an exhaustive `match` on `FlowMsg`, `FlowStatus` or `JournalEntry` needs the new variants, and a
> `FlowState { .. }` literal needs `replies` and `cleanup` or `..FlowState::new(input)`.
>
> The later flow releases (reply waits, timeouts, joins, cancellation, cleanup) are meant to keep this contract, so
> equal hashes will not show which features a node supports: upgrade every host and worker of a flow type before you
> enable new flow code or send a newly supported message, or a node of 0.18.0 refuses it for good. A new versioned
> type name is the alternative.

## 4. Proof links in the table of guarantees

**Where.** `docs/guarantees.md`, rows G2 and G4. No new guarantee.

- **G2 Atomic**, add: `crates/cicada-actors/tests/flow.rs::a_send_commits_its_create_message_entry_and_continuation_together`
  and `crates/cicada-actors/tests/flow.rs::creates_follow_core_rules_and_a_rejected_commit_stalls_until_a_public_continue`.
- **G4 Effects once**, add: `crates/cicada-actors/tests/flow.rs::a_saved_send_replays_without_another_create_or_message`.

`examples/tests/docs.rs` checks that each named test exists.

## 5. README

Nothing beyond the release pins, which change at the release.
